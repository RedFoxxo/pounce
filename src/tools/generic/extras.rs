//! Raw API surface beyond the catalog: v2 queries, history, context, conversions,
//! storage, attachment upload and undelete.

use std::sync::LazyLock;

use base64::Engine;
use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};
use regex::Regex;
use serde_json::{Map, Value, json};

use crate::catalog::Request;
use crate::format::html::html_to_text;
use crate::format::shape::{compact, num, person, reference};
use crate::http::v1::{ListOptions, ReadOptions};
use crate::http::v2::QueryOptions;
use crate::http::{Body, FormField, Method, Query, Spec, encode_component};
use crate::resolve::card::card_info;
use crate::schema::{Field, Kind, field, limit, list, pattern, string};
use crate::tools::params::{id, resource};
use crate::tools::workflow::common::unresolved;
use crate::tools::{Tool, failed, failure, handler, invalid, success};

/// A string argument, unless empty (JavaScript falsy).
fn given(args: &crate::schema::Args, name: &str) -> Option<String> {
    args.opt_str(name).filter(|s| !s.is_empty())
}

/// `.name` of a reference, or null.
fn ref_name(value: &Value) -> Value {
    reference(value).get("name").cloned().unwrap_or(Value::Null)
}

// ---------------------------------------------------------------- v2

fn read_v2_query() -> Tool {
    Tool {
        name: "read_v2_query",
        description: "Targetprocess v2 query (raw): select, where with and/or, aggregations. v2 omits null fields: absence means null.",
        fields: vec![
            field(
                "entity",
                pattern("^[A-Za-z]+$", Some("entity name, e.g. userStory")),
            )
            .describe("Singular, camelCase: userStory, bug, feature"),
            field("select", string())
                .optional()
                .describe("e.g. {id,name,tasks.count as tasks}"),
            field("where", string())
                .optional()
                .describe("v2 syntax, e.g. (entityState.isFinal==false and project.id==2)"),
            field("result", string())
                .optional()
                .describe("Aggregation, e.g. {count:count,effort:sum(effort)}; returns one object"),
            field("orderBy", string())
                .optional()
                .describe("e.g. \"createDate desc\""),
            field("filter", string())
                .optional()
                .describe("Board filter DSL"),
            field("includeDeleted", Kind::Bool).optional(),
            limit("limit", 500, 5000),
        ],
        handler: handler!(|ctx, args| {
            let entity = args.str("entity");
            let where_ = given(&args, "where");
            let filter = given(&args, "filter");
            if let Some(result) = given(&args, "result") {
                let data = ctx
                    .v2
                    .aggregate(&entity, &result, where_.as_deref(), filter.as_deref())
                    .await
                    .map_err(failed(format!("v2 aggregation on {entity} failed")))?;
                return success(json!({ "entity": entity, "result": data }));
            }
            let options = QueryOptions {
                select: given(&args, "select"),
                where_,
                order_by: given(&args, "orderBy"),
                filter,
                include_deleted: args.bool("includeDeleted"),
                limit: Some(args.opt_num("limit").map_or(500, |n| n as usize)),
            };
            let r = ctx
                .v2
                .query(&entity, &options)
                .await
                .map_err(failed(format!("v2 query on {entity} failed")))?;
            success(json!({
                "entity": entity,
                "count": r.items.len(),
                "truncated": r.truncated,
                "items": r.items,
            }))
        }),
    }
}

fn read_deleted() -> Tool {
    Tool {
        name: "read_deleted",
        description: "Deleted projects or users, with their delete date (restore: admin_undelete).",
        fields: vec![
            field("kind", Kind::OneOf(&["projects", "users"])),
            limit("limit", 500, 5000),
        ],
        handler: handler!(|ctx, args| {
            let kind = args.str("kind");
            let select = if kind == "users" {
                "{id,firstName,lastName,login,email,deleteDate}"
            } else {
                "{id,name,abbreviation,deleteDate}"
            };
            let options = QueryOptions {
                select: Some(select.into()),
                where_: Some("(deleteDate!=null)".into()),
                include_deleted: true,
                limit: Some(args.opt_num("limit").map_or(500, |n| n as usize)),
                ..Default::default()
            };
            let r = ctx
                .v2
                .query(&kind, &options)
                .await
                .map_err(failed(format!("Could not read deleted {kind}")))?;
            success(json!({
                "kind": kind,
                "count": r.items.len(),
                "truncated": r.truncated,
                "items": r.items,
            }))
        }),
    }
}

// ---------------------------------------------------------------- history

/// Bookkeeping fields Targetprocess flags on nearly every record.
const HISTORY_NOISE: [&str; 3] = ["ModifyDate", "CreateDate", "CreateDateOffset"];

fn history_entry(raw: &Value) -> Value {
    static TAG: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"<[A-Za-z/][^>]*>").expect("valid pattern"));
    let empty = Map::new();
    let fields = raw.as_object().unwrap_or(&empty);
    // `Changes` is only returned when explicitly included; the IsChanged{Field} flags always are.
    let listed = match fields.get("Changes") {
        Some(Value::String(s)) => s.split(',').map(|c| c.trim().to_string()).collect(),
        _ => Vec::new(),
    };
    let flagged = fields
        .iter()
        .filter(|(k, v)| k.starts_with("IsChanged") && **v == Value::Bool(true))
        .map(|(k, _)| k["IsChanged".len()..].to_string());
    let is_add = fields.get("Modification") == Some(&json!("Add"));
    let mut changes: Vec<String> = Vec::new();
    for c in listed.into_iter().chain(flagged) {
        if !changes.contains(&c) {
            changes.push(c);
        }
    }
    changes.retain(|c| !c.is_empty() && !(HISTORY_NOISE.contains(&c.as_str()) && !is_add));
    let mut values = Map::new();
    for field in &changes {
        let Some(v) = fields.get(field) else { continue };
        let shown = match v {
            Value::Object(_) | Value::Array(_) => {
                let r = reference(v);
                match (r.get("name"), r.get("id")) {
                    (Some(name), _) => name.clone(),
                    (None, Some(id)) => id.clone(),
                    _ => v.clone(),
                }
            }
            Value::String(s) if TAG.is_match(s) => json!(html_to_text(Some(s))),
            other => other.clone(),
        };
        values.insert(field.clone(), shown);
    }
    compact([
        ("id", num(&raw["Id"])),
        ("date", raw["Date"].clone()),
        ("modification", raw["Modification"].clone()),
        ("by", person(&raw["Modifier"])),
        ("changed", json!(changes)),
        (
            "values",
            if values.is_empty() {
                Value::Null
            } else {
                Value::Object(values)
            },
        ),
    ])
}

fn simple_entry(raw: &Value) -> Value {
    compact([
        ("id", num(&raw["Id"])),
        ("date", raw["Date"].clone()),
        ("by", person(&raw["Modifier"])),
        ("state", ref_name(&raw["EntityState"])),
        ("effort", num(&raw["Effort"])),
        ("effortCompleted", num(&raw["EffortCompleted"])),
        ("effortToDo", num(&raw["EffortToDo"])),
        ("release", ref_name(&raw["Release"])),
        ("iteration", ref_name(&raw["Iteration"])),
        ("project", ref_name(&raw["Project"])),
    ])
}

fn read_history() -> Tool {
    Tool {
        name: "read_history",
        description: "Change history of an entity, oldest first: state, effort, release, iteration; full: true for every field change.",
        fields: vec![
            id("id"),
            resource()
                .optional()
                .describe("Omit for cards; needed for other entities, e.g. Comment"),
            field("full", Kind::Bool).optional(),
            limit("limit", 200, 5000),
        ],
        handler: handler!(|ctx, args| {
            let id = args.id("id");
            let name = args.opt_str("resource");
            let catalog = ctx
                .catalog
                .get(&Request::Names(name.iter().cloned().collect()))
                .await;
            let r = match &name {
                Some(n) => catalog.resource(n).map_err(invalid)?.clone(),
                None => {
                    card_info(ctx, id)
                        .await
                        .map_err(|e| unresolved(&e))?
                        .resource
                }
            };
            let max = args.opt_num("limit").map_or(200, |n| n as usize);
            if args.bool("full") {
                let hist_name = format!("{}Histories", r.name);
                let hist_catalog = ctx
                    .catalog
                    .get(&Request::Names(vec![hist_name.clone()]))
                    .await;
                let Some(hist) = hist_catalog.find(&hist_name).filter(|h| h.available) else {
                    return Err(invalid(format!(
                        "{} has no full history resource ({hist_name}) on this instance",
                        r.name
                    )));
                };
                let options = ListOptions {
                    where_: Some(format!("(SourceEntityId eq {id})")),
                    order_by_desc: Some("Date".into()),
                    limit: Some(max),
                    ..Default::default()
                };
                let res = ctx
                    .v1
                    .list(&hist.path, &options)
                    .await
                    .map_err(failed(format!(
                        "Could not read the full history of {} {id}",
                        r.name
                    )))?;
                let entries: Vec<Value> = res.items.iter().rev().map(history_entry).collect();
                return success(json!({
                    "resource": r.name,
                    "id": id,
                    "kind": "full",
                    "count": res.items.len(),
                    "truncated": res.truncated,
                    "entries": entries,
                }));
            }
            let inner = r.collections.iter().any(|c| c.name == "History");
            let (path, where_) = if inner {
                (format!("{}/{id}/History", r.path), None)
            } else {
                let simple_name = format!("{}SimpleHistories", r.name);
                let simple_catalog = ctx
                    .catalog
                    .get(&Request::Names(vec![simple_name.clone()]))
                    .await;
                let Some(simple) = simple_catalog.find(&simple_name).filter(|s| s.available) else {
                    return Err(invalid(format!(
                        "{} has no simple history; try full: true",
                        r.name
                    )));
                };
                (
                    simple.path.clone(),
                    Some(format!("({}.Id eq {id})", r.name)),
                )
            };
            let options = ListOptions {
                where_,
                order_by_desc: Some("Date".into()),
                limit: Some(max),
                ..Default::default()
            };
            let res = ctx.v1.list(&path, &options).await.map_err(failed(format!(
                "Could not read the history of {} {id}",
                r.name
            )))?;
            let entries: Vec<Value> = res.items.iter().rev().map(simple_entry).collect();
            success(json!({
                "resource": r.name,
                "id": id,
                "kind": "simple",
                "count": res.items.len(),
                "truncated": res.truncated,
                "entries": entries,
            }))
        }),
    }
}

// ---------------------------------------------------------------- context, conversions

fn joined(ids: Option<Vec<i64>>) -> Option<String> {
    ids.filter(|ids| !ids.is_empty())
        .map(|ids| ids.iter().map(i64::to_string).collect::<Vec<_>>().join(","))
}

fn read_context() -> Tool {
    Tool {
        name: "read_context",
        description: "Targetprocess Context for entities, projects or teams: processes, practices, terms, custom field definitions.",
        fields: vec![
            field("ids", list(Kind::Id, None, None))
                .optional()
                .describe("Entity ids"),
            field("projectIds", list(Kind::Id, None, None)).optional(),
            field("teamIds", list(Kind::Id, None, None)).optional(),
            field("acid", pattern("^[0-9A-Fa-f]+$", None)).optional(),
        ],
        handler: handler!(|ctx, args| {
            let mut query = Query::new();
            for name in ["ids", "projectIds", "teamIds"] {
                if let Some(value) = joined(args.opt_ids(name)) {
                    query = query.with(name, value);
                }
            }
            if let Some(acid) = given(&args, "acid") {
                query = query.with("acid", acid);
            }
            let data = ctx
                .v1
                .get_path("Context", query)
                .await
                .map_err(failed("Could not read the context"))?;
            success(data)
        }),
    }
}

fn conversion(c: &Value) -> Value {
    let actual = &c["ActualGeneral"];
    let actual_ref = reference(actual);
    let mut from = Map::new();
    for (key, value) in [
        ("id", num(&c["FromGeneralID"])),
        ("type", ref_name(&c["FromGeneralType"])),
    ] {
        if !value.is_null() {
            from.insert(key.into(), value);
        }
    }
    compact([
        ("from", Value::Object(from)),
        (
            "now",
            compact([
                ("id", actual_ref.get("id").cloned().unwrap_or(Value::Null)),
                (
                    "name",
                    actual_ref.get("name").cloned().unwrap_or(Value::Null),
                ),
                ("type", actual["EntityType"]["Name"].clone()),
            ]),
        ),
    ])
}

fn read_conversions() -> Tool {
    Tool {
        name: "read_conversions",
        description: "Type conversions of a card: its current id if it was converted, and the ids it was converted from.",
        fields: vec![id("id")],
        handler: handler!(|ctx, args| {
            let id = args.id("id");
            let include = "[Id,FromGeneralID,FromGeneralType[Id,Name],ActualGeneral[Id,Name,EntityType[Name]]]";
            let options = |where_: String| ListOptions {
                read: ReadOptions {
                    include: Some(include.into()),
                    ..Default::default()
                },
                where_: Some(where_),
                ..Default::default()
            };
            let from_options = options(format!("(FromGeneralID eq {id})"));
            let to_options = options(format!("(ActualGeneral.Id eq {id})"));
            let (from, to) = tokio::join!(
                ctx.v1.list("GeneralConversions", &from_options),
                ctx.v1.list("GeneralConversions", &to_options),
            );
            let summary = format!("Could not read conversions of {id}");
            let from = from.map_err(failed(summary.clone()))?;
            let to = to.map_err(failed(summary))?;
            success(json!({
                "id": id,
                "convertedTo": from.items.iter().map(conversion).collect::<Vec<_>>(),
                "convertedFrom": to.items.iter().map(conversion).collect::<Vec<_>>(),
            }))
        }),
    }
}

// ---------------------------------------------------------------- storage

/// JavaScript's `\w` is ASCII; the regex crate's is Unicode. The published pattern
/// stays JavaScript's, and this check refuses what only the Unicode `\w` lets through.
fn ascii_storage_name(s: &str) -> bool {
    static UNICODE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^[\w.\-]+$").expect("valid pattern"));
    s.is_ascii() || !UNICODE.is_match(s)
}

const STORAGE_MESSAGE: &str = "letters, digits, _ . - only";

fn storage_name(name: &'static str) -> Field {
    let Kind::Str { pattern, .. } = pattern(r"^[\w.\-]+$", Some(STORAGE_MESSAGE)) else {
        unreachable!("pattern() builds a string kind")
    };
    field(
        name,
        Kind::Str {
            trim: false,
            min: None,
            pattern,
            refine: Some((ascii_storage_name, STORAGE_MESSAGE)),
        },
    )
}

fn storage_path(group: Option<&str>, key: Option<&str>) -> String {
    format!(
        "/storage/v1/{}{}",
        group.map(encode_component).unwrap_or_default(),
        key.map(|k| format!("/{}", encode_component(k)))
            .unwrap_or_default()
    )
}

fn read_storage() -> Tool {
    Tool {
        name: "read_storage",
        description: "RESTful storage (views, boards, settings): list groups, a group's storages, or one storage by group and key.",
        fields: vec![
            storage_name("group").optional(),
            storage_name("key").optional(),
            field("select", string())
                .optional()
                .describe("Storage syntax, e.g. {key,publicData.name}"),
            field("where", string())
                .optional()
                .describe("Storage syntax, e.g. (scope == \"Public\")"),
            limit("limit", 100, 1000),
        ],
        handler: handler!(|ctx, args| {
            let group = given(&args, "group");
            let key = given(&args, "key");
            if key.is_some() && group.is_none() {
                return Err(invalid("key needs a group"));
            }
            let path = storage_path(group.as_deref(), key.as_deref());
            let query = if key.is_some() {
                Query::new()
            } else {
                let take = if group.is_some() {
                    args.opt_num("limit").map_or(100, |n| n as u64)
                } else {
                    1000
                };
                Query::new()
                    .set("select", args.opt_str("select"))
                    .set("where", args.opt_str("where"))
                    .with("take", take)
            };
            let data = ctx
                .http
                .request(Spec::new(Method::Get, path, query))
                .await
                .map_err(failed(format!(
                    "Could not read storage {}{}",
                    group.as_deref().unwrap_or(""),
                    key.as_deref().map(|k| format!("/{k}")).unwrap_or_default()
                )))?;
            if key.is_some() {
                return success(data);
            }
            let items = match data.get("items") {
                Some(Value::Array(a)) => a.clone(),
                _ => Vec::new(),
            };
            let truncated = data.get("next").is_some_and(truthy);
            let mut out = Map::new();
            if let Some(g) = &group {
                out.insert("group".into(), json!(g));
            }
            out.insert("count".into(), json!(items.len()));
            out.insert("truncated".into(), json!(truncated));
            out.insert("items".into(), Value::Array(items));
            success(Value::Object(out))
        }),
    }
}

fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0 && !f.is_nan()),
        Value::String(s) => !s.is_empty(),
        _ => true,
    }
}

/// `JSON.stringify` equality, as the read-back check compares values.
fn same_json(a: Option<&Value>, b: &Value) -> bool {
    a.map(Value::to_string) == Some(b.to_string())
}

fn write_storage() -> Tool {
    Tool {
        name: "write_storage",
        description: "Create or merge a storage entry: posted keys are set, other keys kept, null deletes a key. Owner or admin only.",
        fields: vec![
            storage_name("group"),
            storage_name("key")
                .optional()
                .describe("Omit to let Targetprocess generate one"),
            field("scope", Kind::OneOf(&["Public", "Private"])).optional(),
            field("publicData", Kind::Record(None)).optional(),
            field("userData", Kind::Record(None)).optional(),
        ],
        handler: handler!(|ctx, args| {
            let group = args.str("group");
            let key = given(&args, "key");
            let scope = given(&args, "scope");
            let public_data = args.opt_map("publicData").cloned();
            let user_data = args.opt_map("userData").cloned();
            if scope.is_none() && public_data.is_none() && user_data.is_none() {
                return Err(invalid("pass scope, publicData and/or userData"));
            }
            let mut body = Map::new();
            if let Some(s) = &scope {
                body.insert("scope".into(), json!(s));
            }
            if let Some(d) = &public_data {
                body.insert("publicData".into(), Value::Object(d.clone()));
            }
            if let Some(d) = &user_data {
                body.insert("userData".into(), Value::Object(d.clone()));
            }
            let path = storage_path(Some(&group), key.as_deref());
            let label = format!(
                "{group}{}",
                key.as_deref().map(|k| format!("/{k}")).unwrap_or_default()
            );
            let data = ctx
                .http
                .request(Spec::new(Method::Post, path, Query::new()).json(Value::Object(body)))
                .await
                .map_err(failed(format!("Could not write storage {label}")))?;
            let key = key.or_else(|| data.get("key").and_then(Value::as_str).map(str::to_string));
            let back = match &key {
                Some(k) => Some(
                    ctx.http
                        .request(Spec::new(
                            Method::Get,
                            storage_path(Some(&group), Some(k)),
                            Query::new(),
                        ))
                        .await,
                ),
                None => None,
            };
            let mut not_persisted: Vec<String> = Vec::new();
            if let Some(Ok(back)) = &back {
                for (section, sent) in [("publicData", &public_data), ("userData", &user_data)] {
                    let got = back.get(section).and_then(Value::as_object);
                    for (k, v) in sent.iter().flatten() {
                        let current = got.and_then(|g| g.get(k));
                        let differs = if v.is_null() {
                            current.is_some_and(|c| !c.is_null())
                        } else {
                            !same_json(current, v)
                        };
                        if differs {
                            not_persisted.push(format!("{section}.{k}"));
                        }
                    }
                }
            }
            let mut out = Map::new();
            out.insert("group".into(), json!(group));
            if let Some(k) = &key {
                out.insert("key".into(), json!(k));
            }
            out.insert(
                "storage".into(),
                match back {
                    Some(Ok(back)) => back,
                    _ => data,
                },
            );
            if !not_persisted.is_empty() {
                out.insert("notPersisted".into(), json!(not_persisted));
            }
            success(Value::Object(out))
        }),
    }
}

fn delete_storage() -> Tool {
    Tool {
        name: "delete_storage",
        description: "Delete one storage entry (owner or admin only).",
        fields: vec![storage_name("group"), storage_name("key")],
        handler: handler!(|ctx, args| {
            let group = args.str("group");
            let key = args.str("key");
            let path = storage_path(Some(&group), Some(&key));
            ctx.http
                .request(Spec::new(Method::Get, path.clone(), Query::new()))
                .await
                .map_err(failed(format!(
                    "Could not read storage {group}/{key}; nothing was deleted"
                )))?;
            ctx.http
                .request(Spec::new(Method::Delete, path, Query::new()))
                .await
                .map_err(failed(format!("Could not delete storage {group}/{key}")))?;
            success(json!({ "deleted": { "group": group, "key": key } }))
        }),
    }
}

// ---------------------------------------------------------------- attachments

const MAX_UPLOAD_BYTES: usize = 50 * 1024 * 1024;

/// Decodes as Node's `Buffer.from(text, 'base64')`: padding optional, a dangling
/// last character ignored.
fn decode_base64(text: &str) -> Option<Vec<u8>> {
    const LENIENT: GeneralPurpose = GeneralPurpose::new(
        &base64::alphabet::STANDARD,
        GeneralPurposeConfig::new()
            .with_decode_padding_mode(DecodePaddingMode::Indifferent)
            .with_decode_allow_trailing_bits(true),
    );
    let data = text.trim_end_matches('=');
    let data = if data.len() % 4 == 1 {
        &data[..data.len() - 1]
    } else {
        data
    };
    LENIENT.decode(data).ok()
}

/// `String(value)` for the names Targetprocess returns.
fn js_string(value: Option<&Value>) -> String {
    match value {
        None => "undefined".into(),
        Some(Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
    }
}

fn write_attachment() -> Tool {
    Tool {
        name: "write_attachment",
        description: "Upload base64-encoded files to a card and read them back; pounce never reads local files.",
        fields: vec![
            id("id"),
            field(
                "files",
                list(
                    Kind::Object(vec![
                        field("name", crate::schema::nonempty()),
                        field("contentBase64", crate::schema::nonempty()),
                        field("mimeType", string()).optional(),
                    ]),
                    Some(1),
                    Some(10),
                ),
            ),
        ],
        handler: handler!(|ctx, args| {
            static BASE64: LazyLock<Regex> =
                LazyLock::new(|| Regex::new(r"^[A-Za-z0-9+/]+={0,2}$").expect("valid pattern"));
            let id = args.id("id");
            let mut form = vec![FormField::Text {
                name: "generalId".into(),
                value: id.to_string(),
            }];
            let mut names: Vec<String> = Vec::new();
            for f in args.opt_list("files").into_iter().flatten() {
                let name = f["name"].as_str().unwrap_or_default();
                let content: String = f["contentBase64"]
                    .as_str()
                    .unwrap_or_default()
                    .chars()
                    .filter(|c| !crate::catalog::is_js_space(*c))
                    .collect();
                let bytes = BASE64
                    .is_match(&content)
                    .then(|| decode_base64(&content))
                    .flatten()
                    .ok_or_else(|| invalid(format!("{name}: contentBase64 is not valid base64")))?;
                if bytes.len() > MAX_UPLOAD_BYTES {
                    return Err(invalid(format!(
                        "{name} is larger than {} MB",
                        MAX_UPLOAD_BYTES / 1024 / 1024
                    )));
                }
                names.push(name.to_string());
                form.push(FormField::File {
                    name: "file".into(),
                    filename: name.to_string(),
                    content_type: f["mimeType"]
                        .as_str()
                        .unwrap_or("application/octet-stream")
                        .to_string(),
                    bytes,
                });
            }
            // Existing attachments first, so an older file with the same name cannot pass for a failed upload.
            let options = ListOptions {
                read: ReadOptions {
                    include: Some("[Id,Name,Date]".into()),
                    ..Default::default()
                },
                where_: Some(format!("(General.Id eq {id})")),
                limit: Some(20_000),
                ..Default::default()
            };
            let before = ctx
                .v1
                .list("Attachments", &options)
                .await
                .map_err(failed(format!(
                    "Could not read the attachments of {id}; nothing was uploaded"
                )))?;
            let known: Vec<Value> = before.items.iter().map(|a| num(&a["Id"])).collect();
            let mut spec = Spec::new(Method::Post, "/UploadFile.ashx", Query::new()).text();
            spec.body = Body::Form(form);
            ctx.http
                .request(spec)
                .await
                .map_err(failed(format!("Could not upload to {id}")))?;
            let back = ctx.v1.list("Attachments", &options).await;
            let fresh: Vec<&Value> = match &back {
                Ok(b) => b
                    .items
                    .iter()
                    .filter(|a| !known.contains(&num(&a["Id"])))
                    .collect(),
                Err(_) => Vec::new(),
            };
            let mut missing = names.clone();
            for a in &fresh {
                let name = js_string(a.get("Name"));
                if let Some(i) = missing.iter().position(|m| *m == name) {
                    missing.remove(i);
                }
            }
            let attachments: Vec<Value> = fresh
                .iter()
                .filter(|a| names.contains(&js_string(a.get("Name"))))
                .map(|a| {
                    let mut o = Map::new();
                    let n = num(&a["Id"]);
                    if !n.is_null() {
                        o.insert("id".into(), n);
                    }
                    if let Some(name) = a.get("Name") {
                        o.insert("name".into(), name.clone());
                    }
                    Value::Object(o)
                })
                .collect();
            let mut out = Map::new();
            out.insert("card".into(), json!(id));
            out.insert("uploaded".into(), json!(names));
            out.insert("attachments".into(), Value::Array(attachments));
            match &back {
                Ok(_) if !missing.is_empty() => {
                    out.insert(
                        "notPersisted".into(),
                        json!(
                            missing
                                .iter()
                                .map(|n| format!("{n} did not appear as a new attachment"))
                                .collect::<Vec<_>>()
                        ),
                    );
                }
                Ok(_) => {}
                Err(e) => {
                    out.insert(
                        "warning".into(),
                        json!(format!("Could not read attachments back: {}", e.message)),
                    );
                }
            }
            success(Value::Object(out))
        }),
    }
}

// ---------------------------------------------------------------- undelete

fn admin_undelete() -> Tool {
    Tool {
        name: "admin_undelete",
        description: "Restore deleted entities by id and entity type; needs an administrator token. Comments cannot be undeleted.",
        fields: vec![field(
            "items",
            list(
                Kind::Object(vec![
                    id("id"),
                    field("entityType", pattern("^[A-Za-z]+$", None)),
                ]),
                Some(1),
                Some(500),
            ),
        )],
        handler: handler!(|ctx, args| {
            let items = args.opt_list("items").cloned().unwrap_or_default();
            let refused: Vec<&str> = items
                .iter()
                .filter_map(|i| i["entityType"].as_str())
                .filter(|t| {
                    ["comment", "milestone", "program"].contains(&t.to_lowercase().as_str())
                })
                .collect();
            if !refused.is_empty() {
                return Err(invalid(format!(
                    "{} cannot be undeleted",
                    refused.join(", ")
                )));
            }
            let payload: Vec<Value> = items
                .iter()
                .map(|i| json!({ "Id": i["id"], "EntityType": i["entityType"] }))
                .collect();
            let r = if payload.len() == 1 {
                ctx.v1
                    .post_path("undelete", payload[0].clone(), Query::new())
                    .await
            } else {
                ctx.v1
                    .post_path("undelete/bulk", Value::Array(payload.clone()), Query::new())
                    .await
            };
            match r {
                Ok(data) => success(json!({ "restored": payload, "result": data })),
                Err(e) => Err(failure(
                    if e.status == 403 {
                        "Undelete refused: an administrator token is required"
                    } else {
                        "Undelete failed"
                    },
                    Some(&e),
                    None,
                )),
            }
        }),
    }
}

/// History and attachments serve cards; the rest is raw API surface.
pub fn card_tools() -> Vec<Tool> {
    vec![read_history(), write_attachment()]
}

pub fn raw_tools() -> Vec<Tool> {
    vec![
        read_v2_query(),
        read_context(),
        read_conversions(),
        read_deleted(),
        read_storage(),
        write_storage(),
        delete_storage(),
    ]
}

pub fn admin_tools() -> Vec<Tool> {
    vec![admin_undelete()]
}
