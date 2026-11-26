//! Workflow delete tools: a card (reporting its parent's side effects), a relation.

use serde_json::{Map, Value, json};

use crate::catalog::Request;
use crate::domain::cards::{read_card_raw, shape_card};
use crate::domain::parent::{parent_change, parent_snapshot, snapshot_again};
use crate::format::shape::{num, reference};
use crate::http::v1::{ListOptions, ReadOptions};
use crate::schema::{Kind, field, string};
use crate::tools::generic::policy::{WriteOperation, WriteTier, resolve_writable};
use crate::tools::generic::validate::js_string;
use crate::tools::params::id;
use crate::tools::{Tool, failed, handler, invalid, success};

/// Counts (from read_card) that are child cards of each type. The same
/// collection means different things per type: a feature's UserStories are its
/// children, a test case's UserStories are only the stories it is linked to
/// (live: a test case "had 1 userStories").
fn child_counts(entity_type: &str) -> &'static [&'static str] {
    match entity_type {
        "PortfolioEpic" => &["epics", "features"],
        "Epic" => &["features"],
        "Feature" => &["userStories", "bugs"],
        "UserStory" => &["tasks", "bugs"],
        "TestPlan" => &["testCases"],
        _ => &[],
    }
}

fn delete_card() -> Tool {
    Tool {
        name: "delete_card",
        description: "Delete a card by id; reports what was deleted and the parent card's state and efforts before/after.",
        fields: vec![
            id("id"),
            field("withChildren", Kind::Bool).optional().describe(
                "Required for a card with child cards (a story's tasks and bugs, a feature's stories, ...): they may be deleted too",
            ),
        ],
        handler: handler!(|ctx, args| {
            let card = read_card_raw(ctx, args.id("id"))
                .await
                .map_err(|p| p.to_result())?;
            let (info, raw) = (&card.info, &card.raw);
            let catalog = ctx
                .catalog
                .get(&Request::Names(vec![info.resource.name.clone()]))
                .await;
            resolve_writable(
                &catalog,
                &info.resource.name,
                WriteOperation::Delete,
                WriteTier::Delete,
            )
            .map_err(invalid)?;
            let shaped = shape_card(info, raw, None);
            let kinds = child_counts(&info.entity_type);
            let children: Map<String, Value> = shaped["counts"]
                .as_object()
                .into_iter()
                .flatten()
                .filter(|(k, n)| {
                    kinds.contains(&k.as_str()) && n.as_f64().is_some_and(|n| n > 0.0)
                })
                .map(|(k, n)| (k.clone(), n.clone()))
                .collect();
            if !children.is_empty() && !args.bool("withChildren") {
                let listed: Vec<String> = children
                    .iter()
                    .map(|(k, n)| format!("{} {k}", js_string(n)))
                    .collect();
                return Err(invalid(format!(
                    "{} {} has child cards ({}); pass withChildren: true to delete it anyway",
                    info.entity_type,
                    info.id,
                    listed.join(", ")
                )));
            }
            let parent_before = parent_snapshot(ctx, raw).await;

            ctx.v1
                .delete(&info.resource.path, info.id)
                .await
                .map_err(failed(format!(
                    "Could not delete {} {}",
                    info.entity_type, info.id
                )))?;
            let gone = ctx
                .v1
                .get(
                    &info.resource.path,
                    info.id,
                    &ReadOptions {
                        include: Some("[Id]".into()),
                        ..ReadOptions::default()
                    },
                )
                .await;
            let parent_after = snapshot_again(ctx, parent_before.as_ref()).await;

            let mut deleted = Map::new();
            deleted.insert("id".into(), json!(info.id));
            deleted.insert("type".into(), json!(info.entity_type));
            deleted.insert(
                "name".into(),
                shaped
                    .get("name")
                    .cloned()
                    .unwrap_or_else(|| json!(info.name)),
            );
            if let Some(state) = shaped["state"].get("name") {
                deleted.insert("state".into(), state.clone());
            }
            if let Some(project) = shaped.get("project") {
                deleted.insert("project".into(), project.clone());
            }
            let mut out = Map::new();
            out.insert("deleted".into(), Value::Object(deleted));
            if !children.is_empty() {
                out.insert("hadChildren".into(), Value::Object(children));
            }
            if parent_before.is_some()
                && let Some(change) = parent_change(parent_before.as_ref(), parent_after.as_ref())
            {
                out.insert("parent".into(), change.to_json());
            }
            if gone.is_ok() {
                out.insert(
                    "notPersisted".into(),
                    json!(["the card can still be read after deleting it"]),
                );
            }
            success(Value::Object(out))
        }),
    }
}

fn delete_relation() -> Tool {
    Tool {
        name: "delete_relation",
        description: "Delete the relation between two cards, in either direction.",
        fields: vec![
            id("id"),
            id("to"),
            field("relation", string())
                .optional()
                .describe("Relation type name, when the cards have several"),
        ],
        handler: handler!(|ctx, args| {
            let (from, to) = (args.id("id"), args.id("to"));
            let relation = args.opt_str("relation").filter(|r| !r.is_empty());
            let options = |where_: String| ListOptions {
                read: ReadOptions {
                    include: Some(
                        "[Id,RelationType[Id,Name],Master[Id,Name],Slave[Id,Name]]".into(),
                    ),
                    ..ReadOptions::default()
                },
                where_: Some(where_),
                ..ListOptions::default()
            };
            let (a_options, b_options) = (
                options(format!("(Master.Id eq {from}) and (Slave.Id eq {to})")),
                options(format!("(Master.Id eq {to}) and (Slave.Id eq {from})")),
            );
            let (a, b) = tokio::join!(
                ctx.v1.list("Relations", &a_options),
                ctx.v1.list("Relations", &b_options),
            );
            let a = a.map_err(failed("Could not read relations"))?;
            let b = b.map_err(failed("Could not read relations"))?;
            let type_name = |r: &Value| reference(&r["RelationType"])["name"].clone();
            let mut found: Vec<Value> = a.items.into_iter().chain(b.items).collect();
            if let Some(wanted) = &relation {
                let wanted = crate::catalog::js_trim(wanted).to_lowercase();
                found.retain(|r| {
                    type_name(r).as_str().map(str::to_lowercase) == Some(wanted.clone())
                });
            }
            if found.is_empty() {
                return Err(invalid(format!(
                    "no{} relation between {from} and {to}",
                    relation
                        .as_deref()
                        .map(|r| format!(" {r}"))
                        .unwrap_or_default()
                )));
            }
            if found.len() > 1 {
                let names: Vec<String> = found
                    .iter()
                    .map(|r| type_name(r).as_str().unwrap_or("").to_string())
                    .collect();
                return Err(invalid(format!(
                    "{} relations between {from} and {to} ({}); pass relation",
                    found.len(),
                    names.join(", ")
                )));
            }
            let target = &found[0];
            let target_id = target["Id"].as_f64().unwrap_or(f64::NAN);
            ctx.v1
                .delete("Relations", target_id as i64)
                .await
                .map_err(failed(format!(
                    "Could not delete relation {}",
                    js_string(&target["Id"])
                )))?;
            let mut deleted = Map::new();
            for (key, value) in [
                ("id", num(&target["Id"])),
                ("type", type_name(target)),
                ("master", reference(&target["Master"])),
                ("slave", reference(&target["Slave"])),
            ] {
                if !value.is_null() {
                    deleted.insert(key.into(), value);
                }
            }
            success(json!({ "deleted": deleted }))
        }),
    }
}

pub fn tools() -> Vec<Tool> {
    vec![delete_card(), delete_relation()]
}
