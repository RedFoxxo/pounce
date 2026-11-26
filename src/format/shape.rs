//! Compact answers: references as `{id, name}`, people with a display name, empty
//! and null fields dropped, HTML as text.

use serde_json::{Map, Value, json};

use super::html::html_to_text;

/// An object from key/value pairs, without the null ones (JSON `null` stands for
/// JavaScript's `undefined` and `null` alike).
pub fn compact<K: Into<String>>(pairs: impl IntoIterator<Item = (K, Value)>) -> Value {
    Value::Object(
        pairs
            .into_iter()
            .filter(|(_, v)| !v.is_null())
            .map(|(k, v)| (k.into(), v))
            .collect::<Map<String, Value>>(),
    )
}

/// `{Id, Name, ...}` → `{id, name}`; null when absent.
pub fn reference(value: &Value) -> Value {
    let Some(id) = value.get("Id").filter(|v| v.is_number()) else {
        return Value::Null;
    };
    match value.get("Name") {
        Some(Value::String(name)) => json!({ "id": id, "name": name }),
        _ => json!({ "id": id }),
    }
}

fn nonempty_str(value: Option<&Value>) -> Option<&str> {
    value.and_then(Value::as_str).filter(|s| !s.is_empty())
}

/// A user/general-user reference with a display name and login; null when absent.
pub fn person(value: &Value) -> Value {
    let Some(id) = value.get("Id").filter(|v| v.is_number()) else {
        return Value::Null;
    };
    let joined = [value.get("FirstName"), value.get("LastName")]
        .into_iter()
        .filter_map(nonempty_str)
        .collect::<Vec<_>>()
        .join(" ");
    let name = match nonempty_str(value.get("FullName")) {
        Some(full) => full.to_string(),
        None if !joined.is_empty() => joined,
        None => match value.get("Login") {
            Some(Value::String(login)) => login.clone(),
            _ => format!("User {id}"),
        },
    };
    match nonempty_str(value.get("Login")) {
        Some(login) => json!({ "id": id, "name": name, "login": login }),
        None => json!({ "id": id, "name": name }),
    }
}

/// Items of a v1 inner collection value (`{Items: [...]}`) or a plain array, objects only.
pub fn items(value: &Value) -> Vec<Value> {
    let list = match value {
        Value::Array(a) => a,
        Value::Object(o) => match o.get("Items") {
            Some(Value::Array(a)) => a,
            _ => return Vec::new(),
        },
        _ => return Vec::new(),
    };
    list.iter().filter(|v| v.is_object()).cloned().collect()
}

/// HTML (or Markdown) as text; null when absent or empty.
pub fn text(value: &Value) -> Value {
    match value {
        Value::String(s) => {
            let t = html_to_text(Some(s));
            if t.is_empty() { Value::Null } else { json!(t) }
        }
        _ => Value::Null,
    }
}

/// The number, or null.
pub fn num(value: &Value) -> Value {
    if value.is_number() {
        value.clone()
    } else {
        Value::Null
    }
}

/// `"a, b"` → `["a","b"]`; null when there are none.
pub fn tags(value: &Value) -> Value {
    match value {
        Value::String(s) if !s.trim().is_empty() => json!(
            s.split(',')
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .collect::<Vec<_>>()
        ),
        _ => Value::Null,
    }
}

/// `[{Name, Type, Value}]` → `{Name: Value}`; null when there are none.
pub fn custom_fields(value: &Value) -> Value {
    let Value::Array(list) = value else {
        return Value::Null;
    };
    let mut out = Map::new();
    for f in list {
        let Some(name) = f.get("Name").and_then(Value::as_str) else {
            continue;
        };
        let raw = f.get("Value").cloned().unwrap_or(Value::Null);
        let shown = if f.get("Type").and_then(Value::as_str) == Some("RichText") {
            text(&raw)
        } else {
            raw
        };
        out.insert(name.to_string(), shown);
    }
    if out.is_empty() {
        Value::Null
    } else {
        Value::Object(out)
    }
}

fn field<'a>(raw: &'a Value, key: &str) -> &'a Value {
    raw.get(key).unwrap_or(&Value::Null)
}

pub fn assignment(raw: &Value) -> Value {
    compact([
        ("id", num(field(raw, "Id"))),
        ("user", person(field(raw, "GeneralUser"))),
        ("role", reference(field(raw, "Role"))),
    ])
}

pub fn role_effort(raw: &Value) -> Value {
    compact([
        ("id", num(field(raw, "Id"))),
        ("role", reference(field(raw, "Role"))),
        ("effort", num(field(raw, "Effort"))),
        ("completed", num(field(raw, "EffortCompleted"))),
        ("toDo", num(field(raw, "EffortToDo"))),
    ])
}

pub fn team_assignment(raw: &Value) -> Value {
    compact([
        ("id", num(field(raw, "Id"))),
        ("team", reference(field(raw, "Team"))),
        ("state", reference(field(raw, "EntityState"))),
    ])
}

/// Compact one-line listing of a card from a query.
pub fn card_summary(raw: &Value) -> Value {
    let name_of = |key: &str| match raw.get(key) {
        Some(Value::Object(o)) => o.get("Name").cloned().unwrap_or(Value::Null),
        _ => Value::Null,
    };
    let kind = match raw.get("EntityType") {
        Some(Value::Object(o)) => o.get("Name").cloned().unwrap_or(Value::Null),
        _ => field(raw, "ResourceType").clone(),
    };
    compact([
        ("id", num(field(raw, "Id"))),
        ("type", kind),
        ("name", field(raw, "Name").clone()),
        ("state", name_of("EntityState")),
        ("project", name_of("Project")),
        ("tags", tags(field(raw, "Tags"))),
        ("modified", field(raw, "ModifyDate").clone()),
    ])
}

/// Inner collections are read with innerTake=1000; a collection that size may have been cut.
pub const INNER_CAP: usize = 1000;

/// Names of included inner collections that reached the cap, in one entity or a list of them.
pub fn capped_inner(value: &Value) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut visit = |entity: &Value| {
        let Value::Object(o) = entity else { return };
        for (key, v) in o {
            if let Some(Value::Array(a)) = v.get("Items")
                && v.is_object()
                && a.len() >= INNER_CAP
                && !out.contains(key)
            {
                out.push(key.clone());
            }
        }
    };
    match value {
        Value::Array(list) => list.iter().for_each(&mut visit),
        other => visit(other),
    }
    out
}

/// `{innerCapped, innerCappedNote}` when any included collection may be incomplete; empty otherwise.
pub fn inner_cap_note(value: &Value) -> Map<String, Value> {
    let capped = capped_inner(value);
    let mut out = Map::new();
    if !capped.is_empty() {
        out.insert("innerCapped".into(), json!(capped));
        out.insert(
            "innerCappedNote".into(),
            json!(format!(
                "these inner collections returned {INNER_CAP} items and may be incomplete; read them with read_collection"
            )),
        );
    }
    out
}

/// A computed number as JavaScript prints it: whole numbers without a fraction.
pub fn number(n: f64) -> Value {
    if n.fract() == 0.0 && n.abs() < 9_007_199_254_740_992.0 {
        json!(n as i64)
    } else {
        json!(n)
    }
}
