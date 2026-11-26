//! Workflow write tools: create and update cards, state, assignments, role
//! efforts, custom fields, teams, comments, time, relations, follow, testing.
//! Every write is sequential per card and read back (domain rules 7 and 8).

use std::time::Duration;

use serde_json::{Map, Value, json};

use super::common::unresolved;
use crate::catalog::{Catalog, Request, js_trim};
use crate::domain::assignments::{
    AssignmentRow, add_assignment, describe_assignment, list_assignments, remove_assignment,
};
use crate::domain::cards::{CardRead, read_card_as, read_card_raw, shape_card};
use crate::domain::create::{is_card_resource, parent_link};
use crate::domain::custom_fields::{
    CustomFieldValue, prepare_custom_fields, unpersisted_custom_fields,
};
use crate::domain::efforts::{EffortRequest, apply_role_efforts, role_effort_rows};
use crate::domain::parent::{
    ParentRead, ParentRef, parent_change, parent_snapshot, snapshot_again, snapshot_of,
};
use crate::domain::time::{TimeBackend, TimeRecordBackend, time_backend};
use crate::domain::{num, v1_ref};
use crate::format::dates::same_date_value;
use crate::format::html::{RichTextFormat, html_to_text, text_to_html};
use crate::format::shape::{items, number, tags as parse_tags};
use crate::http::v1::{ListOptions, ReadOptions, WriteOptions};
use crate::http::{Res, TpErr};
use crate::resolve::card::{CardInfo, ProjectRef, card_info};
use crate::resolve::directory::{TpRole, TpState, TpUser, V1Ref, full_name};
use crate::resolve::matching::{MatchSpec, Resolved, r#match};
use crate::schema::{Args, Field, Kind, field, list, nonempty, pattern, string};
use crate::tools::generic::policy::is_admin_resource;
use crate::tools::generic::validate::{
    Mode, catalog_for_body, js_json, js_number, js_string, prepare_body, unpersisted,
};
use crate::tools::params::{id, name_or_id};
use crate::tools::{Answer, Ctx, Tool, ToolResult, failed, failure, handler, invalid, success};

// ---------------------------------------------------------------- answers

/// A JSON object built in order. `opt` leaves `None` out, as `JSON.stringify` drops `undefined`.
#[derive(Default)]
struct Obj(Map<String, Value>);

impl Obj {
    fn new() -> Self {
        Obj::default()
    }

    fn set(mut self, key: &str, value: impl Into<Value>) -> Self {
        self.0.insert(key.into(), value.into());
        self
    }

    fn opt(self, key: &str, value: Option<impl Into<Value>>) -> Self {
        match value {
            Some(v) => self.set(key, v),
            None => self,
        }
    }

    /// The list under `key`, left out when empty.
    fn listed(self, key: &str, list: Vec<String>) -> Self {
        if list.is_empty() {
            self
        } else {
            self.set(key, list)
        }
    }

    fn value(self) -> Value {
        Value::Object(self.0)
    }
}

/// `ref(...)` as answers show it: `{id, name}`, the name only when known.
fn ref_json(r: &V1Ref) -> Value {
    match &r.name {
        Some(name) => json!({ "id": r.id, "name": name }),
        None => json!({ "id": r.id }),
    }
}

/// `ref(value)?.id`.
fn ref_id(value: &Value) -> Option<i64> {
    v1_ref(value).map(|r| r.id)
}

/// `ref(value)?.name`.
fn ref_name(value: &Value) -> Option<String> {
    v1_ref(value).and_then(|r| r.name)
}

/// An optional name as a template literal prints it.
fn shown(name: Option<&str>) -> &str {
    name.unwrap_or("undefined")
}

/// An optional id as a template literal prints `x ?? null`.
fn id_or_null(id: Option<i64>) -> String {
    id.map_or_else(|| "null".to_string(), |n| n.to_string())
}

/// JavaScript's default sort: UTF-16 code unit order.
fn js_sort(list: &mut [String]) {
    list.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
}

// ---------------------------------------------------------------- shared steps

async fn load(ctx: &Ctx, card_id: i64) -> Result<CardRead, ToolResult> {
    read_card_raw(ctx, card_id).await.map_err(|p| p.to_result())
}

async fn reload(ctx: &Ctx, card: &CardRead) -> Res<Value> {
    read_card_as(ctx, &card.info).await
}

/// The write was sent but could not be read back: say so instead of guessing a verdict.
fn unverified(e: &TpErr) -> String {
    format!(
        "The write was sent but the card could not be read back to verify it ({})",
        e.message
    )
}

async fn resolve_person(ctx: &Ctx, input: &str, allow_inactive: bool) -> Resolved<TpUser> {
    if js_trim(input).to_lowercase() == "me" {
        ctx.directory.me().await
    } else {
        ctx.directory.user(input, allow_inactive).await
    }
}

/// id, type and name of a card; `after` (a read-back) gives the name after a rename.
fn card_label(card: &CardRead, after: Option<&Value>) -> Value {
    let name = after
        .and_then(|a| a.get("Name"))
        .and_then(Value::as_str)
        .unwrap_or(&card.info.name);
    json!({ "id": card.info.id, "type": card.info.entity_type, "name": name })
}

/// A write failed after earlier writes succeeded: say exactly what was done.
fn partial(summary: &str, error: &TpErr, done: Vec<Value>) -> ToolResult {
    let mut extra = Map::new();
    if !done.is_empty() {
        extra.insert("alreadyDone".into(), Value::Array(done));
    }
    failure(summary, Some(error), Some(&extra))
}

/// `"Name (id)"`.
fn who(user: &TpUser) -> String {
    format!("{} ({})", full_name(user), user.id)
}

async fn any_catalog(ctx: &Ctx) -> std::sync::Arc<Catalog> {
    ctx.catalog.get(&Request::Names(Vec::new())).await
}

fn rich_format(args: &Args) -> Option<RichTextFormat> {
    match args.opt_str("format").as_deref() {
        Some("markdown") => Some(RichTextFormat::Markdown),
        Some("html") => Some(RichTextFormat::Html),
        Some("text") => Some(RichTextFormat::Text),
        _ => None,
    }
}

/// `parentChange(before, after)` as an answer value; `None` when the card has no parent.
fn parent_json(before: Option<&ParentRead>, after: Option<&ParentRead>) -> Option<Value> {
    parent_change(before, after).map(|c| c.to_json())
}

// ---------------------------------------------------------------- parameters

fn person(name: &'static str) -> Field {
    name_or_id(name).describe(r#"Person name, login, email, id, or "me""#)
}

fn role(name: &'static str) -> Field {
    name_or_id(name).describe(r#"Role name or id, e.g. "Developer", "Product Owner""#)
}

fn efforts_field(name: &'static str) -> Field {
    let effort = field(
        "effort",
        Kind::Num {
            int: false,
            positive: false,
            min: Some(0.0),
            max: Some(100_000.0),
        },
    )
    .describe("Hours (or points) for this role");
    field(
        name,
        list(Kind::Object(vec![role("role"), effort]), Some(1), None),
    )
    .describe(r#"Effort per role, e.g. [{"role":"Developer","effort":8}]"#)
}

fn rich_format_field() -> Field {
    field("format", Kind::OneOf(&["markdown", "html", "text"]))
        .optional()
        .describe("Default: text, or html when it is HTML; Markdown needs markdown")
}

fn tag() -> Kind {
    Kind::Str {
        trim: true,
        min: Some(1),
        pattern: None,
        refine: Some((
            |t: &str| !t.contains(','),
            "a tag cannot contain a comma (Targetprocess separates tags with commas)",
        )),
    }
}

fn ref_or_null(name: &'static str) -> Field {
    field(name, Kind::IdOrNull).optional()
}

fn text_param(name: &'static str) -> Field {
    field(name, string()).optional()
}

fn flag(name: &'static str) -> Field {
    field(name, Kind::Bool).optional()
}

// ---------------------------------------------------------------- state

fn write_set_state() -> Tool {
    Tool {
        name: "write_set_state",
        description: "Move a card, or one team's state on it, to a state by name or id; reports the parent card's state change.",
        fields: vec![
            id("id"),
            name_or_id("state").describe(r#"State name or id, e.g. "In Progress", "Coded""#),
            name_or_id("team")
                .optional()
                .describe("Team whose team state to set"),
        ],
        handler: handler!(|ctx, args| { set_state(ctx, args).await }),
    }
}

async fn set_state(ctx: &Ctx, args: Args) -> Answer {
    let card = load(ctx, args.id("id")).await?;
    let info = &card.info;
    let raw = &card.raw;
    let (Some(process_id), Some(project)) = (info.process_id, &info.project) else {
        return Err(invalid(format!(
            "{} {} has no project workflow",
            info.entity_type, info.id
        )));
    };
    let parent_before = parent_snapshot(ctx, raw).await;
    let state_input = args.str("state");

    if let Some(team_input) = args.opt_str("team") {
        // A team already on the card may since have been deactivated; its state can still be set.
        let team = ctx
            .directory
            .team(&team_input, true)
            .await
            .map_err(|r| unresolved(&r))?;
        let assigned = items(&raw["AssignedTeams"]);
        let Some(assignment) = assigned
            .iter()
            .find(|t| ref_id(&t["Team"]) == Some(team.id))
        else {
            let teams: Vec<String> = assigned
                .iter()
                .filter_map(|t| ref_name(&t["Team"]))
                .filter(|n| !n.is_empty())
                .collect();
            let teams = if teams.is_empty() {
                "none".to_string()
            } else {
                teams.join(", ")
            };
            return Err(invalid(format!(
                "{} is not assigned to {} {}; assigned teams: {teams}",
                team.name, info.entity_type, info.id
            )));
        };
        let states = ctx
            .directory
            .team_states(process_id, &info.entity_type, team.id, project.id)
            .await
            .map_err(failed(format!("Could not load {} states", team.name)))?;
        let target = {
            let kind = format!("{} {} state", team.name, info.entity_type);
            let spec = MatchSpec {
                kind: &kind,
                id: &|s: &TpState| s.id,
                name: &|s: &TpState| s.name.clone(),
                keys: &|s: &TpState| vec![Some(s.name.clone())],
                describe: None,
            };
            let candidates: Vec<&TpState> = states.iter().collect();
            r#match(&state_input, &spec, &candidates)
                .map_err(|r| unresolved(&r))?
                .clone()
        };
        let before = v1_ref(&assignment["EntityState"]);
        let ta_id = assignment["Id"].as_i64().unwrap_or_default();
        if before.as_ref().map(|b| b.id) != Some(target.id) {
            let mut body = Map::new();
            body.insert("EntityState".into(), json!({ "Id": target.id }));
            ctx.v1
                .update("TeamAssignments", ta_id, body, &WriteOptions::default())
                .await
                .map_err(failed(format!(
                    "Could not set {} state of {} {} to {}",
                    team.name, info.entity_type, info.id, target.name
                )))?;
        }
        let after = reload(ctx, &card).await;
        let parent_after = snapshot_again(ctx, parent_before.as_ref()).await;
        let team_json = json!({ "id": team.id, "name": team.name });
        let before_name = before.as_ref().and_then(|b| b.name.clone());
        let parent = parent_json(parent_before.as_ref(), parent_after.as_ref());
        let after = match after {
            Ok(after) => after,
            Err(e) => {
                return success(
                    Obj::new()
                        .set("card", card_label(&card, None))
                        .set("team", team_json)
                        .set(
                            "teamState",
                            Obj::new()
                                .opt("before", before_name)
                                .set("requested", target.name.as_str())
                                .value(),
                        )
                        .opt("parent", parent)
                        .set("warning", unverified(&e))
                        .value(),
                );
            }
        };
        let got = items(&after["AssignedTeams"])
            .into_iter()
            .find(|t| t["Id"].as_f64() == Some(ta_id as f64))
            .and_then(|t| v1_ref(&t["EntityState"]));
        let got_id = got.as_ref().map(|g| g.id);
        let got_name = got.as_ref().and_then(|g| g.name.clone());
        let not_persisted = if got_id != Some(target.id) {
            vec![format!(
                "team state: requested {}, got {}",
                target.name,
                got_name.as_deref().unwrap_or("no team assignment")
            )]
        } else {
            Vec::new()
        };
        return success(
            Obj::new()
                .set("card", card_label(&card, None))
                .set("team", team_json)
                .set(
                    "teamState",
                    Obj::new()
                        .opt("before", before_name)
                        .set("requested", target.name.as_str())
                        .opt("after", got_name)
                        .set("changed", before.as_ref().map(|b| b.id) != got_id)
                        .value(),
                )
                .opt("cardState", ref_name(&after["EntityState"]))
                .opt("parent", parent)
                .listed("notPersisted", not_persisted)
                .value(),
        );
    }

    let target = ctx
        .directory
        .state(process_id, &info.entity_type, &state_input, false)
        .await
        .map_err(|r| unresolved(&r))?;
    let before = v1_ref(&raw["EntityState"]);
    let before_name = before.as_ref().and_then(|b| b.name.clone());
    if before.as_ref().map(|b| b.id) == Some(target.id) {
        return success(
            Obj::new()
                .set("card", card_label(&card, None))
                .set(
                    "state",
                    Obj::new()
                        .opt("before", before_name.clone())
                        .set("requested", target.name.as_str())
                        .opt("after", before_name)
                        .set("changed", false)
                        .value(),
                )
                .value(),
        );
    }
    let mut body = Map::new();
    body.insert("EntityState".into(), json!({ "Id": target.id }));
    ctx.v1
        .update(&info.resource.path, info.id, body, &WriteOptions::default())
        .await
        .map_err(failed(format!(
            "Could not move {} {} to {}",
            info.entity_type, info.id, target.name
        )))?;
    let after = reload(ctx, &card).await;
    let parent_after = snapshot_again(ctx, parent_before.as_ref()).await;
    let parent = parent_json(parent_before.as_ref(), parent_after.as_ref());
    let after = match after {
        Ok(after) => after,
        Err(e) => {
            return success(
                Obj::new()
                    .set("card", card_label(&card, None))
                    .set(
                        "state",
                        Obj::new()
                            .opt("before", before_name)
                            .set("requested", target.name.as_str())
                            .value(),
                    )
                    .opt("parent", parent)
                    .set("warning", unverified(&e))
                    .value(),
            );
        }
    };
    let got = v1_ref(&after["EntityState"]);
    let got_id = got.as_ref().map(|g| g.id);
    let got_name = got.as_ref().and_then(|g| g.name.clone());
    let team_states: Vec<Value> = items(&after["AssignedTeams"])
        .iter()
        .map(|t| {
            Obj::new()
                .opt("team", ref_name(&t["Team"]))
                .opt("state", ref_name(&t["EntityState"]))
                .value()
        })
        .collect();
    let not_persisted = if got_id != Some(target.id) {
        vec![format!(
            "state: requested {}, got {}",
            target.name,
            got_name.as_deref().unwrap_or("no state")
        )]
    } else {
        Vec::new()
    };
    success(
        Obj::new()
            .set("card", card_label(&card, None))
            .set(
                "state",
                Obj::new()
                    .opt("before", before_name)
                    .set("requested", target.name.as_str())
                    .opt("after", got_name)
                    .set("changed", before.as_ref().map(|b| b.id) != got_id)
                    .value(),
            )
            .set("teamStates", team_states)
            .opt("parent", parent)
            .listed("notPersisted", not_persisted)
            .value(),
    )
}

// ---------------------------------------------------------------- assignments

fn write_assign() -> Tool {
    Tool {
        name: "write_assign",
        description: "Assign a person to a card in a role, adding to existing assignments; reads the assignments back.",
        fields: vec![
            id("id"),
            person("user"),
            role("role"),
            flag("exclusive")
                .describe("Also remove everyone else in this role; only when explicitly asked"),
        ],
        handler: handler!(|ctx, args| { assign(ctx, args).await }),
    }
}

/// `${a.user.name} as ${a.role.name}`.
fn user_as_role(a: &AssignmentRow) -> String {
    format!("{} as {}", a.user.name, shown(a.role.name.as_deref()))
}

async fn assign(ctx: &Ctx, args: Args) -> Answer {
    let card_id = args.id("id");
    let info = card_info(ctx, card_id).await.map_err(|r| unresolved(&r))?;
    if any_catalog(ctx)
        .await
        .member(&info.resource, "Assignments")
        .is_none()
    {
        return Err(invalid(format!(
            "A {} cannot have assignments",
            info.entity_type
        )));
    }
    let user = resolve_person(ctx, &args.str("user"), false)
        .await
        .map_err(|r| unresolved(&r))?;
    let r = ctx
        .directory
        .role(&args.str("role"))
        .await
        .map_err(|r| unresolved(&r))?;
    let current = list_assignments(ctx, card_id)
        .await
        .map_err(failed(format!("Could not read assignments of {card_id}")))?;

    // Add first, so a failure never leaves the role empty.
    let mut done: Vec<Value> = Vec::new();
    let holds = |a: &AssignmentRow| a.user.id == user.id && a.role.id == r.id;
    let already = current.iter().any(holds);
    if !already {
        add_assignment(ctx, card_id, user.id, r.id)
            .await
            .map_err(failed(format!(
                "Could not assign {} as {} on {card_id}; nobody was removed",
                full_name(&user),
                r.name
            )))?;
        done.push(json!({ "assigned": format!("{} as {}", full_name(&user), r.name) }));
    }
    let mut removed: Vec<&AssignmentRow> = Vec::new();
    if args.bool("exclusive") {
        for a in current
            .iter()
            .filter(|a| a.role.id == r.id && a.user.id != user.id)
        {
            if let Err(e) = remove_assignment(ctx, a.id).await {
                return Err(partial(
                    &format!("Could not remove {}", user_as_role(a)),
                    &e,
                    done,
                ));
            }
            removed.push(a);
            done.push(json!({ "removed": describe_assignment(a) }));
        }
    }
    let after = list_assignments(ctx, card_id).await;
    let mut out = Obj::new()
        .set(
            "card",
            json!({ "id": card_id, "type": info.entity_type, "name": info.name }),
        )
        .set("assigned", json!({ "user": who(&user), "role": r.name }))
        .set("alreadyAssigned", already)
        .set(
            "removed",
            removed
                .iter()
                .map(|a| describe_assignment(a))
                .collect::<Vec<_>>(),
        );
    match &after {
        Ok(rows) => {
            let present = rows.iter().any(holds);
            let leftover: Vec<String> = removed
                .iter()
                .filter(|x| rows.iter().any(|a| a.id == x.id))
                .map(|x| format!("{} is still on the card after removing", user_as_role(x)))
                .collect();
            out = out.set(
                "assignments",
                rows.iter().map(describe_assignment).collect::<Vec<_>>(),
            );
            if !present || !leftover.is_empty() {
                let mut not_persisted = Vec::new();
                if !present {
                    not_persisted
                        .push("the assignment is not on the card after writing".to_string());
                }
                not_persisted.extend(leftover);
                out = out.set("notPersisted", not_persisted);
            }
        }
        Err(e) => {
            out = out.set(
                "warning",
                format!("Could not read assignments back: {}", e.message),
            );
        }
    }
    success(out.value())
}

fn write_unassign() -> Tool {
    Tool {
        name: "write_unassign",
        description: "Remove one person's assignment from a card; role is required when they hold several.",
        fields: vec![id("id"), person("user"), role("role").optional()],
        handler: handler!(|ctx, args| { unassign(ctx, args).await }),
    }
}

async fn unassign(ctx: &Ctx, args: Args) -> Answer {
    let card_id = args.id("id");
    let user = resolve_person(ctx, &args.str("user"), true)
        .await
        .map_err(|r| unresolved(&r))?;
    let role_id = match args.opt_str("role") {
        Some(input) => Some(
            ctx.directory
                .role(&input)
                .await
                .map_err(|r| unresolved(&r))?
                .id,
        ),
        None => None,
    };
    let current = list_assignments(ctx, card_id)
        .await
        .map_err(failed(format!("Could not read assignments of {card_id}")))?;
    let hits: Vec<&AssignmentRow> = current
        .iter()
        .filter(|a| a.user.id == user.id && role_id.is_none_or(|r| a.role.id == r))
        .collect();
    let target = match hits.as_slice() {
        [] => {
            let listed: Vec<String> = current.iter().map(user_as_role).collect();
            let listed = if listed.is_empty() {
                "none".to_string()
            } else {
                listed.join(", ")
            };
            return Err(invalid(format!(
                "{} holds no such assignment on {card_id}; assignments: {listed}",
                full_name(&user)
            )));
        }
        [one] => *one,
        several => {
            // Array.join prints a missing name as empty.
            let roles: Vec<&str> = several
                .iter()
                .map(|a| a.role.name.as_deref().unwrap_or(""))
                .collect();
            return Err(invalid(format!(
                "{} holds {} roles on {card_id} ({}); pass the role",
                full_name(&user),
                several.len(),
                roles.join(", ")
            )));
        }
    };
    remove_assignment(ctx, target.id)
        .await
        .map_err(failed(format!(
            "Could not remove {} from {card_id}",
            user_as_role(target)
        )))?;
    let after = list_assignments(ctx, card_id).await;
    let mut out = Obj::new()
        .set("card", card_id)
        .set("removed", describe_assignment(target));
    match &after {
        Ok(rows) => {
            out = out.set(
                "assignments",
                rows.iter().map(describe_assignment).collect::<Vec<_>>(),
            );
            if rows.iter().any(|a| a.id == target.id) {
                out = out.set("notPersisted", vec!["the assignment is still on the card"]);
            }
        }
        Err(e) => {
            out = out.set(
                "warning",
                format!("Could not read assignments back: {}", e.message),
            );
        }
    }
    success(out.value())
}

// ---------------------------------------------------------------- efforts

/// Live: card and parent effort totals are recomputed a few seconds after a role-effort write.
const TOTALS_NOTE: &str = "Targetprocess recomputes effort totals a few seconds after a write; totals read right after it may still change";

async fn resolve_efforts(ctx: &Ctx, list: &[Value]) -> Result<Vec<EffortRequest>, ToolResult> {
    let mut out: Vec<EffortRequest> = Vec::new();
    for e in list {
        let input = e["role"].as_str().unwrap_or_default();
        let r: TpRole = ctx
            .directory
            .role(input)
            .await
            .map_err(|r| unresolved(&r))?;
        if r.has_effort == Some(false) {
            return Err(invalid(format!("Role {} does not carry effort", r.name)));
        }
        if out.iter().any(|x| x.role_id == r.id) {
            return Err(invalid(format!("Role {} is listed twice", r.name)));
        }
        out.push(EffortRequest {
            role_id: r.id,
            role_name: r.name,
            effort: e["effort"].as_f64().unwrap_or_default(),
        });
    }
    Ok(out)
}

/// `${role}: requested ${effort}, got ${got ?? 'no row'}` for every request the rows do not show.
fn unpersisted_efforts(requests: &[EffortRequest], raw: &Value, label: &str) -> Vec<String> {
    let rows = role_effort_rows(raw);
    requests
        .iter()
        .filter_map(|req| {
            let got = rows
                .iter()
                .find(|r| r.role.id == req.role_id)
                .map(|r| r.effort);
            (got != Some(req.effort)).then(|| {
                format!(
                    "{}{label}: requested {}, got {}",
                    req.role_name,
                    js_number(req.effort),
                    got.map_or_else(|| "no row".to_string(), js_number)
                )
            })
        })
        .collect()
}

fn write_set_role_effort() -> Tool {
    Tool {
        name: "write_set_role_effort",
        description: "Set effort per role on a card (never the card total). A task's efforts roll up into its story; both are reported.",
        fields: vec![id("id"), efforts_field("efforts")],
        handler: handler!(|ctx, args| { set_role_effort(ctx, args).await }),
    }
}

async fn set_role_effort(ctx: &Ctx, args: Args) -> Answer {
    let card = load(ctx, args.id("id")).await?;
    if any_catalog(ctx)
        .await
        .member(&card.info.resource, "RoleEfforts")
        .is_none()
    {
        return Err(invalid(format!(
            "A {} has no role efforts",
            card.info.entity_type
        )));
    }
    let requests =
        resolve_efforts(ctx, args.opt_list("efforts").map_or(&[], Vec::as_slice)).await?;
    let parent_before = parent_snapshot(ctx, &card.raw).await;
    let outcome =
        apply_role_efforts(ctx, card.info.id, &role_effort_rows(&card.raw), &requests).await;
    let applied: Vec<Value> = outcome.applied.iter().map(|e| e.to_json()).collect();
    if let Some(error) = &outcome.error {
        return Err(partial(&error.message, &error.error, applied));
    }
    let after = reload(ctx, &card).await;
    let parent_after = snapshot_again(ctx, parent_before.as_ref()).await;
    let parent = parent_json(parent_before.as_ref(), parent_after.as_ref());
    let after = match after {
        Ok(after) => after,
        Err(e) => {
            return success(
                Obj::new()
                    .set("card", card_label(&card, None))
                    .set("efforts", applied)
                    .opt("parent", parent)
                    .set("warning", unverified(&e))
                    .value(),
            );
        }
    };
    let not_persisted = unpersisted_efforts(&requests, &after, "");
    success(
        Obj::new()
            .set("card", card_label(&card, None))
            .set("efforts", applied)
            .set(
                "total",
                Obj::new()
                    .opt("before", num(&card.raw["Effort"]).map(number))
                    .opt("after", num(&after["Effort"]).map(number))
                    .value(),
            )
            .set("totalsNote", TOTALS_NOTE)
            .opt("parent", parent)
            .listed("notPersisted", not_persisted)
            .value(),
    )
}

// ---------------------------------------------------------------- custom fields

fn custom_values_json(values: &[CustomFieldValue]) -> Value {
    Value::Array(values.iter().map(CustomFieldValue::to_json).collect())
}

fn write_set_custom_fields() -> Tool {
    Tool {
        name: "write_set_custom_fields",
        description: r#"Set custom fields by name, e.g. {"BackEnd":"Done"}; dropdown values are validated first, null clears."#,
        fields: vec![
            id("id"),
            field("fields", Kind::Record(Some("give at least one field"))),
        ],
        handler: handler!(|ctx, args| { set_custom_fields(ctx, args).await }),
    }
}

async fn set_custom_fields(ctx: &Ctx, args: Args) -> Answer {
    let card = load(ctx, args.id("id")).await?;
    let info = &card.info;
    let Some(process_id) = info.process_id else {
        return Err(invalid(format!(
            "{} {} has no project/process",
            info.entity_type, info.id
        )));
    };
    let given = args.opt_map("fields").cloned().unwrap_or_default();
    let values = prepare_custom_fields(ctx, process_id, &info.entity_type, &given)
        .await
        .map_err(|r| unresolved(&r))?;
    let mut body = Map::new();
    body.insert("CustomFields".into(), custom_values_json(&values));
    ctx.v1
        .update(&info.resource.path, info.id, body, &WriteOptions::default())
        .await
        .map_err(failed(format!(
            "Could not set custom fields on {} {}",
            info.entity_type, info.id
        )))?;
    let out = Obj::new()
        .set("card", card_label(&card, None))
        .set("set", custom_values_json(&values));
    match reload(ctx, &card).await {
        Err(e) => success(out.set("warning", unverified(&e)).value()),
        Ok(after) => {
            let not_persisted = unpersisted_custom_fields(&values, &after["CustomFields"]);
            success(out.listed("notPersisted", not_persisted).value())
        }
    }
}

// ---------------------------------------------------------------- teams

fn write_team() -> Tool {
    Tool {
        name: "write_team",
        description: "Add or remove teams on a card (names or ids).",
        fields: vec![
            id("id"),
            field("add", list(Kind::NameOrId, None, None)).optional(),
            field("remove", list(Kind::NameOrId, None, None)).optional(),
        ],
        handler: handler!(|ctx, args| { team(ctx, args).await }),
    }
}

async fn team(ctx: &Ctx, args: Args) -> Answer {
    let add = args.opt_strs("add").unwrap_or_default();
    let remove = args.opt_strs("remove").unwrap_or_default();
    if add.is_empty() && remove.is_empty() {
        return Err(invalid("pass add and/or remove"));
    }
    let card = load(ctx, args.id("id")).await?;
    let info = &card.info;
    if any_catalog(ctx)
        .await
        .member(&info.resource, "AssignedTeams")
        .is_none()
    {
        return Err(invalid(format!("A {} cannot have teams", info.entity_type)));
    }
    let current = items(&card.raw["AssignedTeams"]);
    let mut to_add: Vec<(i64, String)> = Vec::new();
    // (TeamAssignment id, team name, team id)
    let mut to_remove: Vec<(i64, String, i64)> = Vec::new();
    for t in &add {
        let team = ctx
            .directory
            .team(t, false)
            .await
            .map_err(|r| unresolved(&r))?;
        if !current.iter().any(|c| ref_id(&c["Team"]) == Some(team.id)) {
            to_add.push((team.id, team.name));
        }
    }
    for t in &remove {
        let team = ctx
            .directory
            .team(t, true)
            .await
            .map_err(|r| unresolved(&r))?;
        let Some(ta) = current.iter().find(|c| ref_id(&c["Team"]) == Some(team.id)) else {
            return Err(invalid(format!(
                "{} is not assigned to {} {}",
                team.name, info.entity_type, info.id
            )));
        };
        to_remove.push((ta["Id"].as_i64().unwrap_or_default(), team.name, team.id));
    }
    let added: Vec<String> = to_add.iter().map(|(_, n)| n.clone()).collect();
    let removed: Vec<String> = to_remove.iter().map(|(_, n, _)| n.clone()).collect();
    let mut done: Vec<Value> = Vec::new();
    if !to_add.is_empty() {
        let team_items: Vec<Value> = to_add
            .iter()
            .map(|(id, _)| json!({ "Team": { "Id": id } }))
            .collect();
        let mut body = Map::new();
        body.insert("AssignedTeams".into(), json!({ "Items": team_items }));
        ctx.v1
            .update(&info.resource.path, info.id, body, &WriteOptions::default())
            .await
            .map_err(failed(format!(
                "Could not add teams to {} {}",
                info.entity_type, info.id
            )))?;
        done.push(json!({ "added": added }));
    }
    for (ta_id, name, _) in &to_remove {
        if let Err(e) = ctx.v1.delete("TeamAssignments", *ta_id).await {
            return Err(partial(
                &format!(
                    "Could not remove {name} from {} {}",
                    info.entity_type, info.id
                ),
                &e,
                done,
            ));
        }
        done.push(json!({ "removed": name }));
    }
    let out = Obj::new()
        .set("card", card_label(&card, None))
        .set("added", added)
        .set("removed", removed);
    let after = match reload(ctx, &card).await {
        Ok(after) => after,
        Err(e) => return success(out.set("warning", unverified(&e)).value()),
    };
    let teams_after = items(&after["AssignedTeams"]);
    let team_ids: Vec<Option<i64>> = teams_after.iter().map(|c| ref_id(&c["Team"])).collect();
    let mut not_persisted: Vec<String> = to_add
        .iter()
        .filter(|(id, _)| !team_ids.contains(&Some(*id)))
        .map(|(_, name)| format!("{name} is not on the card after adding"))
        .collect();
    not_persisted.extend(
        to_remove
            .iter()
            .filter(|(_, _, id)| team_ids.contains(&Some(*id)))
            .map(|(_, name, _)| format!("{name} is still on the card after removing")),
    );
    let teams: Vec<Value> = teams_after
        .iter()
        .map(|c| {
            Obj::new()
                .opt("team", ref_name(&c["Team"]))
                .opt("state", ref_name(&c["EntityState"]))
                .value()
        })
        .collect();
    success(
        out.set("teams", teams)
            .listed("notPersisted", not_persisted)
            .value(),
    )
}

// ---------------------------------------------------------------- shared verification

#[derive(Default)]
struct Verdict {
    not_persisted: Vec<String>,
    /// Sent, but not present in any read-back, so neither confirmed nor refuted.
    not_verified: Vec<String>,
}

/// `JSON.stringify(value)` of a possibly missing value.
fn stringify(value: Option<&Value>) -> String {
    value.map_or_else(|| "undefined".to_string(), js_json)
}

/// `String(value ?? '')`.
fn text_of(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => String::new(),
        Some(v) => js_string(v),
    }
}

/// Lower-cased, sorted and comma-joined tags.
fn tag_key(mut list: Vec<String>) -> String {
    js_sort(&mut list);
    list.join(",")
}

fn tag_list(value: &Value) -> Vec<String> {
    parse_tags(value)
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// Checks what create and update both write: name, description, tags,
/// release/iteration/team iteration, the parent link, and every extra field.
/// Fields the card read does not include are read explicitly; anything still
/// missing is listed as not verified rather than silently skipped.
async fn verify_written(
    ctx: &Ctx,
    info: &CardInfo,
    body: &Map<String, Value>,
    after: &Value,
    parent_field: Option<&str>,
    parent_id: Option<i64>,
    extra: &Map<String, Value>,
) -> Verdict {
    let mut v = Verdict::default();
    let has = |key: &str| after.as_object().is_some_and(|o| o.contains_key(key));
    if let Some(name) = body.get("Name")
        && after.get("Name") != Some(name)
    {
        v.not_persisted.push(format!(
            "name: requested {}, got {}",
            js_json(name),
            stringify(after.get("Name"))
        ));
    }
    if let Some(description) = body.get("Description")
        && html_to_text(Some(&text_of(after.get("Description"))))
            != html_to_text(Some(&js_string(description)))
    {
        v.not_persisted
            .push("description differs from what was sent".to_string());
    }
    if let Some(tags) = body.get("Tags") {
        let got = tag_key(
            tag_list(&after["Tags"])
                .iter()
                .map(|t| t.to_lowercase())
                .collect(),
        );
        let want = tag_key(
            js_string(tags)
                .split(',')
                .filter(|t| !t.is_empty())
                .map(str::to_lowercase)
                .collect(),
        );
        if got != want {
            v.not_persisted
                .push(format!("tags: requested [{want}], got [{got}]"));
        }
    }
    for key in ["Release", "Iteration", "TeamIteration"] {
        let Some(sent) = body.get(key) else { continue };
        if !has(key) {
            v.not_verified.push(key.to_string());
            continue;
        }
        let want = sent.get("Id").and_then(Value::as_i64);
        let got = ref_id(&after[key]);
        if want != got {
            v.not_persisted.push(format!(
                "{key}: requested {}, got {}",
                id_or_null(want),
                id_or_null(got)
            ));
        }
    }
    let parent_shown = parent_id.map_or_else(|| "undefined".to_string(), |p| p.to_string());
    match parent_field {
        Some("TestPlans") => {
            let options = ReadOptions {
                include: Some("[Id,TestPlans[Id]]".into()),
                inner_take: Some(1000),
                ..ReadOptions::default()
            };
            match ctx.v1.get(&info.resource.path, info.id, &options).await {
                Err(e) => v
                    .not_verified
                    .push(format!("test plan link ({})", e.message)),
                Ok(plans) => {
                    if !items(&plans["TestPlans"])
                        .iter()
                        .any(|p| p["Id"].as_i64().is_some() && p["Id"].as_i64() == parent_id)
                    {
                        v.not_persisted
                            .push(format!("not in test plan {parent_shown}"));
                    }
                }
            }
        }
        Some(field) => {
            if !has(field) {
                v.not_verified.push(field.to_string());
            } else {
                let got = ref_id(&after[field]);
                if got != parent_id {
                    v.not_persisted.push(format!(
                        "{field}: requested {parent_shown}, got {}",
                        id_or_null(got)
                    ));
                }
            }
        }
        None => {}
    }
    let missing: Vec<&str> = extra
        .keys()
        .filter(|k| !has(k))
        .map(String::as_str)
        .collect();
    let mut extra_back = after.clone();
    if !missing.is_empty() {
        let options = ReadOptions {
            include: Some(format!("[Id,{}]", missing.join(","))),
            ..ReadOptions::default()
        };
        if let Ok(Value::Object(data)) = ctx.v1.get(&info.resource.path, info.id, &options).await
            && let Value::Object(merged) = &mut extra_back
        {
            merged.extend(data);
        }
    }
    v.not_persisted.extend(unpersisted(extra, &extra_back));
    v.not_verified.extend(
        extra
            .keys()
            .filter(|k| extra_back.get(k.as_str()).is_none())
            .cloned(),
    );
    v
}

fn with_verdict(out: Obj, v: Verdict) -> Obj {
    out.listed("notPersisted", v.not_persisted)
        .listed("notVerified", v.not_verified)
}

// ---------------------------------------------------------------- create

/// Fields that have their own argument or tool, because a domain rule applies to them.
const RESERVED: [(&str, &str); 26] = [
    ("Name", "name"),
    ("Description", "description"),
    ("Project", "project (or parent, to inherit it)"),
    ("EntityState", "state, or write_set_state"),
    (
        "Effort",
        "write_set_role_effort: the card total is computed from role efforts",
    ),
    ("RoleEfforts", "roleEfforts, or write_set_role_effort"),
    ("Assignments", "assignees, or write_assign"),
    ("AssignedTeams", "teams, or write_team"),
    ("CustomFields", "customFields, or write_set_custom_fields"),
    ("Tags", "tags"),
    ("TagObjects", "tags"),
    ("Release", "release"),
    ("Iteration", "iteration"),
    ("TeamIteration", "teamIteration"),
    ("UserStory", "parent"),
    ("Feature", "parent"),
    ("Epic", "parent"),
    ("PortfolioEpic", "parent"),
    ("LinkedGeneral", "parent"),
    ("TestPlans", "parent"),
    (
        "Tasks",
        "write_create_card with parent (children get the creation rules)",
    ),
    (
        "Bugs",
        "write_create_card with parent (children get the creation rules)",
    ),
    (
        "UserStories",
        "write_create_card with parent (children get the creation rules)",
    ),
    (
        "Features",
        "write_create_card with parent (children get the creation rules)",
    ),
    (
        "Epics",
        "write_create_card with parent (children get the creation rules)",
    ),
    ("TestCases", "write_test_cases"),
];

fn reserved_fields(fields: &Map<String, Value>) -> Option<String> {
    let hits: Vec<String> = fields
        .keys()
        .filter_map(|k| {
            let lower = k.to_lowercase();
            RESERVED
                .iter()
                .find(|(r, _)| r.to_lowercase() == lower)
                .map(|(r, hint)| format!("{r} → use {hint}"))
        })
        .collect();
    (!hits.is_empty()).then(|| {
        format!(
            "these fields have their own argument or tool:\n- {}",
            hits.join("\n- ")
        )
    })
}

/// `{Id}` or null for release, iteration and team iteration, when given.
fn reference_args(args: &Args, body: &mut Map<String, Value>) -> Vec<&'static str> {
    let mut given = Vec::new();
    for (key, arg) in [
        ("Release", "release"),
        ("Iteration", "iteration"),
        ("TeamIteration", "teamIteration"),
    ] {
        if let Some(value) = args.get(arg) {
            let value = match value.as_i64() {
                Some(id) => json!({ "Id": id }),
                None => Value::Null,
            };
            body.insert(key.into(), value);
            given.push(key);
        }
    }
    given
}

/// `shapeCard(...)` without the description: the caller just sent it, and verifyWritten reports a difference.
fn shaped_without_description(info: &CardInfo, raw: &Value) -> Value {
    let mut card = shape_card(info, raw, None);
    if let Value::Object(o) = &mut card {
        o.shift_remove("description");
    }
    card
}

fn write_create_card() -> Tool {
    Tool {
        name: "write_create_card",
        description: "Create a card under its parent, inheriting the project; replaces default assignees with the requested ones, reports parent changes.",
        fields: vec![
            field("type", nonempty()).describe("Entity type, e.g. UserStory, Task, Bug"),
            field("name", nonempty()),
            id("parent").optional().describe(
                "Parent card id (story for a task, feature for a story, test plan for a test case, ...)",
            ),
            name_or_id("project")
                .optional()
                .describe("Project; defaults to the parent's project"),
            text_param("description"),
            rich_format_field(),
            name_or_id("state").optional(),
            field("teams", list(Kind::NameOrId, None, None))
                .optional()
                .describe("Teams to assign; TP_DEFAULT_TEAM_ID applies when omitted"),
            field(
                "assignees",
                list(Kind::Object(vec![person("user"), role("role")]), None, None),
            )
            .optional(),
            efforts_field("roleEfforts").optional(),
            field("tags", list(tag(), None, None)).optional(),
            field("customFields", Kind::Record(None)).optional(),
            ref_or_null("release"),
            ref_or_null("iteration"),
            ref_or_null("teamIteration"),
            field("fields", Kind::Record(None)).optional().describe(
                "Any other settable fields as Targetprocess JSON, validated against the catalog",
            ),
        ],
        handler: handler!(|ctx, args| { create_card(ctx, args).await }),
    }
}

async fn create_card(ctx: &Ctx, args: Args) -> Answer {
    let type_name = args.str("type");
    let card_name = args.str("name");
    let catalog = ctx
        .catalog
        .get(&Request::Names(vec![type_name.clone()]))
        .await;
    let resource = catalog.resource(&type_name).map_err(invalid)?.clone();
    if !is_card_resource(&resource) || is_admin_resource(&resource) {
        return Err(invalid(format!(
            "{} is not a card type pounce creates here; use write_create (raw toolset)",
            resource.name
        )));
    }
    let member = |name: &str| catalog.member(&resource, name).is_some();

    // ---- resolve everything before posting
    let mut body = Map::new();
    body.insert("Name".into(), json!(card_name));
    let mut parent_ref: Option<ParentRef> = None;
    let mut parent_field: Option<String> = None;
    let mut parent_project: Option<ProjectRef> = None;
    if let Some(parent_id) = args.opt_id("parent") {
        let p = card_info(ctx, parent_id)
            .await
            .map_err(|r| unresolved(&r))?;
        let link = parent_link(&resource, &p).map_err(invalid)?;
        body.extend(link.body);
        parent_project = p.project.clone();
        parent_ref = Some(ParentRef {
            id: p.id,
            resource_type: Some(p.entity_type.clone()),
        });
        parent_field = Some(link.field);
    }

    let mut process_id: Option<i64> = None;
    let project_id = match args.opt_str("project") {
        Some(input) => {
            let project = ctx
                .directory
                .project(&input)
                .await
                .map_err(|r| unresolved(&r))?;
            process_id = project.process.map(|p| p.id);
            Some(project.id)
        }
        None => parent_project
            .as_ref()
            .map(|p| p.id)
            .or(ctx.config.default_project_id),
    };
    let Some(project_id) = project_id else {
        return Err(invalid(
            "no project could be resolved: pass project, or a parent card to inherit it from (TP_DEFAULT_PROJECT_ID is not set)",
        ));
    };
    if process_id.is_none() {
        let project = ctx
            .directory
            .project(&project_id.to_string())
            .await
            .map_err(|r| unresolved(&r))?;
        process_id = project.process.map(|p| p.id);
    }
    body.insert("Project".into(), json!({ "Id": project_id }));

    if let Some(description) = args.opt_str("description") {
        body.insert(
            "Description".into(),
            json!(text_to_html(&description, rich_format(&args))),
        );
    }
    if let Some(state_input) = args.opt_str("state") {
        let Some(pid) = process_id else {
            return Err(invalid(
                "the project has no process, so states cannot be resolved",
            ));
        };
        let state = ctx
            .directory
            .state(pid, &resource.name, &state_input, false)
            .await
            .map_err(|r| unresolved(&r))?;
        body.insert("EntityState".into(), json!({ "Id": state.id }));
    }

    // (id, name when resolved from the arguments)
    let mut teams: Vec<(i64, Option<String>)> = Vec::new();
    let team_args = args.opt_strs("teams");
    for t in team_args.iter().flatten() {
        let team = ctx
            .directory
            .team(t, false)
            .await
            .map_err(|r| unresolved(&r))?;
        teams.push((team.id, Some(team.name)));
    }
    if team_args.is_none()
        && let Some(default_team) = ctx.config.default_team_id
        && member("AssignedTeams")
    {
        teams.push((default_team, None));
    }
    if !teams.is_empty() {
        if !member("AssignedTeams") {
            return Err(invalid(format!("A {} cannot have teams", resource.name)));
        }
        let team_items: Vec<Value> = teams
            .iter()
            .map(|(id, _)| json!({ "Team": { "Id": id } }))
            .collect();
        body.insert("AssignedTeams".into(), json!({ "Items": team_items }));
    }

    let mut assignees: Vec<(TpUser, TpRole)> = Vec::new();
    for a in args.opt_list("assignees").into_iter().flatten() {
        let user = resolve_person(ctx, a["user"].as_str().unwrap_or_default(), false)
            .await
            .map_err(|r| unresolved(&r))?;
        let r = ctx
            .directory
            .role(a["role"].as_str().unwrap_or_default())
            .await
            .map_err(|r| unresolved(&r))?;
        if !assignees
            .iter()
            .any(|(u, x)| u.id == user.id && x.id == r.id)
        {
            assignees.push((user, r));
        }
    }
    if !assignees.is_empty() && !member("Assignments") {
        return Err(invalid(format!(
            "A {} cannot have assignments",
            resource.name
        )));
    }
    let requested = |a: &AssignmentRow| {
        assignees
            .iter()
            .any(|(u, r)| u.id == a.user.id && r.id == a.role.id)
    };

    let mut efforts: Vec<EffortRequest> = Vec::new();
    if let Some(list) = args.opt_list("roleEfforts").filter(|l| !l.is_empty()) {
        if !member("RoleEfforts") {
            return Err(invalid(format!("A {} has no role efforts", resource.name)));
        }
        efforts = resolve_efforts(ctx, list).await?;
    }

    let mut custom_fields: Vec<CustomFieldValue> = Vec::new();
    if let Some(given) = args.opt_map("customFields").filter(|m| !m.is_empty()) {
        let Some(pid) = process_id else {
            return Err(invalid(
                "the project has no process, so custom fields cannot be resolved",
            ));
        };
        custom_fields = prepare_custom_fields(ctx, pid, &resource.name, given)
            .await
            .map_err(|r| unresolved(&r))?;
        body.insert("CustomFields".into(), custom_values_json(&custom_fields));
    }
    if let Some(tags) = args.opt_strs("tags").filter(|t| !t.is_empty()) {
        let mut unique: Vec<&str> = Vec::new();
        for t in &tags {
            let t = js_trim(t);
            if !unique.contains(&t) {
                unique.push(t);
            }
        }
        body.insert("Tags".into(), json!(unique.join(",")));
    }
    reference_args(&args, &mut body);
    let mut extra_fields = Map::new();
    if let Some(given) = args.opt_map("fields").filter(|m| !m.is_empty()) {
        if let Some(reserved) = reserved_fields(given) {
            return Err(invalid(reserved));
        }
        let for_body = catalog_for_body(&ctx.catalog, catalog.clone(), &resource, given).await;
        extra_fields =
            prepare_body(&for_body, &resource, given, Mode::Create, "").map_err(invalid)?;
        body.extend(extra_fields.clone());
    }

    // ---- write, sequentially
    let parent_before = match &parent_ref {
        Some(p) => Some(snapshot_of(ctx, p).await),
        None => None,
    };
    let options = WriteOptions {
        result_include: Some("[Id,Name]".into()),
        ..WriteOptions::default()
    };
    let created = ctx
        .v1
        .create(&resource.path, Value::Object(body.clone()), &options)
        .await
        .map_err(failed(format!("Could not create the {}", resource.name)))?;
    let Some(new_id) = created.get("Id").and_then(Value::as_i64) else {
        let mut details = Map::new();
        details.insert("response".into(), created.clone());
        return Err(failure(
            &format!("Created a {} but the response carried no Id", resource.name),
            None,
            Some(&details),
        ));
    };
    let mut done: Vec<Value> = vec![json!({ "created": { "id": new_id, "type": resource.name } })];
    let created_label = format!("Created {} {new_id}", resource.name);

    let mut removed_defaults: Vec<Value> = Vec::new();
    let has_assignments = member("Assignments");
    if has_assignments {
        let defaults = match list_assignments(ctx, new_id).await {
            Ok(rows) => rows,
            Err(e) => {
                return Err(partial(
                    &format!("{created_label} but could not read its default assignments"),
                    &e,
                    done,
                ));
            }
        };
        for a in defaults.iter().filter(|a| !requested(a)) {
            if let Err(e) = remove_assignment(ctx, a.id).await {
                return Err(partial(
                    &format!(
                        "{created_label} but could not remove default assignment {}",
                        user_as_role(a)
                    ),
                    &e,
                    done,
                ));
            }
            removed_defaults.push(describe_assignment(a));
            done.push(json!({ "removedDefault": describe_assignment(a) }));
        }
        let kept: Vec<&AssignmentRow> = defaults.iter().filter(|a| requested(a)).collect();
        for (user, r) in &assignees {
            if kept
                .iter()
                .any(|k| k.user.id == user.id && k.role.id == r.id)
            {
                continue;
            }
            if let Err(e) = add_assignment(ctx, new_id, user.id, r.id).await {
                return Err(partial(
                    &format!(
                        "{created_label} but could not assign {} as {}",
                        full_name(user),
                        r.name
                    ),
                    &e,
                    done,
                ));
            }
            done.push(json!({ "assigned": format!("{} as {}", full_name(user), r.name) }));
        }
    }

    let info = CardInfo {
        id: new_id,
        name: card_name.clone(),
        entity_type: resource.name.clone(),
        resource: resource.clone(),
        project: Some(ProjectRef {
            id: project_id,
            name: String::new(),
        }),
        process_id,
    };
    if !efforts.is_empty() {
        let fresh = match read_card_as(ctx, &info).await {
            Ok(fresh) => fresh,
            Err(e) => {
                return Err(partial(
                    &format!("{created_label} but could not read it to set role efforts"),
                    &e,
                    done,
                ));
            }
        };
        let outcome = apply_role_efforts(ctx, new_id, &role_effort_rows(&fresh), &efforts).await;
        if let Some(error) = &outcome.error {
            done.extend(outcome.applied.iter().map(|e| e.to_json()));
            return Err(partial(
                &format!("{created_label}; {}", error.message),
                &error.error,
                done,
            ));
        }
        done.extend(
            outcome
                .applied
                .iter()
                .map(|e| json!({ "effort": e.to_json() })),
        );
    }

    // Targetprocess can add default assignments after the first check: look again and remove them.
    if has_assignments {
        for wait in &ctx.settle_delays_ms {
            tokio::time::sleep(Duration::from_millis(*wait)).await;
            let Ok(now) = list_assignments(ctx, new_id).await else {
                break;
            };
            for a in now.iter().filter(|a| !requested(a)) {
                if let Err(e) = remove_assignment(ctx, a.id).await {
                    return Err(partial(
                        &format!(
                            "{created_label} but could not remove late default assignment {}",
                            user_as_role(a)
                        ),
                        &e,
                        done,
                    ));
                }
                let mut late = describe_assignment(a);
                late["late"] = json!(true);
                removed_defaults.push(late);
                done.push(json!({ "removedDefault": describe_assignment(a), "late": true }));
            }
        }
    }

    // ---- read back and verify
    let created_json = json!({ "id": new_id, "type": resource.name, "name": card_name });
    let raw = match read_card_as(ctx, &info).await {
        Ok(raw) => raw,
        Err(e) => {
            // Every write succeeded; only the check failed. Not an error: a retry would create a duplicate.
            let parent_after = snapshot_again(ctx, parent_before.as_ref()).await;
            let parent = parent_before
                .as_ref()
                .and_then(|_| parent_json(parent_before.as_ref(), parent_after.as_ref()));
            return success(
                Obj::new()
                    .set("created", created_json)
                    .set("done", done)
                    .set("removedDefaultAssignments", removed_defaults)
                    .opt("parent", parent)
                    .set("warning", unverified(&e))
                    .value(),
            );
        }
    };
    let mut not_persisted: Vec<String> = Vec::new();
    let got_project = v1_ref(&raw["Project"]);
    if got_project.as_ref().map(|p| p.id) != Some(project_id) {
        not_persisted.push(format!(
            "project: requested {project_id}, got {}",
            got_project
                .as_ref()
                .map_or_else(|| "undefined".to_string(), |p| p.id.to_string())
        ));
    }
    if let Some(wanted) = body.get("EntityState").and_then(|s| s["Id"].as_i64()) {
        let got = v1_ref(&raw["EntityState"]);
        if got.as_ref().map(|s| s.id) != Some(wanted) {
            not_persisted.push(format!(
                "state: got {}",
                shown(got.as_ref().and_then(|s| s.name.as_deref()))
            ));
        }
    }
    if has_assignments {
        let rows = items(&raw["Assignments"]);
        let id_text =
            |v: &Value| ref_id(v).map_or_else(|| "undefined".to_string(), |i| i.to_string());
        let mut got: Vec<String> = rows
            .iter()
            .map(|a| format!("{}:{}", id_text(&a["GeneralUser"]), id_text(&a["Role"])))
            .collect();
        js_sort(&mut got);
        let mut want: Vec<String> = assignees
            .iter()
            .map(|(u, r)| format!("{}:{}", u.id, r.id))
            .collect();
        js_sort(&mut want);
        if got.join(",") != want.join(",") {
            let listed: Vec<String> = rows
                .iter()
                .map(|a| {
                    format!(
                        "{} as {}",
                        id_text(&a["GeneralUser"]),
                        shown(ref_name(&a["Role"]).as_deref())
                    )
                })
                .collect();
            let listed = if listed.is_empty() {
                "none".to_string()
            } else {
                listed.join(", ")
            };
            not_persisted.push(format!(
                "assignments: expected exactly {}, card has {listed}",
                want.len()
            ));
        }
    }
    not_persisted.extend(unpersisted_efforts(&efforts, &raw, " effort"));
    let team_ids_after: Vec<Option<i64>> = items(&raw["AssignedTeams"])
        .iter()
        .map(|t| ref_id(&t["Team"]))
        .collect();
    for (id, name) in &teams {
        if !team_ids_after.contains(&Some(*id)) {
            not_persisted.push(format!(
                "team {} is not on the card",
                name.clone().unwrap_or_else(|| id.to_string())
            ));
        }
    }
    not_persisted.extend(unpersisted_custom_fields(
        &custom_fields,
        &raw["CustomFields"],
    ));
    let common = verify_written(
        ctx,
        &info,
        &body,
        &raw,
        parent_field.as_deref(),
        parent_ref.as_ref().map(|p| p.id),
        &extra_fields,
    )
    .await;
    not_persisted.extend(common.not_persisted);

    let parent_after = snapshot_again(ctx, parent_before.as_ref()).await;
    let mut shaped_info = info.clone();
    if let Some(p) = got_project {
        shaped_info.project = Some(ProjectRef {
            id: p.id,
            name: p.name.unwrap_or_default(),
        });
    }
    let out = Obj::new()
        .set("created", created_json)
        .set("card", shaped_without_description(&shaped_info, &raw))
        .set("removedDefaultAssignments", removed_defaults)
        .opt("totalsNote", (!efforts.is_empty()).then_some(TOTALS_NOTE))
        .opt(
            "parent",
            parent_json(parent_before.as_ref(), parent_after.as_ref()),
        );
    success(
        with_verdict(
            out,
            Verdict {
                not_persisted,
                not_verified: common.not_verified,
            },
        )
        .value(),
    )
}

// ---------------------------------------------------------------- update

fn write_update_card() -> Tool {
    Tool {
        name: "write_update_card",
        description: "Update a card's name, description, tags, release, iteration, parent or other fields; changes are read back.",
        fields: vec![
            id("id"),
            field("name", nonempty()).optional(),
            text_param("description"),
            rich_format_field(),
            field("tags", list(tag(), None, None))
                .optional()
                .describe("Replaces the whole list; use addTags/removeTags to edit it"),
            field("addTags", list(tag(), None, None)).optional(),
            field("removeTags", list(nonempty(), None, None)).optional(),
            ref_or_null("release"),
            ref_or_null("iteration"),
            ref_or_null("teamIteration"),
            id("parent").optional().describe(
                "New parent card; for a test case, a test plan to ADD it to (existing plans are kept)",
            ),
            field("fields", Kind::Record(None)).optional().describe(
                "Other settable fields; state, people, effort, teams and custom fields have their own tools",
            ),
        ],
        handler: handler!(|ctx, args| { update_card(ctx, args).await }),
    }
}

/// `t.trim().toLowerCase()`.
fn tag_lower(t: &str) -> String {
    js_trim(t).to_lowercase()
}

async fn update_card(ctx: &Ctx, args: Args) -> Answer {
    let card = load(ctx, args.id("id")).await?;
    let info = &card.info;
    let catalog = any_catalog(ctx).await;
    let mut body = Map::new();
    if let Some(name) = args.opt_str("name") {
        body.insert("Name".into(), json!(name));
    }
    if let Some(description) = args.opt_str("description") {
        body.insert(
            "Description".into(),
            json!(text_to_html(&description, rich_format(&args))),
        );
    }

    let current_tags = tag_list(&card.raw["Tags"]);
    let mut removed_tags: Vec<String> = Vec::new();
    let tags = args.opt_strs("tags");
    let add_tags = args.opt_strs("addTags").unwrap_or_default();
    let remove_tags = args.opt_strs("removeTags").unwrap_or_default();
    if tags.is_some() || !add_tags.is_empty() || !remove_tags.is_empty() {
        if tags.is_some() && (!add_tags.is_empty() || !remove_tags.is_empty()) {
            return Err(invalid("use either tags or addTags/removeTags"));
        }
        let mut next: Vec<String> = match &tags {
            Some(list) => list
                .iter()
                .map(|t| js_trim(t).to_string())
                .filter(|t| !t.is_empty())
                .collect(),
            None => current_tags.clone(),
        };
        for t in &add_tags {
            if !next.iter().any(|n| tag_lower(n) == tag_lower(t)) {
                next.push(js_trim(t).to_string());
            }
        }
        if !remove_tags.is_empty() {
            next.retain(|n| !remove_tags.iter().any(|r| tag_lower(r) == tag_lower(n)));
        }
        // `new Map(...)`: the first spelling's position, the last spelling's value.
        let mut keys: Vec<String> = Vec::new();
        let mut unique: Vec<String> = Vec::new();
        for t in next {
            let key = tag_lower(&t);
            match keys.iter().position(|k| *k == key) {
                Some(i) => unique[i] = t,
                None => {
                    keys.push(key);
                    unique.push(t);
                }
            }
        }
        removed_tags = current_tags
            .iter()
            .filter(|t| !unique.iter().any(|n| tag_lower(n) == tag_lower(t)))
            .cloned()
            .collect();
        body.insert("Tags".into(), json!(unique.join(",")));
    }
    let mut references = Map::new();
    for key in reference_args(&args, &mut references) {
        if catalog.member(&info.resource, key).is_none() {
            return Err(invalid(format!("A {} has no {key}", info.entity_type)));
        }
    }
    body.extend(references);
    let mut parent_field: Option<String> = None;
    let parent_id = args.opt_id("parent");
    if let Some(parent_id) = parent_id {
        let p = card_info(ctx, parent_id)
            .await
            .map_err(|r| unresolved(&r))?;
        let link = parent_link(&info.resource, &p).map_err(invalid)?;
        body.extend(link.body);
        parent_field = Some(link.field);
    }
    let mut extra_fields = Map::new();
    if let Some(given) = args.opt_map("fields").filter(|m| !m.is_empty()) {
        if let Some(reserved) = reserved_fields(given) {
            return Err(invalid(format!(
                "{reserved}\n(write_update in the raw toolset is the escape hatch for an explicit override)"
            )));
        }
        let for_body = catalog_for_body(&ctx.catalog, catalog.clone(), &info.resource, given).await;
        extra_fields =
            prepare_body(&for_body, &info.resource, given, Mode::Update, "").map_err(invalid)?;
        body.extend(extra_fields.clone());
    }
    if body.is_empty() {
        return Err(invalid("nothing to update"));
    }

    ctx.v1
        .update(
            &info.resource.path,
            info.id,
            body.clone(),
            &WriteOptions::default(),
        )
        .await
        .map_err(failed(format!(
            "Could not update {} {}",
            info.entity_type, info.id
        )))?;
    let updated: Vec<String> = body.keys().cloned().collect();
    let after = match reload(ctx, &card).await {
        Ok(after) => after,
        Err(e) => {
            return success(
                Obj::new()
                    .set("card", card_label(&card, None))
                    .set("updated", updated)
                    .listed("removedTags", removed_tags)
                    .set("warning", unverified(&e))
                    .value(),
            );
        }
    };
    let checked = verify_written(
        ctx,
        info,
        &body,
        &after,
        parent_field.as_deref(),
        parent_id,
        &extra_fields,
    )
    .await;
    let after_name = after.get("Name").and_then(Value::as_str);
    let mut now_info = info.clone();
    if let Some(name) = after_name {
        now_info.name = name.to_string();
    }
    let out = Obj::new()
        .set("card", card_label(&card, Some(&after)))
        .opt(
            "renamedFrom",
            (after_name != Some(info.name.as_str())).then(|| info.name.clone()),
        )
        .set("updated", updated)
        .listed("removedTags", removed_tags)
        .set("now", shaped_without_description(&now_info, &after));
    success(with_verdict(out, checked).value())
}

// ---------------------------------------------------------------- comments, time, relations, follow

fn write_comment() -> Tool {
    Tool {
        name: "write_comment",
        description: "Add a comment to a card, or reply to one with replyTo.",
        fields: vec![
            id("id"),
            field("text", nonempty()),
            rich_format_field(),
            id("replyTo").optional(),
            flag("private"),
        ],
        handler: handler!(|ctx, args| { comment(ctx, args).await }),
    }
}

async fn comment(ctx: &Ctx, args: Args) -> Answer {
    let card_id = args.id("id");
    let description = text_to_html(&args.str("text"), rich_format(&args));
    let mut body = Map::new();
    body.insert("General".into(), json!({ "Id": card_id }));
    body.insert("Description".into(), json!(description));
    if let Some(reply_to) = args.opt_id("replyTo") {
        body.insert("ParentId".into(), json!(reply_to));
    }
    if let Some(private) = args.opt_bool("private") {
        body.insert("IsPrivate".into(), json!(private));
    }
    let options = WriteOptions {
        result_include: Some("[Id,Description,General[Id,Name],ParentId,CreateDate]".into()),
        ..WriteOptions::default()
    };
    let r = ctx
        .v1
        .create("Comments", Value::Object(body), &options)
        .await
        .map_err(failed(format!("Could not comment on {card_id}")))?;
    let persisted =
        html_to_text(Some(&text_of(r.get("Description")))) == html_to_text(Some(&description));
    let card = v1_ref(&r["General"]).map_or_else(|| json!({ "id": card_id }), |g| ref_json(&g));
    let mut out = Obj::new()
        .opt("comment", r.get("Id").and_then(num).map(number))
        .set("card", card);
    if !persisted {
        out = out.set(
            "notPersisted",
            vec!["the stored comment text differs from what was sent"],
        );
    }
    success(out.value())
}

fn write_log_time() -> Tool {
    Tool {
        name: "write_log_time",
        description: "Log time spent on a card for a person (default: you, today). Creates a Time entry or a time record, as the process tracks it.",
        fields: vec![
            id("id"),
            field(
                "spent",
                Kind::Num {
                    int: false,
                    positive: true,
                    min: None,
                    max: Some(1000.0),
                },
            ),
            field(
                "remain",
                Kind::Num {
                    int: false,
                    positive: false,
                    min: Some(0.0),
                    max: Some(100_000.0),
                },
            )
            .optional()
            .describe("Remaining time to record on the card"),
            field("date", pattern(r"^\d{4}-\d{2}-\d{2}$", None))
                .optional()
                .describe("YYYY-MM-DD; default today in the server's time zone"),
            text_param("description"),
            role("role").optional().describe(
                "Defaults to the person's only role on the card; required when they have none or several",
            ),
            person("user").optional(),
        ],
        handler: handler!(|ctx, args| { log_time(ctx, args).await }),
    }
}

async fn log_time(ctx: &Ctx, args: Args) -> Answer {
    let card = load(ctx, args.id("id")).await?;
    let info = &card.info;
    let Some(project) = &info.project else {
        return Err(invalid(format!(
            "{} {} has no project",
            info.entity_type, info.id
        )));
    };
    let user = resolve_person(
        ctx,
        &args.opt_str("user").unwrap_or_else(|| "me".into()),
        false,
    )
    .await
    .map_err(|r| unresolved(&r))?;
    let backend = time_backend(ctx, info.process_id)
        .await
        .map_err(|p| p.to_result())?;
    if let TimeBackend::TimeRecord(record) = backend {
        return log_time_record(ctx, &card, &user, &record, &args).await;
    }
    let spent = args.opt_num("spent").unwrap_or_default();
    let (role_id, role_name) = match args.opt_str("role") {
        Some(input) => {
            let r = ctx
                .directory
                .role(&input)
                .await
                .map_err(|r| unresolved(&r))?;
            (r.id, Some(r.name))
        }
        None => {
            let mine: Vec<Value> = items(&card.raw["Assignments"])
                .into_iter()
                .filter(|a| ref_id(&a["GeneralUser"]) == Some(user.id))
                .collect();
            // A single row without a role reference fails like `ref(...)!` would: no role to log against.
            let single = match mine.as_slice() {
                [one] => v1_ref(&one["Role"]),
                _ => None,
            };
            let Some(r) = single else {
                let held = if mine.is_empty() {
                    "no role".to_string()
                } else {
                    // Array.join prints a missing name as empty.
                    let names: Vec<String> = mine
                        .iter()
                        .map(|a| ref_name(&a["Role"]).unwrap_or_default())
                        .collect();
                    format!("{} roles ({})", mine.len(), names.join(", "))
                };
                return Err(invalid(format!(
                    "{} has {held} on {} {}; pass role",
                    full_name(&user),
                    info.entity_type,
                    info.id
                )));
            };
            (r.id, r.name)
        }
    };
    let mut body = Map::new();
    body.insert("Assignable".into(), json!({ "Id": info.id }));
    body.insert("Project".into(), json!({ "Id": project.id }));
    body.insert("User".into(), json!({ "Id": user.id }));
    body.insert("Role".into(), json!({ "Id": role_id }));
    body.insert("Spent".into(), number(spent));
    body.insert(
        "Date".into(),
        json!(args.opt_str("date").unwrap_or_else(|| (ctx.clock.today)())),
    );
    if let Some(remain) = args.opt_num("remain") {
        body.insert("Remain".into(), number(remain));
    }
    if let Some(description) = args.opt_str("description").filter(|d| !d.is_empty()) {
        body.insert("Description".into(), json!(description));
    }
    let options = WriteOptions {
        result_include: Some("[Id,Spent,Remain,Date]".into()),
        ..WriteOptions::default()
    };
    let r = ctx
        .v1
        .create("Times", Value::Object(body), &options)
        .await
        .map_err(failed(format!(
            "Could not log time on {} {}",
            info.entity_type, info.id
        )))?;
    let got_spent = r.get("Spent").and_then(num);
    let not_persisted = if got_spent == Some(spent) {
        Vec::new()
    } else {
        let shown = match r.get("Spent") {
            None => "undefined".to_string(),
            Some(v) => js_string(v),
        };
        vec![format!(
            "spent: requested {}, got {shown}",
            js_number(spent)
        )]
    };
    let after = reload(ctx, &card).await;
    let card_time = after.ok().map(|a| {
        Obj::new()
            .opt("spent", num(&a["TimeSpent"]).map(number))
            .opt("remaining", num(&a["TimeRemain"]).map(number))
            .value()
    });
    success(
        Obj::new()
            .opt("time", r.get("Id").and_then(num).map(number))
            .set("card", card_label(&card, None))
            .set("user", who(&user))
            .opt("role", role_name)
            .opt("spent", got_spent.map(number))
            .opt("date", r.get("Date").cloned())
            .opt("cardTime", card_time)
            .listed("notPersisted", not_persisted)
            .value(),
    )
}

/// write_log_time on an instance that tracks time in a TimeRecord type.
async fn log_time_record(
    ctx: &Ctx,
    card: &CardRead,
    user: &TpUser,
    backend: &TimeRecordBackend,
    args: &Args,
) -> Answer {
    let info = &card.info;
    let kind = &backend.resource.name;
    if args.has("remain") {
        return Err(invalid(format!(
            "time records have no remaining time; log only spent ({kind})"
        )));
    }
    let Some(card_ref) = backend.card_refs.iter().find(|r| **r == info.entity_type) else {
        return Err(invalid(format!(
            "a {kind} cannot be linked to a {}; it links to {}",
            info.entity_type,
            backend.card_refs.join(", ")
        )));
    };
    let spent = args.opt_num("spent").unwrap_or_default();
    let date = args.opt_str("date").unwrap_or_else(|| (ctx.clock.today)());
    let mut links: Vec<(String, i64)> = vec![(card_ref.clone(), info.id)];
    // A task's record also carries its story, as the team's own records do.
    if info.entity_type == "Task"
        && let Some(story) = v1_ref(&card.raw["UserStory"])
        && backend.card_refs.iter().any(|r| r == "UserStory")
    {
        match links.iter_mut().find(|(k, _)| k == "UserStory") {
            Some(link) => link.1 = story.id,
            None => links.push(("UserStory".into(), story.id)),
        }
    }
    let mut body = Map::new();
    // Provisional: an automation renames records ("<card> / <person> / <hours>h").
    body.insert(
        "Name".into(),
        json!(format!(
            "{} / {} / {}h",
            info.name,
            full_name(user),
            js_number(spent)
        )),
    );
    for (k, id) in &links {
        body.insert(k.clone(), json!({ "Id": id }));
    }
    body.insert("ConnectedUser".into(), json!({ "Id": user.id }));
    let mut custom = vec![json!({ "Name": backend.hours, "Value": number(spent) })];
    if let Some(date_field) = &backend.date {
        custom.push(json!({ "Name": date_field, "Value": date }));
    }
    body.insert("CustomFields".into(), Value::Array(custom));
    if let Some(description) = args.opt_str("description").filter(|d| !d.is_empty()) {
        body.insert("Description".into(), json!(description));
    }
    let options = WriteOptions {
        result_include: Some("[Id]".into()),
        ..WriteOptions::default()
    };
    let created = ctx
        .v1
        .create(&backend.resource.path, Value::Object(body), &options)
        .await
        .map_err(failed(format!(
            "Could not log time on {} {}",
            info.entity_type, info.id
        )))?;
    let record_id = created
        .get("Id")
        .and_then(Value::as_i64)
        .unwrap_or_default();
    let link_include: Vec<String> = links.iter().map(|(k, _)| format!("{k}[Id]")).collect();
    let read = ReadOptions {
        include: Some(format!(
            "[Id,Name,ConnectedUser[Id],{},CustomFields,DayPeriod[Name]]",
            link_include.join(",")
        )),
        ..ReadOptions::default()
    };
    let back = ctx.v1.get(&backend.resource.path, record_id, &read).await;
    let label = Obj::new()
        .set("time", record_id)
        .set("kind", kind.as_str())
        .set("card", card_label(card, None))
        .set("user", who(user))
        .set("spent", number(spent))
        .set("date", date.as_str())
        .opt(
            "note",
            args.has("role")
                .then(|| format!("role ignored: {kind} entries have no role")),
        );
    let back = match back {
        Ok(back) => back,
        Err(e) => {
            return success(
                label
                    .set(
                        "warning",
                        format!(
                            "The record was created but could not be read back to verify it ({})",
                            e.message
                        ),
                    )
                    .value(),
            );
        }
    };
    let fields: &[Value] = back["CustomFields"].as_array().map_or(&[], Vec::as_slice);
    let value = |name: &str| {
        fields
            .iter()
            .find(|f| f.get("Name").and_then(Value::as_str) == Some(name))
            .and_then(|f| f.get("Value"))
    };
    let or_null = |v: Option<&Value>| js_json(v.unwrap_or(&Value::Null));
    let mut not_persisted: Vec<String> = Vec::new();
    let hours = value(&backend.hours);
    if !hours.is_some_and(|h| h.as_f64() == Some(spent)) {
        not_persisted.push(format!(
            "{}: requested {}, got {}",
            backend.hours,
            js_number(spent),
            or_null(hours)
        ));
    }
    if let Some(date_field) = &backend.date {
        let got = value(date_field);
        if !got
            .and_then(Value::as_str)
            .is_some_and(|g| same_date_value(&date, g))
        {
            not_persisted.push(format!(
                "{date_field}: requested {date}, got {}",
                or_null(got)
            ));
        }
    }
    let connected = ref_id(&back["ConnectedUser"]);
    if connected != Some(user.id) {
        not_persisted.push(format!(
            "user: requested {}, got {}",
            user.id,
            id_or_null(connected)
        ));
    }
    for (k, id) in &links {
        let got = ref_id(&back[k.as_str()]);
        if got != Some(*id) {
            not_persisted.push(format!("{k}: requested {id}, got {}", id_or_null(got)));
        }
    }
    success(
        label
            .opt("name", back.get("Name").cloned())
            .opt(
                "period",
                ref_name(&back["DayPeriod"]).filter(|n| !n.is_empty()),
            )
            .listed("notPersisted", not_persisted)
            .value(),
    )
}

fn write_relate() -> Tool {
    Tool {
        name: "write_relate",
        description: "Relate two cards (Relation by default; Dependency, Blocker, Link, Duplicate, ...). An identical relation is left alone.",
        fields: vec![
            id("id"),
            id("to"),
            name_or_id("relation").optional(),
            field("direction", Kind::OneOf(&["outbound", "inbound"]))
                .optional()
                .describe("outbound (default): id is the master of to; inbound: to is the master"),
        ],
        handler: handler!(|ctx, args| { relate(ctx, args).await }),
    }
}

/// `String(t.Name)`.
fn name_text(t: &Value) -> String {
    t.get("Name")
        .map_or_else(|| "undefined".to_string(), js_string)
}

async fn relate(ctx: &Ctx, args: Args) -> Answer {
    let card_id = args.id("id");
    let to = args.id("to");
    if card_id == to {
        return Err(invalid("a card cannot relate to itself"));
    }
    let list_options = |include: &str, where_: Option<String>| ListOptions {
        read: ReadOptions {
            include: Some(include.into()),
            ..ReadOptions::default()
        },
        where_,
        ..ListOptions::default()
    };
    let types = ctx
        .v1
        .list("RelationTypes", &list_options("[Id,Name]", None))
        .await
        .map_err(failed("Could not load relation types"))?;
    let relation = args
        .opt_str("relation")
        .unwrap_or_else(|| "Relation".into());
    let kind = {
        let spec = MatchSpec {
            kind: "relation type",
            id: &|t: &Value| t["Id"].as_i64().unwrap_or_default(),
            name: &name_text,
            keys: &|t: &Value| vec![Some(name_text(t))],
            describe: None,
        };
        let candidates: Vec<&Value> = types.items.iter().collect();
        r#match(&relation, &spec, &candidates)
            .map_err(|r| unresolved(&r))?
            .clone()
    };
    let kind_id = kind.get("Id").and_then(num);
    let (master, slave) = if args.opt_str("direction").as_deref() == Some("inbound") {
        (to, card_id)
    } else {
        (card_id, to)
    };
    let existing = ctx
        .v1
        .list(
            "Relations",
            &list_options(
                "[Id,RelationType[Id,Name]]",
                Some(format!("(Master.Id eq {master}) and (Slave.Id eq {slave})")),
            ),
        )
        .await
        .map_err(failed("Could not read existing relations"))?;
    if let Some(same) = existing
        .items
        .iter()
        .find(|r| ref_id(&r["RelationType"]).map(|i| i as f64) == kind_id)
    {
        return success(
            Obj::new()
                .opt("relation", same.get("Id").and_then(num).map(number))
                .set("master", master)
                .set("slave", slave)
                .opt("type", kind.get("Name").cloned())
                .set("alreadyRelated", true)
                .value(),
        );
    }
    let body = json!({
        "Master": { "Id": master },
        "Slave": { "Id": slave },
        "RelationType": { "Id": kind_id.map(number) },
    });
    let options = WriteOptions {
        result_include: Some("[Id,Master[Id,Name],Slave[Id,Name],RelationType[Id,Name]]".into()),
        ..WriteOptions::default()
    };
    let r = ctx
        .v1
        .create("Relations", body, &options)
        .await
        .map_err(failed(format!("Could not relate {master} → {slave}")))?;
    let type_name = ref_name(&r["RelationType"])
        .map(Value::from)
        .or_else(|| kind.get("Name").cloned());
    success(
        Obj::new()
            .opt("relation", r.get("Id").and_then(num).map(number))
            .set(
                "master",
                v1_ref(&r["Master"]).map_or_else(|| json!(master), |m| ref_json(&m)),
            )
            .set(
                "slave",
                v1_ref(&r["Slave"]).map_or_else(|| json!(slave), |s| ref_json(&s)),
            )
            .opt("type", type_name)
            .value(),
    )
}

fn write_follow() -> Tool {
    Tool {
        name: "write_follow",
        description: "Follow (or with unfollow: true, stop following) a card, for yourself or another person.",
        fields: vec![id("id"), person("user").optional(), flag("unfollow")],
        handler: handler!(|ctx, args| { follow(ctx, args).await }),
    }
}

async fn follow(ctx: &Ctx, args: Args) -> Answer {
    let card_id = args.id("id");
    let user = resolve_person(
        ctx,
        &args.opt_str("user").unwrap_or_else(|| "me".into()),
        false,
    )
    .await
    .map_err(|r| unresolved(&r))?;
    let options = ListOptions {
        read: ReadOptions {
            include: Some("[Id]".into()),
            ..ReadOptions::default()
        },
        where_: Some(format!(
            "(General.Id eq {card_id}) and (User.Id eq {})",
            user.id
        )),
        ..ListOptions::default()
    };
    let rows = ctx
        .v1
        .list("GeneralFollowers", &options)
        .await
        .map_err(failed(format!("Could not read followers of {card_id}")))?
        .items;
    let who = who(&user);
    let want = !args.bool("unfollow");
    let out = Obj::new().set("card", card_id).set("user", who);
    if want == !rows.is_empty() {
        return success(out.set("following", want).set("changed", false).value());
    }
    if want {
        let body = json!({ "General": { "Id": card_id }, "User": { "Id": user.id } });
        ctx.v1
            .create("GeneralFollowers", body, &WriteOptions::default())
            .await
            .map_err(failed(format!("Could not follow {card_id}")))?;
    } else {
        for row in &rows {
            ctx.v1
                .delete("GeneralFollowers", row["Id"].as_i64().unwrap_or_default())
                .await
                .map_err(failed(format!("Could not unfollow {card_id}")))?;
        }
    }
    match ctx.v1.list("GeneralFollowers", &options).await {
        Err(e) => success(
            out.set("following", want)
                .set("changed", true)
                .set(
                    "warning",
                    format!("Could not read followers back ({})", e.message),
                )
                .value(),
        ),
        Ok(after) => {
            let following = !after.items.is_empty();
            let not_persisted = if following != want {
                vec![format!("following: requested {want}, got {following}")]
            } else {
                Vec::new()
            };
            success(
                out.set("following", following)
                    .set("changed", true)
                    .listed("notPersisted", not_persisted)
                    .value(),
            )
        }
    }
}

// ---------------------------------------------------------------- testing

fn write_test_cases() -> Tool {
    let step = Kind::Object(vec![field("step", nonempty()), text_param("expected")]);
    let case = Kind::Object(vec![
        field("name", nonempty()),
        text_param("description"),
        field("steps", list(step, None, None)).optional(),
    ]);
    Tool {
        name: "write_test_cases",
        description: "Create test cases with ordered steps under a test plan; each case is read back.",
        fields: vec![
            id("testPlan"),
            field("cases", list(case, Some(1), Some(100))),
        ],
        handler: handler!(|ctx, args| { test_cases(ctx, args).await }),
    }
}

/// A test plan by id, refused when the card is something else.
async fn test_plan(ctx: &Ctx, plan_id: i64) -> Result<CardInfo, ToolResult> {
    let plan = card_info(ctx, plan_id).await.map_err(|r| unresolved(&r))?;
    if plan.entity_type != "TestPlan" {
        return Err(invalid(format!(
            "{plan_id} is a {}, not a TestPlan",
            plan.entity_type
        )));
    }
    Ok(plan)
}

/// A non-empty string field of an argument object (JavaScript truthy).
fn given<'v>(value: &'v Value, key: &str) -> Option<&'v str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
}

async fn test_cases(ctx: &Ctx, args: Args) -> Answer {
    let plan_id = args.id("testPlan");
    let plan = test_plan(ctx, plan_id).await?;
    let Some(project) = &plan.project else {
        return Err(invalid(format!("Test plan {plan_id} has no project")));
    };
    let mut created: Vec<Value> = Vec::new();
    for c in args.opt_list("cases").into_iter().flatten() {
        let name = c["name"].as_str().unwrap_or_default();
        let steps: &[Value] = c["steps"].as_array().map_or(&[], Vec::as_slice);
        let mut body = Map::new();
        body.insert("Name".into(), json!(name));
        body.insert("Project".into(), json!({ "Id": project.id }));
        body.insert("TestPlans".into(), json!({ "Items": [{ "Id": plan_id }] }));
        if let Some(description) = given(c, "description") {
            body.insert("Description".into(), json!(text_to_html(description, None)));
        }
        if !steps.is_empty() {
            let step_items: Vec<Value> = steps
                .iter()
                .enumerate()
                .map(|(i, s)| {
                    let mut step = Map::new();
                    step.insert(
                        "Description".into(),
                        json!(text_to_html(s["step"].as_str().unwrap_or_default(), None)),
                    );
                    if let Some(expected) = given(s, "expected") {
                        step.insert("Result".into(), json!(text_to_html(expected, None)));
                    }
                    step.insert("RunOrder".into(), json!(i + 1));
                    Value::Object(step)
                })
                .collect();
            body.insert("TestSteps".into(), json!({ "Items": step_items }));
        }
        let options = WriteOptions {
            result_include: Some("[Id,Name]".into()),
            ..WriteOptions::default()
        };
        let r = match ctx
            .v1
            .create("TestCases", Value::Object(body), &options)
            .await
        {
            Ok(r) => r,
            Err(e) => {
                return Err(partial(
                    &format!("Could not create test case \"{name}\""),
                    &e,
                    created,
                ));
            }
        };
        let case_id = r.get("Id").and_then(Value::as_i64).unwrap_or_default();
        let read = ReadOptions {
            include: Some("[Id,Name,TestPlans[Id],TestSteps[Id]]".into()),
            inner_take: Some(1000),
            ..ReadOptions::default()
        };
        let back = ctx.v1.get("TestCases", case_id, &read).await;
        let mut not_persisted: Vec<String> = Vec::new();
        if let Ok(back) = &back {
            if !items(&back["TestPlans"])
                .iter()
                .any(|p| p["Id"].as_i64() == Some(plan_id))
            {
                not_persisted.push("not linked to the test plan".to_string());
            }
            let found = items(&back["TestSteps"]).len();
            if found != steps.len() {
                not_persisted.push(format!("steps: sent {}, found {found}", steps.len()));
            }
        }
        created.push(
            Obj::new()
                .set("id", case_id)
                .set("name", name)
                .set("steps", steps.len())
                .listed("notPersisted", not_persisted)
                .opt(
                    "warning",
                    back.err().map(|e| {
                        format!(
                            "created, but could not be read back to verify ({})",
                            e.message
                        )
                    }),
                )
                .value(),
        );
    }
    success(json!({
        "testPlan": { "id": plan_id, "name": plan.name },
        "created": created,
    }))
}

const RUN_STATUSES: [&str; 5] = ["Passed", "Failed", "Blocked", "OnHold", "NotRun"];

fn write_test_run() -> Tool {
    let result = Kind::Object(vec![
        id("testCase"),
        field("status", Kind::OneOf(&RUN_STATUSES)),
        text_param("comment"),
    ]);
    Tool {
        name: "write_test_run",
        description: "Record a run of a test plan, with a result per test case (Passed, Failed, Blocked, OnHold, NotRun).",
        fields: vec![
            id("testPlan"),
            text_param("name"),
            field("results", list(result, None, None)).optional(),
        ],
        handler: handler!(|ctx, args| { test_run(ctx, args).await }),
    }
}

async fn test_run(ctx: &Ctx, args: Args) -> Answer {
    let plan_id = args.id("testPlan");
    let plan = test_plan(ctx, plan_id).await?;
    let results: &[Value] = args.opt_list("results").map_or(&[], Vec::as_slice);
    let id_list = |include: &str, where_: Option<String>| ListOptions {
        read: ReadOptions {
            include: Some(include.into()),
            ..ReadOptions::default()
        },
        where_,
        limit: Some(20_000),
        ..ListOptions::default()
    };
    if !results.is_empty() {
        let cases = ctx
            .v1
            .list(
                &format!("TestPlans/{plan_id}/TestCases"),
                &id_list("[Id]", None),
            )
            .await
            .map_err(failed(format!(
                "Could not read the test cases of test plan {plan_id}"
            )))?;
        let in_plan: Vec<f64> = cases.items.iter().filter_map(|c| num(&c["Id"])).collect();
        let foreign: Vec<String> = results
            .iter()
            .filter_map(|r| r["testCase"].as_i64())
            .filter(|tc| !in_plan.contains(&(*tc as f64)))
            .map(|tc| tc.to_string())
            .collect();
        if !foreign.is_empty() {
            return Err(invalid(format!(
                "test cases {} are not in test plan {plan_id}; no run was started",
                foreign.join(", ")
            )));
        }
    }
    let mut body = Map::new();
    body.insert("TestPlan".into(), json!({ "Id": plan_id }));
    if let Some(name) = args.opt_str("name").filter(|n| !n.is_empty()) {
        body.insert("Name".into(), json!(name));
    }
    if let Some(project) = &plan.project {
        body.insert("Project".into(), json!({ "Id": project.id }));
    }
    let options = WriteOptions {
        result_include: Some("[Id,Name]".into()),
        ..WriteOptions::default()
    };
    let run = ctx
        .v1
        .create("TestPlanRuns", Value::Object(body), &options)
        .await
        .map_err(failed(format!(
            "Could not start a run of test plan {plan_id}"
        )))?;
    let run_id = run.get("Id").and_then(Value::as_i64).unwrap_or_default();
    let done = vec![json!({ "testPlanRun": run_id })];
    // Read the generated case runs fully paged: an inner collection in the response would stop at 25.
    let case_runs = match ctx
        .v1
        .list(
            "TestCaseRuns",
            &id_list(
                "[Id,TestCase[Id,Name]]",
                Some(format!("(TestPlanRun.Id eq {run_id})")),
            ),
        )
        .await
    {
        Ok(list) => list.items,
        Err(e) => {
            return Err(partial(
                &format!("Started run {run_id} but could not read its test case runs"),
                &e,
                done,
            ));
        }
    };
    let mut recorded: Vec<Value> = Vec::new();
    let mut missing: Vec<i64> = Vec::new();
    for res in results {
        let test_case = res["testCase"].as_i64().unwrap_or_default();
        let status = res["status"].as_str().unwrap_or_default();
        let Some(cr) = case_runs
            .iter()
            .find(|c| ref_id(&c["TestCase"]) == Some(test_case))
        else {
            missing.push(test_case);
            continue;
        };
        let mut update = Map::new();
        update.insert("Status".into(), json!(status));
        if let Some(comment) = given(res, "comment") {
            update.insert("Comment".into(), json!(comment));
        }
        let options = WriteOptions {
            result_include: Some("[Id,Status,Comment]".into()),
            ..WriteOptions::default()
        };
        let r = match ctx
            .v1
            .update(
                "TestCaseRuns",
                cr["Id"].as_i64().unwrap_or_default(),
                update,
                &options,
            )
            .await
        {
            Ok(r) => r,
            Err(e) => {
                let mut so_far = done;
                so_far.extend(recorded);
                return Err(partial(
                    &format!("Could not record {status} for test case {test_case}"),
                    &e,
                    so_far,
                ));
            }
        };
        let got = r.get("Status").filter(|s| !s.is_null());
        let differs = got.filter(|s| is_truthy(s) && s.as_str() != Some(status));
        recorded.push(
            Obj::new()
                .set("testCase", test_case)
                .set("status", got.cloned().unwrap_or_else(|| json!(status)))
                .opt(
                    "notPersisted",
                    differs.map(|s| format!("status: got {}", js_string(s))),
                )
                .value(),
        );
    }
    let mut out = Obj::new()
        .set(
            "testPlanRun",
            Obj::new()
                .set("id", run_id)
                .opt("name", run.get("Name").cloned())
                .value(),
        )
        .set("caseRuns", case_runs.len())
        .set("recorded", recorded);
    if !missing.is_empty() {
        out = out.set("notInRun", missing);
    }
    success(out.value())
}

/// JavaScript truthiness of a JSON value.
fn is_truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Value::String(s) => !s.is_empty(),
        _ => true,
    }
}

/// In the order of the TypeScript export (`workflowWriteTools`), which `tools/list` pins.
pub fn tools() -> Vec<Tool> {
    vec![
        write_create_card(),
        write_update_card(),
        write_set_state(),
        write_assign(),
        write_unassign(),
        write_set_role_effort(),
        write_set_custom_fields(),
        write_team(),
        write_comment(),
        write_log_time(),
        write_relate(),
        write_follow(),
        write_test_cases(),
        write_test_run(),
    ]
}
