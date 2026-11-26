//! Payload checks against the catalog before anything is sent: field names made
//! canonical, read-only fields and empty references refused, nested collection
//! items checked against their own resource, and the read-back compared.

use std::sync::{Arc, LazyLock};

use regex::Regex;
use serde_json::{Map, Value, json};

use crate::catalog::loader::decode_char_refs;
use crate::catalog::{
    Catalog, CatalogCollection, CatalogResource, Member, Provider, Request, did_you_mean,
    is_js_space, js_trim, same_name, suggestions,
};
use crate::format::dates::same_date_value;
use crate::format::shape;
use crate::http::BoxFuture;

/// A checked payload, or every problem found, as one message.
pub type Prepared<T> = Result<T, String>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Create,
    Update,
}

/// Collections Targetprocess takes as a plain array rather than `{"Items": [...]}`.
const ARRAY_COLLECTIONS: [&str; 2] = ["tagobjects", "customfields"];

/// `Number.MAX_SAFE_INTEGER`.
const MAX_SAFE: f64 = 9_007_199_254_740_991.0;

fn settable_names(resource: &CatalogResource) -> Vec<&str> {
    let values = resource
        .values
        .iter()
        .filter(|f| f.can_set && f.name != "Id")
        .map(|f| f.name.as_str());
    let references = resource
        .references
        .iter()
        .filter(|f| f.can_set)
        .map(|f| f.name.as_str());
    let collections = resource
        .collections
        .iter()
        .filter(|c| c.can_add)
        .map(|c| c.name.as_str());
    values.chain(references).chain(collections).collect()
}

/// The items of `[...]` or `{"Items": [...]}`; None for anything else.
fn item_list(value: &Value) -> Option<&Vec<Value>> {
    match value {
        Value::Array(items) => Some(items),
        Value::Object(m) => m.get("Items").and_then(Value::as_array),
        _ => None,
    }
}

/// A reference is `{"Id": n}` (numeric string accepted) or `null` to clear it. Never an empty reference.
pub fn prepare_reference(value: &Value, label: &str) -> Prepared<Value> {
    if value.is_null() {
        return Ok(Value::Null);
    }
    let id_value = match value {
        Value::Object(m) => m.get("Id").filter(|v| !v.is_null()).or_else(|| m.get("id")),
        other => Some(other),
    };
    match id_value {
        Some(Value::Number(n)) => {
            let n = n.as_f64().unwrap_or(f64::NAN);
            if n.fract() == 0.0 && n.abs() <= MAX_SAFE && n > 0.0 {
                return Ok(json!({ "Id": n as i64 }));
            }
        }
        Some(Value::String(s)) if !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) => {
            let n: f64 = s.parse().unwrap_or(f64::INFINITY);
            if n > 0.0 {
                return Ok(json!({ "Id": shape::number(n) }));
            }
        }
        _ => {}
    }
    Err(format!(
        "{label} must be a reference {{\"Id\": <number>}} (or null to clear); got {}. Empty references are never sent.",
        js_json(value)
    ))
}

/// Loads the resources of the collection items a payload nests, at every depth
/// (e.g. `{"Tasks":{"Items":[...]}}` on a UserStory needs Task), so `prepare_body`
/// can validate them. Items that only link an existing entity (`{"Id": n}`) need nothing.
pub fn catalog_for_body<'a>(
    provider: &'a Provider,
    catalog: Arc<Catalog>,
    resource: &'a CatalogResource,
    body: &'a Map<String, Value>,
) -> BoxFuture<'a, Arc<Catalog>> {
    Box::pin(async move {
        let mut current = catalog;
        for (key, value) in body {
            let Some(collection) = resource
                .collections
                .iter()
                .find(|c| same_name(&c.name, key) && c.name != "CustomFields")
            else {
                continue;
            };
            let nested: Vec<&Map<String, Value>> = item_list(value)
                .into_iter()
                .flatten()
                .filter_map(Value::as_object)
                .filter(|item| item.keys().any(|k| !k.eq_ignore_ascii_case("id")))
                .collect();
            if nested.is_empty() {
                continue;
            }
            current = provider
                .get(&Request::Names(vec![collection.type_name.clone()]))
                .await;
            let holder = current.clone();
            let Some(child) = holder.find(&collection.type_name).filter(|c| c.available) else {
                continue;
            };
            for item in nested {
                current = catalog_for_body(provider, current, child, item).await;
            }
        }
        current
    })
}

/// Validates a create/update payload against the catalog and returns it with
/// canonical field names. Nested collection items are validated against the
/// collection's own resource when the catalog holds it (see `catalog_for_body`).
/// `path` labels a nested payload (`Tasks[0]`); empty at the top.
pub fn prepare_body(
    catalog: &Catalog,
    resource: &CatalogResource,
    body: &Map<String, Value>,
    mode: Mode,
    path: &str,
) -> Prepared<Map<String, Value>> {
    let mut problems: Vec<String> = Vec::new();
    let mut out = Map::new();
    let label = |key: &str| {
        if path.is_empty() {
            key.to_string()
        } else {
            format!("{path}.{key}")
        }
    };

    for (key, value) in body {
        if key.to_lowercase() == "id" {
            problems.push(match mode {
                Mode::Create => format!(
                    "{}: remove Id when creating (posting an Id updates an existing entity instead)",
                    label(key)
                ),
                Mode::Update => format!(
                    "{}: pass the id as the \"id\" argument, not inside fields",
                    label(key)
                ),
            });
            continue;
        }
        let Some(member) = catalog.member(resource, key) else {
            let close = suggestions(key, &settable_names(resource), 5);
            let custom_hint = if resource
                .collections
                .iter()
                .any(|c| c.name == "CustomFields")
            {
                " Custom fields go in \"CustomFields\": [{\"Name\": \"...\", \"Value\": ...}]."
            } else {
                ""
            };
            problems.push(format!(
                "{}: {} has no field \"{key}\".{}{custom_hint}",
                label(key),
                resource.name,
                did_you_mean(&close)
            ));
            continue;
        };

        let name = member.name();
        match member {
            Member::Value(_) | Member::Reference(_) if !member.can_set() => {
                problems.push(format!(
                    "{}: {}.{name} is read-only",
                    label(name),
                    resource.name
                ));
            }
            Member::Value(_) => {
                out.insert(name.to_string(), value.clone());
            }
            Member::Reference(_) => match prepare_reference(value, &label(name)) {
                Ok(reference) => {
                    out.insert(name.to_string(), reference);
                }
                Err(message) => problems.push(message),
            },
            Member::Collection(collection) => {
                match prepare_items(catalog, resource, collection, value, &label(name)) {
                    Ok(items) => {
                        out.insert(name.to_string(), items);
                    }
                    Err(message) => problems.push(message),
                }
            }
        }
    }

    if !problems.is_empty() {
        let count = if problems.len() == 1 {
            "Invalid field".to_string()
        } else {
            format!("{} invalid fields", problems.len())
        };
        let settable = if path.is_empty() {
            format!(
                "\nSettable on {}: {}",
                resource.name,
                settable_names(resource).join(", ")
            )
        } else {
            String::new()
        };
        return Err(format!(
            "{count} for {}:\n- {}{settable}",
            resource.name,
            problems.join("\n- ")
        ));
    }
    Ok(out)
}

/// Validates the items posted into a collection and returns the wire shape:
/// `{"Items": [...]}`, or a plain array for TagObjects and CustomFields.
pub fn prepare_items(
    catalog: &Catalog,
    resource: &CatalogResource,
    collection: &CatalogCollection,
    value: &Value,
    label: &str,
) -> Prepared<Value> {
    if !collection.can_add {
        return Err(format!(
            "{label}: items cannot be added to {}.{}",
            resource.name, collection.name
        ));
    }
    let Some(raw) = item_list(value) else {
        return Err(format!(
            "{label}: expected an array of items or {{\"Items\": [...]}}"
        ));
    };
    if raw.is_empty() {
        return Err(format!("{label}: no items given"));
    }

    let custom_fields = collection.name == "CustomFields";
    let child = if custom_fields {
        None
    } else {
        catalog.find(&collection.type_name)
    };
    let mut problems: Vec<String> = Vec::new();
    let mut items: Vec<Value> = Vec::new();

    for (i, item) in raw.iter().enumerate() {
        let item_label = format!("{label}[{i}]");
        let Value::Object(fields) = item else {
            problems.push(format!("{item_label}: expected an object"));
            continue;
        };
        if custom_fields {
            match fields.get("Name") {
                Some(Value::String(name)) if !name.is_empty() => items.push(json!({
                    "Name": name,
                    "Value": fields.get("Value").cloned().unwrap_or(Value::Null),
                })),
                _ => problems.push(format!("{item_label}: custom field needs a \"Name\"")),
            }
            continue;
        }
        if fields.len() == 1 && fields.keys().all(|k| k.to_lowercase() == "id") {
            match prepare_reference(item, &item_label) {
                Ok(reference) => items.push(reference),
                Err(message) => problems.push(message),
            }
            continue;
        }
        let Some(child) = child.filter(|c| c.available) else {
            items.push(item.clone());
            continue;
        };
        match prepare_body(catalog, child, fields, Mode::Create, &item_label) {
            Ok(nested) => items.push(Value::Object(nested)),
            Err(message) => problems.push(message),
        }
    }

    if !problems.is_empty() {
        return Err(problems.join("\n- "));
    }
    if ARRAY_COLLECTIONS.contains(&collection.name.to_lowercase().as_str()) {
        Ok(Value::Array(items))
    } else {
        Ok(json!({ "Items": items }))
    }
}

/// Requested scalar values or reference ids that the read-back does not show.
pub fn unpersisted(requested: &Map<String, Value>, returned: &Value) -> Vec<String> {
    let Value::Object(returned) = returned else {
        return Vec::new();
    };
    let mut missing = Vec::new();
    for (key, want) in requested {
        let Some(got) = returned.get(key) else {
            continue;
        };
        match want {
            Value::Null => {
                if !got.is_null() {
                    missing.push(format!("{key}: requested null, got {}", js_json(got)));
                }
            }
            Value::Object(w) if w.len() == 1 && w.contains_key("Id") => {
                let want_id = &w["Id"];
                let got_id = got.as_object().and_then(|g| g.get("Id"));
                if !got_id.is_some_and(|g| strict_equal(g, want_id)) {
                    let shown = got_id.filter(|g| !g.is_null()).unwrap_or(got);
                    missing.push(format!(
                        "{key}: requested Id {}, got {}",
                        js_string(want_id),
                        js_json(shown)
                    ));
                }
            }
            Value::Number(_) | Value::Bool(_) => {
                if !strict_equal(got, want) {
                    missing.push(format!(
                        "{key}: requested {}, got {}",
                        js_json(want),
                        js_json(got)
                    ));
                }
            }
            Value::String(w) => {
                if let Value::String(g) = got
                    && g != w
                    && !same_date(w, g)
                    && !same_markup(w, g)
                {
                    missing.push(format!(
                        "{key}: requested {}, got {}",
                        js_json(want),
                        js_json(got)
                    ));
                }
            }
            _ => {}
        }
    }
    missing
}

fn same_date(want: &str, got: &str) -> bool {
    static DATE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^[0-9]{4}-[0-9]{2}-[0-9]{2}").unwrap());
    DATE.is_match(want) && same_date_value(want, got)
}

/// Targetprocess may wrap or entity-encode text it stores (e.g. descriptions in `<div>`).
fn same_markup(want: &str, got: &str) -> bool {
    plain(want) == plain(got)
}

fn plain(text: &str) -> String {
    static TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<[^>]*>").unwrap());
    let decoded = decode_char_refs(&TAG.replace_all(text, ""))
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"");
    let mut collapsed = String::with_capacity(decoded.len());
    let mut in_space = false;
    for c in decoded.chars() {
        if is_js_space(c) {
            if !in_space {
                collapsed.push(' ');
            }
            in_space = true;
        } else {
            collapsed.push(c);
            in_space = false;
        }
    }
    js_trim(&collapsed).to_string()
}

/// `===` between JSON values: numbers by value; objects and arrays are never identical.
fn strict_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
        (Value::String(x), Value::String(y)) => x == y,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::Null, Value::Null) => true,
        _ => false,
    }
}

/// A number as JavaScript prints it: `5` for 5.0, exponents from 1e21 and below 1e-6.
fn js_number(n: f64) -> String {
    if n == 0.0 {
        return "0".to_string();
    }
    if (1e-6..1e21).contains(&n.abs()) {
        return n.to_string();
    }
    let text = format!("{n:e}");
    match text.split_once('e') {
        Some((mantissa, exp)) if !exp.starts_with('-') => format!("{mantissa}e+{exp}"),
        _ => text,
    }
}

/// `JSON.stringify` of a JSON value: compact, numbers as JavaScript prints them.
fn js_json(value: &Value) -> String {
    match value {
        Value::Number(n) => js_number(n.as_f64().unwrap_or(0.0)),
        Value::Array(items) => {
            let inner: Vec<String> = items.iter().map(js_json).collect();
            format!("[{}]", inner.join(","))
        }
        Value::Object(map) => {
            let inner: Vec<String> = map
                .iter()
                .map(|(k, v)| {
                    format!(
                        "{}:{}",
                        serde_json::to_string(k).unwrap_or_default(),
                        js_json(v)
                    )
                })
                .collect();
            format!("{{{}}}", inner.join(","))
        }
        other => other.to_string(),
    }
}

/// `String(value)`.
fn js_string(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Number(n) => js_number(n.as_f64().unwrap_or(0.0)),
        Value::Array(items) => items
            .iter()
            .map(|v| {
                if v.is_null() {
                    String::new()
                } else {
                    js_string(v)
                }
            })
            .collect::<Vec<_>>()
            .join(","),
        Value::Object(_) => "[object Object]".to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::snapshot;

    fn map(value: Value) -> Map<String, Value> {
        value.as_object().unwrap().clone()
    }

    fn create(resource: &str, fields: Value) -> Prepared<Map<String, Value>> {
        let catalog = snapshot();
        let r = catalog.find(resource).unwrap().clone();
        prepare_body(&catalog, &r, &map(fields), Mode::Create, "")
    }

    #[test]
    fn sends_exactly_the_requested_fields_with_canonical_names() {
        let body = create(
            "UserStory",
            json!({ "name": "New", "project": { "Id": "26080" } }),
        )
        .unwrap();
        assert_eq!(
            Value::Object(body),
            json!({ "Name": "New", "Project": { "Id": 26080 } })
        );
    }

    #[test]
    fn never_sends_an_empty_reference() {
        let e = create("UserStory", json!({ "Name": "x", "Project": { "Id": "" } })).unwrap_err();
        assert!(e.contains(
            "Project must be a reference {\"Id\": <number>} (or null to clear); got {\"Id\":\"\"}. Empty references are never sent."
        ), "{e}");
    }

    #[test]
    fn rejects_id_on_create_unknown_and_read_only_fields_listing_valid_options() {
        let e = create(
            "UserStory",
            json!({ "Id": 1, "Nmae": "x", "CreateDate": "2026-01-01", "MyCustom": "v" }),
        )
        .unwrap_err();
        assert!(
            e.starts_with("4 invalid fields for UserStory:\n- Id: remove Id when creating"),
            "{e}"
        );
        assert!(e.contains("Did you mean: Name"), "{e}");
        assert!(
            e.contains("CreateDate: UserStory.CreateDate is read-only"),
            "{e}"
        );
        assert!(e.contains("Custom fields go in \"CustomFields\""), "{e}");
        assert!(
            Regex::new(r"\nSettable on UserStory: .*Name")
                .unwrap()
                .is_match(&e),
            "{e}"
        );
    }

    #[test]
    fn update_refuses_id_inside_fields() {
        let catalog = snapshot();
        let r = catalog.find("UserStory").unwrap();
        let e = prepare_body(&catalog, r, &map(json!({ "id": 5 })), Mode::Update, "").unwrap_err();
        assert!(e.starts_with("Invalid field for UserStory:\n- id: pass the id as the \"id\" argument, not inside fields\nSettable on UserStory: "), "{e}");
    }

    #[test]
    fn validates_nested_children_against_their_own_resource() {
        let e = create(
            "UserStory",
            json!({ "Name": "x", "Project": { "Id": 2 }, "Tasks": { "Items": [{ "Name": "t", "Bogus": 1 }] } }),
        )
        .unwrap_err();
        assert!(
            e.contains("Tasks[0].Bogus: Task has no field \"Bogus\""),
            "{e}"
        );
    }

    #[test]
    fn wraps_nested_collections_in_items_and_custom_fields_as_an_array() {
        let body = create(
            "UserStory",
            json!({
                "Name": "x",
                "Project": { "Id": 2 },
                "Tasks": [{ "Name": "t" }],
                "CustomFields": [{ "Name": "FrontEnd", "Value": "To Do" }],
            }),
        )
        .unwrap();
        assert_eq!(
            Value::Object(body),
            json!({
                "Name": "x",
                "Project": { "Id": 2 },
                "Tasks": { "Items": [{ "Name": "t" }] },
                "CustomFields": [{ "Name": "FrontEnd", "Value": "To Do" }],
            })
        );
    }

    #[test]
    fn bulk_items_are_labelled_without_the_settable_list() {
        let catalog = snapshot();
        let task = catalog.find("Task").unwrap();
        let e = prepare_body(
            &catalog,
            task,
            &map(json!({ "Nope": 1 })),
            Mode::Update,
            "items[1]",
        )
        .unwrap_err();
        assert!(
            e.starts_with("Invalid field for Task:\n- items[1].Nope: Task has no field \"Nope\"."),
            "{e}"
        );
        assert!(!e.contains("Settable on"), "{e}");
    }

    #[test]
    fn collection_items_wire_shapes() {
        let catalog = snapshot();
        let build = catalog.find("Build").unwrap();
        let bugs = catalog.collection(build, "bugs").unwrap();
        assert_eq!(
            prepare_items(&catalog, build, bugs, &json!([{ "Id": 42 }]), "Bugs").unwrap(),
            json!({ "Items": [{ "Id": 42 }] })
        );
        let bug = catalog.find("Bug").unwrap();
        let tags = catalog.collection(bug, "TagObjects").unwrap();
        assert_eq!(
            prepare_items(
                &catalog,
                bug,
                tags,
                &json!([{ "Name": "ui" }]),
                "TagObjects"
            )
            .unwrap(),
            json!([{ "Name": "ui" }])
        );
    }

    #[test]
    fn refuses_read_only_collections_and_bad_items() {
        let catalog = snapshot();
        let story = catalog.find("UserStory").unwrap();
        let bugs = catalog.collection(story, "Bugs").unwrap();
        let e = prepare_items(&catalog, story, bugs, &json!([{ "Id": 1 }]), "Bugs").unwrap_err();
        assert_eq!(e, "Bugs: items cannot be added to UserStory.Bugs");
        let tasks = catalog.collection(story, "Tasks").unwrap();
        assert_eq!(
            prepare_items(&catalog, story, tasks, &json!("x"), "Tasks").unwrap_err(),
            "Tasks: expected an array of items or {\"Items\": [...]}"
        );
        assert_eq!(
            prepare_items(&catalog, story, tasks, &json!([]), "Tasks").unwrap_err(),
            "Tasks: no items given"
        );
        assert_eq!(
            prepare_items(&catalog, story, tasks, &json!([1, { "Id": 0 }]), "Tasks").unwrap_err(),
            "Tasks[0]: expected an object\n- Tasks[1] must be a reference {\"Id\": <number>} (or null to clear); got {\"Id\":0}. Empty references are never sent."
        );
        let custom = catalog.collection(story, "CustomFields").unwrap();
        assert_eq!(
            prepare_items(
                &catalog,
                story,
                custom,
                &json!([{ "Value": 1 }]),
                "CustomFields"
            )
            .unwrap_err(),
            "CustomFields[0]: custom field needs a \"Name\""
        );
    }

    #[test]
    fn references_accept_ids_numeric_strings_and_null() {
        assert_eq!(prepare_reference(&json!(null), "P").unwrap(), Value::Null);
        assert_eq!(
            prepare_reference(&json!(7.0), "P").unwrap(),
            json!({ "Id": 7 })
        );
        assert_eq!(
            prepare_reference(&json!({ "id": "007" }), "P").unwrap(),
            json!({ "Id": 7 })
        );
        assert!(prepare_reference(&json!(1.5), "P").is_err());
        assert!(prepare_reference(&json!("-3"), "P").is_err());
    }

    #[tokio::test]
    async fn catalog_for_body_keeps_the_catalog_for_link_only_items() {
        let catalog = snapshot();
        let provider = Provider::fixed(catalog.clone());
        let story = catalog.find("UserStory").unwrap();
        let body = map(
            json!({ "Tasks": [{ "Name": "t", "Bugs": [{ "Name": "b" }] }], "Bugs": [{ "Id": 1 }] }),
        );
        let got = catalog_for_body(&provider, catalog.clone(), story, &body).await;
        assert!(Arc::ptr_eq(&got, &catalog));
    }

    #[test]
    fn unpersisted_reports_values_the_read_back_does_not_show() {
        let requested = map(json!({
            "Name": "New name",
            "Effort": 5,
            "Project": { "Id": 2 },
            "Owner": null,
            "Description": "<p>a &amp; b</p>",
            "StartDate": "2026-09-11",
            "Missing": 1,
        }));
        let returned = json!({
            "Name": "Old name",
            "Effort": 5.0,
            "Project": { "Id": 3 },
            "Owner": { "Id": 4 },
            "Description": "<div>a & b</div>",
            "StartDate": "2026-09-11T00:00:00.000+02:00",
        });
        assert_eq!(
            unpersisted(&requested, &returned),
            [
                "Name: requested \"New name\", got \"Old name\"",
                "Project: requested Id 2, got 3",
                "Owner: requested null, got {\"Id\":4}",
            ]
        );
        assert!(unpersisted(&requested, &json!(null)).is_empty());
    }

    #[test]
    fn numbers_print_as_javascript_does() {
        assert_eq!(js_json(&json!(5.0)), "5");
        assert_eq!(js_json(&json!(1.5)), "1.5");
        assert_eq!(js_json(&json!(1e21)), "1e+21");
        assert_eq!(js_json(&json!(1e-7)), "1e-7");
        assert_eq!(js_json(&json!({ "a": [1.0, "x"] })), "{\"a\":[1,\"x\"]}");
    }
}
