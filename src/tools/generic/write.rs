//! Raw write, delete and admin tools, validated against the catalog. The admin
//! tools share the handlers; the tier decides which resources each may touch.

use std::sync::Arc;

use serde_json::{Map, Value, json};

use super::policy::{WriteOperation, WriteTier, resolve_writable};
use super::validate::{Mode, catalog_for_body, prepare_body, prepare_items, unpersisted};
use crate::catalog::{Catalog, CatalogResource, Request};
use crate::http::v1::{BULK_LIMIT, ReadOptions, WriteOptions};
use crate::schema::{Args, Field, Kind, field, list, nonempty};
use crate::tools::params::{fields, id, ids, resource};
use crate::tools::{Answer, Ctx, Tool, ToolResult, failed, failure, handler, invalid, success};

/// Number.isSafeInteger(n) && n > 0, after a digits-only string became a number.
fn parse_id(value: &Value) -> Option<i64> {
    let n = match value {
        Value::Number(n) => n.as_f64()?,
        Value::String(s) if !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) => {
            s.parse::<f64>().ok()?
        }
        _ => return None,
    };
    (n.fract() == 0.0 && n > 0.0 && n <= 9_007_199_254_740_991.0).then_some(n as i64)
}

async fn writable(
    ctx: &Ctx,
    name: &str,
    op: WriteOperation,
    tier: WriteTier,
) -> Result<(Arc<Catalog>, Arc<CatalogResource>), ToolResult> {
    let catalog = ctx
        .catalog
        .get(&Request::Names(vec![name.to_string()]))
        .await;
    let r = resolve_writable(&catalog, name, op, tier)
        .map_err(invalid)?
        .clone();
    Ok((catalog, r))
}

/// `{key: value, notPersisted?}` tail of a create/update answer.
fn with_unpersisted(mut out: Map<String, Value>, body: &Map<String, Value>, data: &Value) -> Value {
    let missing = unpersisted(body, data);
    if !missing.is_empty() {
        out.insert("notPersisted".into(), json!(missing));
    }
    Value::Object(out)
}

async fn create_entity(ctx: &Ctx, tier: WriteTier, args: Args) -> Answer {
    let given = args.opt_map("fields").cloned().unwrap_or_default();
    let (loaded, r) = writable(ctx, &args.str("resource"), WriteOperation::Create, tier).await?;
    let catalog = catalog_for_body(&ctx.catalog, loaded, &r, &given).await;
    let body = prepare_body(&catalog, &r, &given, Mode::Create, "").map_err(invalid)?;
    if body.is_empty() {
        return Err(invalid(format!("no fields given for the new {}", r.name)));
    }
    let data = ctx
        .v1
        .create(
            &r.path,
            Value::Object(body.clone()),
            &WriteOptions::default(),
        )
        .await
        .map_err(failed(format!("Could not create {}", r.name)))?;
    let mut out = Map::new();
    out.insert("created".into(), json!(r.name));
    if let Some(id) = data.get("Id") {
        out.insert("id".into(), id.clone());
    }
    out.insert("entity".into(), data.clone());
    success(with_unpersisted(out, &body, &data))
}

async fn update_entity(ctx: &Ctx, tier: WriteTier, args: Args) -> Answer {
    let given = args.opt_map("fields").cloned().unwrap_or_default();
    let id = args.id("id");
    let (loaded, r) = writable(ctx, &args.str("resource"), WriteOperation::Update, tier).await?;
    let catalog = catalog_for_body(&ctx.catalog, loaded, &r, &given).await;
    let body = prepare_body(&catalog, &r, &given, Mode::Update, "").map_err(invalid)?;
    if body.is_empty() {
        return Err(invalid(format!(
            "no fields given to update on {} {id}",
            r.name
        )));
    }
    let data = ctx
        .v1
        .update(&r.path, id, body.clone(), &WriteOptions::default())
        .await
        .map_err(failed(format!("Could not update {} {id}", r.name)))?;
    let mut out = Map::new();
    out.insert("updated".into(), json!(r.name));
    out.insert("id".into(), json!(id));
    out.insert("entity".into(), data.clone());
    success(with_unpersisted(out, &body, &data))
}

async fn add_to_collection(ctx: &Ctx, tier: WriteTier, args: Args) -> Answer {
    let id = args.id("id");
    let items = args.opt_list("items").cloned().unwrap_or_default();
    let (catalog, r) = writable(ctx, &args.str("resource"), WriteOperation::Add, tier).await?;
    let coll = catalog
        .collection(&r, &args.str("collection"))
        .map_err(invalid)?;
    let mut nested = Map::new();
    nested.insert(coll.name.clone(), Value::Array(items.clone()));
    let catalog = catalog_for_body(&ctx.catalog, catalog.clone(), &r, &nested).await;
    let count = items.len();
    let prepared =
        prepare_items(&catalog, &r, coll, &Value::Array(items), &coll.name).map_err(invalid)?;
    let mut body = Map::new();
    body.insert(coll.name.clone(), prepared);
    let data = ctx
        .v1
        .update(&r.path, id, body, &WriteOptions::default())
        .await
        .map_err(failed(format!(
            "Could not add to {} {id} {}",
            r.name, coll.name
        )))?;
    success(json!({
        "added": count,
        "resource": r.name,
        "id": id,
        "collection": coll.name,
        "entity": data,
    }))
}

async fn delete_entity(ctx: &Ctx, tier: WriteTier, args: Args) -> Answer {
    let id = args.id("id");
    let (_, r) = writable(ctx, &args.str("resource"), WriteOperation::Delete, tier).await?;
    let before = ctx
        .v1
        .get(&r.path, id, &ReadOptions::default())
        .await
        .map_err(failed(format!(
            "Could not read {} {id} before deleting it; nothing was deleted",
            r.name
        )))?;
    ctx.v1
        .delete(&r.path, id)
        .await
        .map_err(failed(format!("Could not delete {} {id}", r.name)))?;
    let mut out = Map::new();
    out.insert("deleted".into(), json!(r.name));
    out.insert("id".into(), json!(id));
    if let Some(Value::String(name)) = before.get("Name") {
        out.insert("name".into(), json!(name));
    }
    success(Value::Object(out))
}

async fn remove_from_collection(ctx: &Ctx, tier: WriteTier, args: Args) -> Answer {
    let id = args.id("id");
    let child_ids = args.opt_ids("childIds").unwrap_or_default();
    let (catalog, r) = writable(ctx, &args.str("resource"), WriteOperation::Remove, tier).await?;
    let coll = catalog
        .collection(&r, &args.str("collection"))
        .map_err(invalid)?;
    if !coll.can_remove {
        return Err(invalid(format!(
            "items cannot be removed from {}.{}",
            r.name, coll.name
        )));
    }
    let removal = ctx
        .v1
        .remove_from_collection(&r.path, id, &coll.name, &child_ids)
        .await;
    if let Some(e) = &removal.error {
        let mut details = Map::new();
        if !removal.removed.is_empty() {
            details.insert("alreadyRemoved".into(), json!(removal.removed));
        }
        return Err(failure(
            &format!("Could not remove from {} {id} {}", r.name, coll.name),
            Some(e),
            Some(&details),
        ));
    }
    success(json!({
        "removed": child_ids,
        "resource": r.name,
        "id": id,
        "collection": coll.name,
    }))
}

fn collection_name() -> Field {
    field("collection", nonempty())
        .describe("Collection name from read_meta, e.g. Assignments, TagObjects, Builds")
}

fn collection_items() -> Field {
    field("items", list(Kind::Record(None), Some(1), None)).describe(
        r#"Items to add, e.g. [{"GeneralUser":{"Id":1},"Role":{"Id":13}}] or [{"Id":42}] to link an existing entity"#,
    )
}

fn write_bulk() -> Tool {
    Tool {
        name: "write_bulk",
        description: "Create or update up to 500 entities of one resource (raw); items with \"Id\" update. Nothing is sent if one is invalid.",
        fields: vec![
            resource(),
            field("items", list(Kind::Record(None), Some(1), Some(BULK_LIMIT))),
        ],
        handler: handler!(|ctx, args| {
            let name = args.str("resource");
            let mut catalog = ctx.catalog.get(&Request::Names(vec![name.clone()])).await;
            let r = catalog.resource(&name).map_err(invalid)?.clone();
            let mut problems: Vec<String> = Vec::new();
            let mut payload: Vec<Value> = Vec::new();
            for (i, item) in args.opt_list("items").into_iter().flatten().enumerate() {
                let Some(item) = item.as_object() else {
                    continue;
                };
                let id_key = item.keys().find(|k| k.to_lowercase() == "id").cloned();
                let mode = if id_key.is_some() {
                    WriteOperation::Update
                } else {
                    WriteOperation::Create
                };
                let target = match resolve_writable(&catalog, &name, mode, WriteTier::Write) {
                    Ok(t) => t.clone(),
                    Err(message) => {
                        problems.push(format!("items[{i}]: {message}"));
                        continue;
                    }
                };
                let item_id = match &id_key {
                    Some(k) => match parse_id(&item[k]) {
                        Some(n) => Some(n),
                        None => {
                            problems.push(format!("items[{i}].Id: must be a numeric id"));
                            continue;
                        }
                    },
                    None => None,
                };
                let rest: Map<String, Value> = item
                    .iter()
                    .filter(|(k, _)| Some(*k) != id_key.as_ref())
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect();
                catalog = catalog_for_body(&ctx.catalog, catalog, &target, &rest).await;
                let body_mode = if item_id.is_some() {
                    Mode::Update
                } else {
                    Mode::Create
                };
                match prepare_body(&catalog, &target, &rest, body_mode, &format!("items[{i}]")) {
                    Err(message) => problems.push(message),
                    Ok(body) => match item_id {
                        None => payload.push(Value::Object(body)),
                        Some(n) => {
                            let mut with_id = Map::new();
                            with_id.insert("Id".into(), json!(n));
                            with_id.extend(body);
                            payload.push(Value::Object(with_id));
                        }
                    },
                }
            }
            if !problems.is_empty() {
                return Err(invalid(format!(
                    "bulk payload rejected:\n- {}",
                    problems.join("\n- ")
                )));
            }
            let sent = payload.len();
            let data = ctx
                .v1
                .bulk(&r.path, payload, &WriteOptions::default())
                .await
                .map_err(failed(format!("Bulk write to {} failed", r.path)))?;
            success(json!({ "resource": r.name, "sent": sent, "result": data }))
        }),
    }
}

fn delete_bulk() -> Tool {
    Tool {
        name: "delete_bulk",
        description: "Delete up to 500 entities of one resource by id (raw).",
        fields: vec![resource(), ids("ids", BULK_LIMIT)],
        handler: handler!(|ctx, args| {
            let ids = args.opt_ids("ids").unwrap_or_default();
            let (_, r) = writable(
                ctx,
                &args.str("resource"),
                WriteOperation::Delete,
                WriteTier::Delete,
            )
            .await?;
            let data = ctx
                .v1
                .delete_bulk(&r.path, &ids)
                .await
                .map_err(failed(format!("Bulk delete on {} failed", r.path)))?;
            success(json!({ "deleted": r.name, "ids": ids, "result": data }))
        }),
    }
}

pub fn write_tools() -> Vec<Tool> {
    vec![
        Tool {
            name: "write_create",
            description: "Create an entity of any resource (raw), validated against the catalog. Prefer write_create_card for cards.",
            fields: vec![
                resource(),
                fields().describe(
                    r#"Targetprocess JSON without Id, e.g. {"Name":"x","Project":{"Id":2}}; children nest as {"Tasks":{"Items":[...]}}; custom fields as "CustomFields":[{"Name","Value"}]"#,
                ),
            ],
            handler: handler!(|ctx, args| { create_entity(ctx, WriteTier::Write, args).await }),
        },
        Tool {
            name: "write_update",
            description: "Update fields of any entity (raw), validated and read back. Card effort: use write_set_role_effort.",
            fields: vec![resource(), id("id"), fields()],
            handler: handler!(|ctx, args| { update_entity(ctx, WriteTier::Write, args).await }),
        },
        write_bulk(),
        Tool {
            name: "write_collection_add",
            description: "Append items to a collection of any entity (raw); nothing is replaced.",
            fields: vec![resource(), id("id"), collection_name(), collection_items()],
            handler: handler!(|ctx, args| { add_to_collection(ctx, WriteTier::Write, args).await }),
        },
    ]
}

pub fn delete_tools() -> Vec<Tool> {
    vec![
        Tool {
            name: "delete_entity",
            description: "Delete one entity of any resource (raw); reports what was deleted.",
            fields: vec![resource(), id("id")],
            handler: handler!(|ctx, args| { delete_entity(ctx, WriteTier::Delete, args).await }),
        },
        delete_bulk(),
        Tool {
            name: "delete_collection_remove",
            description: "Remove items from a collection of any entity by child id (raw), e.g. unlink test cases from a test plan.",
            fields: vec![
                resource(),
                id("id"),
                collection_name(),
                ids("childIds", BULK_LIMIT),
            ],
            handler: handler!(|ctx, args| {
                remove_from_collection(ctx, WriteTier::Delete, args).await
            }),
        },
    ]
}

macro_rules! admin_note {
    () => {
        " (Project, Team, User, Process, Workflow, Role, CustomField, ...); needs an administrator token."
    };
}

pub fn admin_tools() -> Vec<Tool> {
    vec![
        Tool {
            name: "admin_create",
            description: concat!("Create an administration entity", admin_note!()),
            fields: vec![resource(), fields()],
            handler: handler!(|ctx, args| { create_entity(ctx, WriteTier::Admin, args).await }),
        },
        Tool {
            name: "admin_update",
            description: concat!("Update an administration entity", admin_note!()),
            fields: vec![resource(), id("id"), fields()],
            handler: handler!(|ctx, args| { update_entity(ctx, WriteTier::Admin, args).await }),
        },
        Tool {
            name: "admin_delete",
            description: concat!("Delete an administration entity", admin_note!()),
            fields: vec![resource(), id("id")],
            handler: handler!(|ctx, args| { delete_entity(ctx, WriteTier::Admin, args).await }),
        },
        Tool {
            name: "admin_collection_add",
            description: concat!(
                "Add items to a collection of an administration entity, e.g. Team TeamMembers",
                admin_note!()
            ),
            fields: vec![resource(), id("id"), collection_name(), collection_items()],
            handler: handler!(|ctx, args| { add_to_collection(ctx, WriteTier::Admin, args).await }),
        },
        Tool {
            name: "admin_collection_remove",
            description: concat!(
                "Remove items from a collection of an administration entity",
                admin_note!()
            ),
            fields: vec![
                resource(),
                id("id"),
                collection_name(),
                ids("childIds", BULK_LIMIT),
            ],
            handler: handler!(|ctx, args| {
                remove_from_collection(ctx, WriteTier::Admin, args).await
            }),
        },
    ]
}
