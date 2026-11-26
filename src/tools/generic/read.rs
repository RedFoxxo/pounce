//! Raw read tools: the catalog, one entity, a query, an inner collection.

use std::sync::LazyLock;

use regex::Regex;
use serde_json::{Map, Value, json};

use super::policy::{is_admin_resource, read_only_in_practice};
use crate::catalog::{CatalogResource, Request, js_trim};
use crate::format::shape::{INNER_CAP, inner_cap_note};
use crate::http::v1::{ListOptions, ReadOptions};
use crate::schema::{Args, Field, field, limit, nonempty, string};
use crate::tools::params::{bracket, id, include, resource};
use crate::tools::{Tool, failed, handler, invalid, success};

/// Inner collections are capped at 25 unless asked; ask for the maximum.
pub const INNER_TAKE: u32 = INNER_CAP as u32;

const QUERY_DEFAULT: u32 = 500;
const QUERY_MAX: u32 = 5000;

fn operations(r: &CatalogResource) -> Vec<&'static str> {
    [
        (r.can_create, "create"),
        (r.can_update, "update"),
        (r.can_delete, "delete"),
    ]
    .into_iter()
    .filter_map(|(can, op)| can.then_some(op))
    .collect()
}

/// `CreateDate desc` → orderByDesc, since v1 rejects the suffix. Empty strings count as absent.
pub fn sort_params(
    order_by: Option<&str>,
    order_by_desc: Option<&str>,
) -> (Option<String>, Option<String>) {
    static SUFFIX: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)^\s*(\S+)\s+(asc|desc)\s*$").expect("valid pattern"));
    let mut asc = None;
    let mut desc = None;
    if let Some(o) = order_by.filter(|o| !o.is_empty()) {
        match SUFFIX.captures(o) {
            Some(m) if m[2].eq_ignore_ascii_case("desc") => desc = Some(m[1].to_string()),
            Some(m) => asc = Some(m[1].to_string()),
            None => asc = Some(js_trim(o).to_string()),
        }
    }
    if let Some(d) = order_by_desc.filter(|d| !d.is_empty()) {
        desc = Some(js_trim(d).to_string());
    }
    (asc, desc)
}

/// The where/include/sort/limit parameters of read_query and read_collection.
fn query_fields() -> Vec<Field> {
    vec![
        field("where", string()).optional().describe(
            "v1 filter, e.g. (EntityState.Name eq 'Open') and (Project.Id eq 2). Operators: eq, ne, gt, gte, lt, lte, contains, in; no 'or'.",
        ),
        include_field(),
        exclude_field(),
        append_field(),
        field("orderBy", string())
            .optional()
            .describe("Sort field; \"Field desc\" is accepted"),
        field("orderByDesc", string())
            .optional()
            .describe("Sort field, descending"),
        limit("limit", QUERY_DEFAULT, QUERY_MAX),
    ]
}

fn include_field() -> Field {
    include("include").describe("Fields to return, e.g. [Id,Name,Project[Name],Tasks[Id,Name]]")
}

fn exclude_field() -> Field {
    include("exclude").describe("Fields to omit")
}

fn append_field() -> Field {
    include("append").describe("Calculated values, e.g. [Tasks-Count,Bugs-Count]")
}

fn read_options(args: &Args) -> ReadOptions {
    ReadOptions {
        include: bracket(args.opt_str("include").as_deref()),
        exclude: bracket(args.opt_str("exclude").as_deref()),
        append: bracket(args.opt_str("append").as_deref()),
        inner_take: Some(INNER_TAKE),
    }
}

fn list_options(args: &Args) -> ListOptions {
    let (order_by, order_by_desc) = sort_params(
        args.opt_str("orderBy").as_deref(),
        args.opt_str("orderByDesc").as_deref(),
    );
    ListOptions {
        read: read_options(args),
        where_: args.opt_str("where"),
        order_by,
        order_by_desc,
        limit: Some(
            args.opt_num("limit")
                .map_or(QUERY_DEFAULT as usize, |n| n as usize),
        ),
    }
}

/// `{...data, ...note}` when the note is not empty.
fn with_note(data: Value, note: Map<String, Value>) -> Value {
    match data {
        Value::Object(mut o) if !note.is_empty() => {
            o.extend(note);
            Value::Object(o)
        }
        other => other,
    }
}

fn read_meta() -> Tool {
    Tool {
        name: "read_meta",
        description: "Describe this instance's resources, or one resource's fields and collections. Use it before other raw tools.",
        fields: vec![resource().optional()],
        handler: handler!(|ctx, args| {
            let name = args.opt_str("resource");
            let request = match &name {
                Some(n) => Request::Names(vec![n.clone()]),
                None => Request::All,
            };
            let catalog = ctx.catalog.get(&request).await;
            let mut out = Map::new();
            out.insert("source".into(), json!(catalog.source.as_str()));
            out.insert("instance".into(), json!(catalog.data.instance));
            if let Some(v) = &catalog.data.version {
                out.insert("version".into(), json!(v));
            }
            out.insert("capturedAt".into(), json!(catalog.data.captured_at));
            let Some(name) = name else {
                let resources: Vec<Value> = catalog
                    .resources()
                    .iter()
                    .map(|r| {
                        let mut o = Map::new();
                        o.insert("name".into(), json!(r.name));
                        o.insert("path".into(), json!(r.path));
                        o.insert(
                            "operations".into(),
                            if r.available {
                                json!(operations(r))
                            } else {
                                json!("unavailable")
                            },
                        );
                        if !r.listed {
                            o.insert("listed".into(), json!(false));
                        }
                        if is_admin_resource(r) {
                            o.insert("admin".into(), json!(true));
                        }
                        Value::Object(o)
                    })
                    .collect();
                out.insert("count".into(), json!(resources.len()));
                out.insert("resources".into(), Value::Array(resources));
                return success(Value::Object(out));
            };
            let r = catalog.resource(&name).map_err(invalid)?;
            out.insert("name".into(), json!(r.name));
            out.insert("path".into(), json!(r.path));
            out.insert("description".into(), json!(r.description));
            out.insert("operations".into(), json!(operations(r)));
            out.insert(
                "writeTools".into(),
                json!(if read_only_in_practice(&r.name).is_some() {
                    "none (read-only in practice)"
                } else if is_admin_resource(r) {
                    "admin_*"
                } else {
                    "write_*/delete_*"
                }),
            );
            out.insert("bases".into(), json!(r.bases));
            let values: Vec<Value> = r
                .values
                .iter()
                .map(|f| {
                    let mut o = Map::new();
                    o.insert("name".into(), json!(f.name));
                    o.insert("type".into(), json!(f.type_name));
                    o.insert("set".into(), json!(f.can_set));
                    if f.required {
                        o.insert("required".into(), json!(true));
                    }
                    if f.deprecated {
                        o.insert("deprecated".into(), json!(true));
                    }
                    Value::Object(o)
                })
                .collect();
            out.insert("values".into(), Value::Array(values));
            let references: Vec<Value> = r
                .references
                .iter()
                .map(|f| {
                    let mut o = Map::new();
                    o.insert("name".into(), json!(f.name));
                    o.insert("type".into(), json!(f.type_name));
                    o.insert("set".into(), json!(f.can_set));
                    if f.required {
                        o.insert("required".into(), json!(true));
                    }
                    Value::Object(o)
                })
                .collect();
            out.insert("references".into(), Value::Array(references));
            let collections: Vec<Value> = r
                .collections
                .iter()
                .map(|c| json!({ "name": c.name, "type": c.type_name, "add": c.can_add, "remove": c.can_remove }))
                .collect();
            out.insert("collections".into(), Value::Array(collections));
            success(Value::Object(out))
        }),
    }
}

fn read_get() -> Tool {
    Tool {
        name: "read_get",
        description: "Get one entity of any resource by id (raw). Prefer read_card for cards.",
        fields: vec![
            resource(),
            id("id"),
            include_field(),
            exclude_field(),
            append_field(),
        ],
        handler: handler!(|ctx, args| {
            let name = args.str("resource");
            let catalog = ctx.catalog.get(&Request::Names(vec![name.clone()])).await;
            let r = catalog.resource(&name).map_err(invalid)?;
            let id = args.id("id");
            let data = ctx
                .v1
                .get(&r.path, id, &read_options(&args))
                .await
                .map_err(failed(format!("Could not read {} {id}", r.name)))?;
            let note = inner_cap_note(&data);
            success(with_note(data, note))
        }),
    }
}

fn read_query() -> Tool {
    let mut fields = vec![resource()];
    fields.extend(query_fields());
    Tool {
        name: "read_query",
        description: "Query any resource with where/include/orderBy (raw), fully paged. Use include to keep results small.",
        fields,
        handler: handler!(|ctx, args| {
            let name = args.str("resource");
            let catalog = ctx.catalog.get(&Request::Names(vec![name.clone()])).await;
            let r = catalog.resource(&name).map_err(invalid)?;
            let result = ctx
                .v1
                .list(&r.path, &list_options(&args))
                .await
                .map_err(failed(format!("Query on {} failed", r.path)))?;
            let items = Value::Array(result.items);
            let mut out = Map::new();
            out.insert("resource".into(), json!(r.name));
            out.insert("count".into(), json!(items.as_array().map_or(0, Vec::len)));
            out.insert("truncated".into(), json!(result.truncated));
            out.extend(inner_cap_note(&items));
            out.insert("items".into(), items);
            success(Value::Object(out))
        }),
    }
}

fn read_collection() -> Tool {
    let mut fields = vec![
        resource(),
        id("id"),
        field("collection", nonempty())
            .describe("Collection name, e.g. Tasks, Comments, Assignments"),
    ];
    fields.extend(query_fields());
    Tool {
        name: "read_collection",
        description: "Get an inner collection of an entity, e.g. UserStories/{id}/Tasks (raw), fully paged.",
        fields,
        handler: handler!(|ctx, args| {
            let name = args.str("resource");
            let catalog = ctx.catalog.get(&Request::Names(vec![name.clone()])).await;
            let r = catalog.resource(&name).map_err(invalid)?;
            let coll = catalog
                .collection(r, &args.str("collection"))
                .map_err(invalid)?;
            let id = args.id("id");
            let path = format!("{}/{id}/{}", r.path, coll.name);
            let result = ctx
                .v1
                .list(&path, &list_options(&args))
                .await
                .map_err(failed(format!("Could not read {path}")))?;
            let items = Value::Array(result.items);
            let mut out = Map::new();
            out.insert("resource".into(), json!(r.name));
            out.insert("id".into(), json!(id));
            out.insert("collection".into(), json!(coll.name));
            out.insert("count".into(), json!(items.as_array().map_or(0, Vec::len)));
            out.insert("truncated".into(), json!(result.truncated));
            out.extend(inner_cap_note(&items));
            out.insert("items".into(), items);
            success(Value::Object(out))
        }),
    }
}

pub fn read_tools() -> Vec<Tool> {
    vec![read_meta(), read_get(), read_query(), read_collection()]
}

#[cfg(test)]
mod tests {
    use super::sort_params;

    #[test]
    fn moves_a_desc_suffix_to_order_by_desc() {
        assert_eq!(
            sort_params(Some(" CreateDate DESC "), None),
            (None, Some("CreateDate".into()))
        );
        assert_eq!(
            sort_params(Some("Name asc"), None),
            (Some("Name".into()), None)
        );
        assert_eq!(
            sort_params(Some(" Name "), Some(" Id ")),
            (Some("Name".into()), Some("Id".into()))
        );
    }
}
