//! Workflow read tools: cards, search, people, teams, projects, states, iterations,
//! custom fields, comments, relations, time, attachments, test plans.

use std::cmp::Ordering;
use std::sync::LazyLock;

use regex::Regex;
use serde_json::{Map, Value, json};

use super::common::{type_scope, unresolved};
use crate::catalog::{Request, js_trim, locale_compare, suggestions};
use crate::domain::cards::{parent_chain, read_card_raw, shape_card};
use crate::domain::time::{TimeBackend, TimeRecordBackend, time_backend, time_record_backend};
use crate::format::shape::{
    card_summary, compact, inner_cap_note, items, num, number, person, reference, text,
};
use crate::http::v1::{ListOptions, ReadOptions, v1_string};
use crate::resolve::card::card_info;
use crate::resolve::directory::{
    V1Ref, custom_field_options, full_name, is_active_user, is_team_workflow_state,
};
use crate::resolve::matching::{MatchSpec, r#match, norm};
use crate::schema::{Field, Kind, field, limit, pattern, string};
use crate::tools::generic::validate::{js_number, js_string};
use crate::tools::params::{id, name_or_id};
use crate::tools::{Ctx, Tool, failed, handler, invalid, success};

/// An optional plain string parameter.
fn text_param(name: &'static str) -> Field {
    field(name, string()).optional()
}

fn flag(name: &'static str) -> Field {
    field(name, Kind::Bool).optional()
}

/// `ref(...)` of a directory reference: `{id, name}`, the name only when known.
fn v1_ref_json(r: Option<&V1Ref>) -> Value {
    match r {
        Some(V1Ref { id, name: Some(n) }) => json!({ "id": id, "name": n }),
        Some(V1Ref { id, name: None }) => json!({ "id": id }),
        None => Value::Null,
    }
}

/// `ref(value)?.name`.
fn ref_name(value: &Value) -> Value {
    reference(value).get("name").cloned().unwrap_or(Value::Null)
}

/// `cond || undefined` for a flag.
fn true_or_null(cond: bool) -> Value {
    if cond { json!(true) } else { Value::Null }
}

/// `a ?? b` on JSON values.
fn or_else(a: Value, b: impl FnOnce() -> Value) -> Value {
    if a.is_null() { b() } else { a }
}

/// `Math.round(x * 100) / 100`, rounding halves up as JavaScript does.
fn round2(x: f64) -> Value {
    let scaled = x * 100.0;
    let floor = scaled.floor();
    let rounded = if scaled - floor >= 0.5 {
        floor + 1.0
    } else {
        floor
    };
    number(rounded / 100.0)
}

fn limit_of(args: &crate::schema::Args, default: usize) -> usize {
    args.opt_num("limit").map_or(default, |n| n as usize)
}

fn list(
    where_: Option<String>,
    include: &str,
    order_by_desc: Option<&str>,
    limit: Option<usize>,
) -> ListOptions {
    ListOptions {
        read: ReadOptions {
            include: Some(include.to_string()),
            ..ReadOptions::default()
        },
        where_,
        order_by: None,
        order_by_desc: order_by_desc.map(str::to_string),
        limit,
    }
}

fn read_card() -> Tool {
    Tool {
        name: "read_card",
        description: "Read a card by id: state, project, parents, teams, assignments, role efforts, custom fields, tags, description.",
        fields: vec![id("id")],
        handler: handler!(|ctx, args| {
            let card = read_card_raw(ctx, args.id("id"))
                .await
                .map_err(|p| p.to_result())?;
            let parents = parent_chain(ctx, &card.raw, 5).await;
            let mut out = shape_card(&card.info, &card.raw, Some(&parents));
            if let Value::Object(o) = &mut out {
                o.extend(inner_cap_note(&card.raw));
            }
            success(out)
        }),
    }
}

#[derive(Default)]
struct SearchArgs {
    text: Option<String>,
    type_name: Option<String>,
    state: Option<String>,
    project: Option<String>,
    assignee: Option<String>,
    /// An already resolved assignee (read_my_work), so the user directory is not loaded again.
    assignee_id: Option<i64>,
    tag: Option<String>,
    include_closed: bool,
    limit: usize,
}

static HASH_ID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^#[0-9]+$").expect("valid pattern"));
static DIGITS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[0-9]+$").expect("valid pattern"));

async fn search_cards(
    ctx: &Ctx,
    args: SearchArgs,
    extra: Map<String, Value>,
) -> crate::tools::Answer {
    let type_name = args.type_name.as_deref().unwrap_or("Assignable");
    let catalog = ctx
        .catalog
        .get(&Request::Names(vec![type_name.to_string()]))
        .await;
    let r = catalog.resource(type_name).map_err(invalid)?.clone();
    let has = |m: &str| catalog.member(&r, m).is_some();
    let mut where_: Vec<String> = Vec::new();

    let t = args.text.as_deref().map(js_trim);
    // "#123" is an id; bare digits are searched both as an id and as name text.
    let id_text = t
        .filter(|t| HASH_ID.is_match(t))
        .map(|t| t[1..].parse::<f64>().unwrap_or(f64::NAN));
    let also_id = t
        .filter(|t| DIGITS.is_match(t))
        .map(|t| t.parse::<f64>().unwrap_or(f64::NAN));
    if let Some(n) = id_text {
        where_.push(format!("(Id eq {})", js_number(n)));
    } else if let Some(t) = t.filter(|t| !t.is_empty()) {
        where_.push(format!("(Name contains {})", v1_string(t)));
    }
    let state = args.state.as_deref().filter(|s| !s.is_empty());
    if let Some(state) = state {
        if !has("EntityState") {
            return Err(invalid(format!("{} has no state", r.name)));
        }
        let scope =
            (!["Assignable", "General"].contains(&r.name.as_str())).then_some(r.name.as_str());
        let names = ctx
            .directory
            .state_names(scope)
            .await
            .map_err(failed("Could not load state names"))?;
        let wanted = norm(state);
        let Some(hit) = names.iter().find(|n| norm(n) == wanted) else {
            let close = suggestions(state, &names, 6);
            return Err(invalid(format!(
                "No {}state is named \"{state}\".{}",
                if r.name == "Assignable" {
                    String::new()
                } else {
                    format!("{} ", r.name)
                },
                if close.is_empty() {
                    String::new()
                } else {
                    format!(" Did you mean: {}?", close.join(", "))
                }
            )));
        };
        where_.push(format!("(EntityState.Name eq {})", v1_string(hit)));
    }
    if let Some(project) = &args.project {
        let project = ctx
            .directory
            .project(project)
            .await
            .map_err(|r| unresolved(&r))?;
        where_.push(format!("(Project.Id eq {})", project.id));
    }
    if args.assignee.is_some() || args.assignee_id.is_some() {
        if !has("AssignedUser") {
            return Err(invalid(format!(
                "{} cannot be filtered by assignee",
                r.name
            )));
        }
        let assignee_id = match (args.assignee_id, &args.assignee) {
            (Some(id), _) => id,
            (None, Some(a)) => {
                let user = if a.to_lowercase() == "me" {
                    ctx.directory.me().await
                } else {
                    ctx.directory.user(a, true).await
                };
                user.map_err(|r| unresolved(&r))?.id
            }
            (None, None) => unreachable!("checked above"),
        };
        where_.push(format!("(AssignedUser.Id eq {assignee_id})"));
    }
    if let Some(tag) = args.tag.as_deref().filter(|t| !t.is_empty()) {
        if !has("TagObjects") {
            return Err(invalid(format!("{} has no tags", r.name)));
        }
        where_.push(format!("(TagObjects.Name eq {})", v1_string(tag)));
    }
    if !args.include_closed && has("EntityState") && state.is_none() {
        where_.push("(EntityState.IsFinal eq 'false')".into());
    }

    let include: Vec<&str> = [
        "Id",
        "Name",
        "EntityType[Name]",
        "EntityState[Name]",
        "Project[Name]",
        "Tags",
        "ModifyDate",
    ]
    .into_iter()
    .filter(|f| has(f.split('[').next().unwrap_or(f)))
    .collect();
    let include = format!("[{}]", include.join(","));
    let order = has("ModifyDate").then_some("ModifyDate");
    let query = |clauses: Vec<&String>| {
        let where_ = (!clauses.is_empty()).then(|| {
            clauses
                .iter()
                .map(|c| c.as_str())
                .collect::<Vec<_>>()
                .join(" and ")
        });
        list(where_, &include, order, Some(args.limit))
    };
    let result = ctx
        .v1
        .list(&r.path, &query(where_.iter().collect()))
        .await
        .map_err(failed("Search failed"))?;
    let truncated = result.truncated;
    let mut found = result.items;
    if let Some(also) = also_id {
        let first = format!("(Id eq {})", js_number(also));
        let mut clauses = vec![&first];
        clauses.extend(where_.iter().filter(|w| !w.starts_with("(Name contains")));
        let by_id = ctx
            .v1
            .list(&r.path, &query(clauses))
            .await
            .map_err(failed("Search by id failed"))?;
        let mut merged = by_id.items;
        merged.extend(
            found
                .into_iter()
                .filter(|c| c.get("Id").and_then(Value::as_f64) != Some(also)),
        );
        found = merged;
    }
    let cards: Vec<Value> = found.iter().map(card_summary).collect();
    let mut by_state: Map<String, Value> = Map::new();
    for c in &cards {
        if let Some(s) = c.get("state").and_then(Value::as_str) {
            let n = by_state.get(s).and_then(Value::as_i64).unwrap_or(0);
            by_state.insert(s.to_string(), json!(n + 1));
        }
    }
    let mut out = extra;
    out.insert("count".into(), json!(cards.len()));
    out.insert("truncated".into(), json!(truncated));
    out.insert("byState".into(), Value::Object(by_state));
    if truncated {
        out.insert(
            "totalsPartial".into(),
            json!("byState counts only the returned cards"),
        );
    }
    out.insert("cards".into(), Value::Array(cards));
    success(Value::Object(out))
}

fn read_search() -> Tool {
    Tool {
        name: "read_search",
        description: "Find cards by name text or #id, type, state, project, assignee and tag; open cards only unless includeClosed.",
        fields: vec![
            text_param("text").describe(
                "Text in the card name, or a #id (bare digits match the id and the name)",
            ),
            text_param("type").describe(
                "Entity type, e.g. UserStory, Task, Bug, Feature (default: any assignable card)",
            ),
            text_param("state").describe("State name, e.g. \"In Progress\""),
            name_or_id("project")
                .optional()
                .describe("Project name, abbreviation or id"),
            name_or_id("assignee")
                .optional()
                .describe("Person name, login, email, id, or \"me\""),
            text_param("tag"),
            flag("includeClosed"),
            limit("limit", 50, 1000),
        ],
        handler: handler!(|ctx, args| {
            let search = SearchArgs {
                text: args.opt_str("text"),
                type_name: args.opt_str("type"),
                state: args.opt_str("state"),
                project: args.opt_str("project"),
                assignee: args.opt_str("assignee"),
                assignee_id: None,
                tag: args.opt_str("tag"),
                include_closed: args.bool("includeClosed"),
                limit: limit_of(&args, 50),
            };
            search_cards(ctx, search, Map::new()).await
        }),
    }
}

fn read_my_work() -> Tool {
    Tool {
        name: "read_my_work",
        description: "Cards assigned to the current user (the token owner), open ones by default, with counts per state.",
        fields: vec![
            text_param("type")
                .describe("Entity type, e.g. Task or Bug (default: any assignable card)"),
            text_param("state"),
            flag("includeClosed"),
            limit("limit", 100, 1000),
        ],
        handler: handler!(|ctx, args| {
            let me = ctx.directory.me().await.map_err(|r| unresolved(&r))?;
            let search = SearchArgs {
                type_name: args.opt_str("type"),
                state: args.opt_str("state"),
                assignee_id: Some(me.id),
                include_closed: args.bool("includeClosed"),
                limit: limit_of(&args, 100),
                ..SearchArgs::default()
            };
            let mut extra = Map::new();
            extra.insert(
                "user".into(),
                json!({ "id": me.id, "name": full_name(&me) }),
            );
            search_cards(ctx, search, extra).await
        }),
    }
}

fn read_states() -> Tool {
    Tool {
        name: "read_states",
        description: "States of a card's own workflow, or of a type in a project; team workflow states are flagged isTeamWorkflow.",
        fields: vec![
            id("id").optional().describe("Card id"),
            name_or_id("project")
                .optional()
                .describe("Project (when no card id)"),
            text_param("type").describe("Entity type (when no card id), e.g. UserStory"),
        ],
        handler: handler!(|ctx, args| {
            let s = type_scope(
                ctx,
                args.opt_id("id"),
                args.opt_str("project").as_deref(),
                args.opt_str("type").as_deref(),
            )
            .await?;
            let states = ctx
                .directory
                .states(s.process_id, &s.entity_type)
                .await
                .map_err(failed(format!("Could not load {} states", s.entity_type)))?;
            let mut current = Value::Null;
            let mut team_states = Value::Null;
            if let Some(card_id) = s.card_id {
                let catalog = ctx.catalog.get(&Request::Names(Vec::new())).await;
                let teams_include = if catalog.member(&s.resource, "AssignedTeams").is_some() {
                    ",AssignedTeams[Id,Team[Id,Name],EntityState[Id,Name]]"
                } else {
                    ""
                };
                let options = ReadOptions {
                    include: Some(format!("[Id,EntityState[Id,Name]{teams_include}]")),
                    ..ReadOptions::default()
                };
                if let Ok(card) = ctx.v1.get(&s.resource.path, card_id, &options).await {
                    current = reference(&card["EntityState"]);
                    let teams = items(&card["AssignedTeams"]);
                    if !teams.is_empty() {
                        team_states = Value::Array(
                            teams
                                .iter()
                                .map(|t| {
                                    json!({ "team": reference(&t["Team"]), "state": reference(&t["EntityState"]) })
                                })
                                .collect(),
                        );
                    }
                }
            }
            let list: Vec<Value> = states
                .iter()
                .map(|st| {
                    compact([
                        ("id", json!(st.id)),
                        ("name", json!(st.name)),
                        ("initial", true_or_null(st.is_initial == Some(true))),
                        ("final", true_or_null(st.is_final == Some(true))),
                        ("planned", true_or_null(st.is_planned == Some(true))),
                        (
                            "role",
                            json!(st.role.as_ref().and_then(|r| r.name.as_deref())),
                        ),
                        (
                            "workflow",
                            json!(st.workflow.as_ref().and_then(|w| w.name.as_deref())),
                        ),
                        ("isTeamWorkflow", true_or_null(is_team_workflow_state(st))),
                        (
                            "parentState",
                            json!(
                                st.parent_entity_state
                                    .as_ref()
                                    .and_then(|p| p.name.as_deref())
                            ),
                        ),
                    ])
                })
                .collect();
            success(compact([
                ("entityType", json!(s.entity_type)),
                (
                    "project",
                    s.project.as_ref().map_or(Value::Null, |p| p.to_json()),
                ),
                ("processId", json!(s.process_id)),
                ("card", json!(s.card_id)),
                ("current", current),
                ("teamStates", team_states),
                ("states", Value::Array(list)),
            ]))
        }),
    }
}

fn read_people() -> Tool {
    Tool {
        name: "read_people",
        description: "Find people by name, login or email (every word must match); without a query, everyone active.",
        fields: vec![
            text_param("query"),
            flag("includeInactive"),
            limit("limit", 100, 5000),
        ],
        handler: handler!(|ctx, args| {
            let users = ctx
                .directory
                .users()
                .await
                .map_err(failed("Could not load users"))?;
            let include_inactive = args.bool("includeInactive");
            let mut pool: Vec<_> = users
                .iter()
                .filter(|u| include_inactive || is_active_user(u))
                .collect();
            if let Some(query) = args.opt_str("query").filter(|q| !js_trim(q).is_empty()) {
                let normalized = norm(&query);
                let words: Vec<&str> = normalized.split(' ').collect();
                pool.retain(|u| {
                    let parts: Vec<String> = [Some(full_name(u)), u.login.clone(), u.email.clone()]
                        .into_iter()
                        .flatten()
                        .filter(|p| !p.is_empty())
                        .collect();
                    let hay = norm(&parts.join(" "));
                    words.iter().all(|w| hay.contains(w))
                });
            }
            let limit = limit_of(&args, 100);
            let people: Vec<Value> = pool
                .iter()
                .take(limit)
                .map(|u| {
                    compact([
                        ("id", json!(u.id)),
                        ("name", json!(full_name(u))),
                        ("login", json!(u.login)),
                        ("email", json!(u.email)),
                        (
                            "role",
                            json!(u.role.as_ref().and_then(|r| r.name.as_deref())),
                        ),
                        ("inactive", true_or_null(!is_active_user(u))),
                    ])
                })
                .collect();
            success(json!({
                "count": pool.len().min(limit),
                "truncated": pool.len() > limit,
                "people": people,
            }))
        }),
    }
}

/// The normalized query, or `None` when it is absent or blank.
fn name_query(query: Option<String>) -> Option<String> {
    query.filter(|q| !js_trim(q).is_empty()).map(|q| norm(&q))
}

fn read_teams() -> Tool {
    Tool {
        name: "read_teams",
        description: "List teams (active by default), optionally filtered by name.",
        fields: vec![text_param("query"), flag("includeInactive")],
        handler: handler!(|ctx, args| {
            let teams = ctx
                .directory
                .teams()
                .await
                .map_err(failed("Could not load teams"))?;
            let include_inactive = args.bool("includeInactive");
            let q = name_query(args.opt_str("query"));
            let list: Vec<Value> = teams
                .iter()
                .filter(|t| include_inactive || t.is_active != Some(false))
                .filter(|t| {
                    q.as_ref()
                        .is_none_or(|q| norm(&t.name).contains(q.as_str()))
                })
                .map(|t| {
                    compact([
                        ("id", json!(t.id)),
                        ("name", json!(t.name)),
                        ("inactive", true_or_null(t.is_active == Some(false))),
                    ])
                })
                .collect();
            success(json!({ "count": list.len(), "teams": list }))
        }),
    }
}

fn read_roles() -> Tool {
    Tool {
        name: "read_roles",
        description: "List roles; hasEffort tells whether a role can carry effort.",
        fields: Vec::new(),
        handler: handler!(|ctx, args| {
            let roles = ctx
                .directory
                .roles()
                .await
                .map_err(failed("Could not load roles"))?;
            let list: Vec<Value> = roles
                .iter()
                .map(|r| json!({ "id": r.id, "name": r.name, "hasEffort": r.has_effort != Some(false) }))
                .collect();
            success(json!({ "count": list.len(), "roles": list }))
        }),
    }
}

fn read_projects() -> Tool {
    Tool {
        name: "read_projects",
        description: "List projects (active by default) with abbreviation and process.",
        fields: vec![text_param("query"), flag("includeInactive")],
        handler: handler!(|ctx, args| {
            let projects = ctx
                .directory
                .projects()
                .await
                .map_err(failed("Could not load projects"))?;
            let include_inactive = args.bool("includeInactive");
            let q = name_query(args.opt_str("query"));
            let list: Vec<Value> = projects
                .iter()
                .filter(|p| include_inactive || p.is_active != Some(false))
                .filter(|p| {
                    q.as_ref().is_none_or(|q| {
                        norm(&p.name).contains(q.as_str())
                            || norm(p.abbreviation.as_deref().unwrap_or("")) == *q
                    })
                })
                .map(|p| {
                    compact([
                        ("id", json!(p.id)),
                        ("name", json!(p.name)),
                        ("abbreviation", json!(p.abbreviation)),
                        ("process", v1_ref_json(p.process.as_ref())),
                        ("inactive", true_or_null(p.is_active == Some(false))),
                    ])
                })
                .collect();
            success(json!({ "count": list.len(), "projects": list }))
        }),
    }
}

fn dated(raw: &Value) -> Value {
    compact([
        ("id", num(&raw["Id"])),
        ("name", raw["Name"].clone()),
        ("start", raw["StartDate"].clone()),
        ("end", raw["EndDate"].clone()),
        ("current", true_or_null(raw["IsCurrent"] == json!(true))),
        ("project", reference(&raw["Project"])),
        ("team", reference(&raw["Team"])),
    ])
}

fn read_releases() -> Tool {
    Tool {
        name: "read_releases",
        description: "Releases, newest first, optionally for one project.",
        fields: vec![
            name_or_id("project").optional(),
            flag("currentOnly"),
            limit("limit", 50, 1000),
        ],
        handler: handler!(|ctx, args| {
            let mut where_: Vec<String> = Vec::new();
            if let Some(project) = args.opt_str("project") {
                let project = ctx
                    .directory
                    .project(&project)
                    .await
                    .map_err(|r| unresolved(&r))?;
                where_.push(format!("(Project.Id eq {})", project.id));
            }
            if args.bool("currentOnly") {
                where_.push("(IsCurrent eq 'true')".into());
            }
            let options = list(
                (!where_.is_empty()).then(|| where_.join(" and ")),
                "[Id,Name,StartDate,EndDate,IsCurrent,Project[Id,Name]]",
                Some("StartDate"),
                Some(limit_of(&args, 50)),
            );
            let r = ctx
                .v1
                .list("Releases", &options)
                .await
                .map_err(failed("Could not load releases"))?;
            success(json!({
                "count": r.items.len(),
                "truncated": r.truncated,
                "releases": r.items.iter().map(dated).collect::<Vec<_>>(),
            }))
        }),
    }
}

fn read_iterations() -> Tool {
    Tool {
        name: "read_iterations",
        description: "Team or project iterations (sprints), newest first; without team or project, every team's current ones.",
        fields: vec![
            name_or_id("team").optional(),
            name_or_id("project").optional(),
            flag("currentOnly"),
            limit("limit", 50, 1000),
        ],
        handler: handler!(|ctx, args| {
            let team = args.opt_str("team");
            let project = args.opt_str("project");
            if project.is_some() && team.is_some() {
                return Err(invalid(
                    "pass team (team iterations) or project (project iterations), not both",
                ));
            }
            let mut where_: Vec<String> = Vec::new();
            let mut path = "TeamIterations";
            let mut include = "[Id,Name,StartDate,EndDate,IsCurrent,Team[Id,Name]]";
            if let Some(project) = &project {
                let project = ctx
                    .directory
                    .project(project)
                    .await
                    .map_err(|r| unresolved(&r))?;
                path = "Iterations";
                include = "[Id,Name,StartDate,EndDate,IsCurrent,Project[Id,Name]]";
                where_.push(format!("(Project.Id eq {})", project.id));
            } else if let Some(team) = &team {
                let team = ctx
                    .directory
                    .team(team, true)
                    .await
                    .map_err(|r| unresolved(&r))?;
                where_.push(format!("(Team.Id eq {})", team.id));
            }
            if args.bool("currentOnly") || (team.is_none() && project.is_none()) {
                where_.push("(IsCurrent eq 'true')".into());
            }
            let options = list(
                Some(where_.join(" and ")),
                include,
                Some("StartDate"),
                Some(limit_of(&args, 50)),
            );
            let r = ctx
                .v1
                .list(path, &options)
                .await
                .map_err(failed(format!("Could not load {path}")))?;
            success(json!({
                "kind": path,
                "count": r.items.len(),
                "truncated": r.truncated,
                "iterations": r.items.iter().map(dated).collect::<Vec<_>>(),
            }))
        }),
    }
}

fn read_custom_field_options() -> Tool {
    Tool {
        name: "read_custom_field_options",
        description: "Custom fields of a card's type, or of a type in a project, with the allowed dropdown values.",
        fields: vec![
            id("id").optional(),
            name_or_id("project").optional(),
            text_param("type"),
            text_param("field"),
        ],
        handler: handler!(|ctx, args| {
            let s = type_scope(
                ctx,
                args.opt_id("id"),
                args.opt_str("project").as_deref(),
                args.opt_str("type").as_deref(),
            )
            .await?;
            let fields = ctx
                .directory
                .custom_fields(s.process_id, &s.entity_type)
                .await
                .map_err(failed(format!(
                    "Could not load {} custom fields",
                    s.entity_type
                )))?;
            let mut list: Vec<_> = fields.iter().collect();
            if let Some(wanted) = args.opt_str("field").filter(|f| !f.is_empty()) {
                let kind = format!("{} custom field", s.entity_type);
                let spec = MatchSpec {
                    kind: &kind,
                    id: &|f: &crate::resolve::directory::TpCustomField| f.id,
                    name: &|f: &crate::resolve::directory::TpCustomField| f.name.clone(),
                    keys: &|f: &crate::resolve::directory::TpCustomField| {
                        vec![Some(f.name.clone())]
                    },
                    describe: None,
                };
                let one = r#match(&wanted, &spec, &list).map_err(|r| unresolved(&r))?;
                list = vec![one];
            }
            let shown: Vec<Value> = list
                .iter()
                .map(|f| {
                    compact([
                        ("id", json!(f.id)),
                        ("name", json!(f.name)),
                        ("type", json!(f.field_type)),
                        ("required", true_or_null(f.required == Some(true))),
                        ("options", json!(custom_field_options(f))),
                    ])
                })
                .collect();
            success(json!({
                "entityType": s.entity_type,
                "processId": s.process_id,
                "fields": shown,
            }))
        }),
    }
}

fn read_comments() -> Tool {
    Tool {
        name: "read_comments",
        description: "Comments on a card as plain text, oldest first, with author and reply parent.",
        fields: vec![id("id"), limit("limit", 200, 5000)],
        handler: handler!(|ctx, args| {
            let card = args.id("id");
            // Newest first so a cap drops the oldest, then shown oldest first.
            let options = list(
                Some(format!("(General.Id eq {card})")),
                "[Id,Description,CreateDate,Owner[Id,FirstName,LastName,Login],ParentId,IsPrivate,IsPinned]",
                Some("CreateDate"),
                Some(limit_of(&args, 200)),
            );
            let r = ctx
                .v1
                .list("Comments", &options)
                .await
                .map_err(failed(format!("Could not read comments of {card}")))?;
            let comments: Vec<Value> = r
                .items
                .iter()
                .rev()
                .map(|c| {
                    compact([
                        ("id", num(&c["Id"])),
                        ("author", person(&c["Owner"])),
                        ("date", c["CreateDate"].clone()),
                        ("replyTo", num(&c["ParentId"])),
                        ("private", true_or_null(c["IsPrivate"] == json!(true))),
                        ("pinned", true_or_null(c["IsPinned"] == json!(true))),
                        ("text", or_else(text(&c["Description"]), || json!(""))),
                    ])
                })
                .collect();
            success(json!({
                "card": card,
                "count": r.items.len(),
                "truncated": r.truncated,
                "comments": comments,
            }))
        }),
    }
}

fn relation_end(raw: &Value) -> Value {
    if !raw.is_object() {
        return Value::Null;
    }
    compact([
        ("id", num(&raw["Id"])),
        (
            "type",
            or_else(raw["EntityType"]["Name"].clone(), || {
                raw["ResourceType"].clone()
            }),
        ),
        ("name", raw["Name"].clone()),
    ])
}

fn read_relations() -> Tool {
    Tool {
        name: "read_relations",
        description: "Relations of a card in both directions, with the related card.",
        fields: vec![id("id")],
        handler: handler!(|ctx, args| {
            let card = args.id("id");
            let include = "[Id,RelationType[Id,Name],Master[Id,Name,EntityType[Name]],Slave[Id,Name,EntityType[Name]]]";
            let out_options = list(Some(format!("(Master.Id eq {card})")), include, None, None);
            let in_options = list(Some(format!("(Slave.Id eq {card})")), include, None, None);
            let (out, inn) = tokio::join!(
                ctx.v1.list("Relations", &out_options),
                ctx.v1.list("Relations", &in_options),
            );
            let summary = format!("Could not read relations of {card}");
            let out = out.map_err(failed(summary.clone()))?;
            let inn = inn.map_err(failed(summary))?;
            let row = |r: &Value, direction: &str, end: &str| {
                compact([
                    ("id", num(&r["Id"])),
                    ("relation", ref_name(&r["RelationType"])),
                    ("direction", json!(direction)),
                    ("card", relation_end(&r[end])),
                ])
            };
            let relations: Vec<Value> = out
                .items
                .iter()
                .map(|r| row(r, "outbound", "Slave"))
                .chain(inn.items.iter().map(|r| row(r, "inbound", "Master")))
                .collect();
            success(json!({
                "card": card,
                "count": relations.len(),
                "truncated": out.truncated || inn.truncated,
                "relations": relations,
            }))
        }),
    }
}

fn time_entry(t: &Value) -> Value {
    compact([
        ("id", num(&t["Id"])),
        ("kind", json!("Time")),
        ("date", t["Date"].clone()),
        ("spent", num(&t["Spent"])),
        ("remain", num(&t["Remain"])),
        ("user", person(&t["User"])),
        ("role", reference(&t["Role"])),
        ("card", reference(&t["Assignable"])),
        ("description", text(&t["Description"])),
    ])
}

fn record_entry(r: &Value, backend: &TimeRecordBackend) -> Value {
    let fields = r["CustomFields"].as_array();
    let value = |name: Option<&str>| -> Value {
        name.and_then(|name| {
            fields?
                .iter()
                .find(|f| f["Name"].as_str() == Some(name))
                .map(|f| f["Value"].clone())
        })
        .unwrap_or(Value::Null)
    };
    let card = backend
        .card_refs
        .iter()
        .map(|k| reference(&r[k.as_str()]))
        .find(|v| !v.is_null())
        .unwrap_or(Value::Null);
    compact([
        ("id", num(&r["Id"])),
        ("kind", json!(backend.resource.name)),
        ("date", value(backend.date.as_deref())),
        ("spent", num(&value(Some(&backend.hours)))),
        ("user", person(&r["ConnectedUser"])),
        ("card", card),
        ("name", r["Name"].clone()),
        ("description", text(&r["Description"])),
    ])
}

fn read_times() -> Tool {
    Tool {
        name: "read_times",
        description: "Time logged on a card or by a person, optionally between dates, with the total spent.",
        fields: vec![
            id("id").optional().describe("Card id"),
            name_or_id("user").optional().describe("Person, or \"me\""),
            field("from", pattern(r"^\d{4}-\d{2}-\d{2}$", None))
                .optional()
                .describe("YYYY-MM-DD"),
            field("to", pattern(r"^\d{4}-\d{2}-\d{2}$", None))
                .optional()
                .describe("YYYY-MM-DD"),
            limit("limit", 500, 5000),
        ],
        handler: handler!(|ctx, args| {
            let card = args.opt_id("id");
            let user = args.opt_str("user");
            if card.is_none() && user.is_none() {
                return Err(invalid("pass a card id or a user"));
            }
            let mut user_id = None;
            if let Some(user) = &user {
                let found = if user.to_lowercase() == "me" {
                    ctx.directory.me().await
                } else {
                    ctx.directory.user(user, true).await
                };
                user_id = Some(found.map_err(|r| unresolved(&r))?.id);
            }
            let limit = limit_of(&args, 500);
            let from = args.opt_str("from").filter(|s| !s.is_empty());
            let to = args.opt_str("to").filter(|s| !s.is_empty());

            // Which kinds to read: by card, the card's process decides; by person, every kind the instance has.
            let mut read_time = true;
            let mut record: Option<TimeRecordBackend> = None;
            let mut card_ref: Option<String> = None;
            if let Some(card) = card {
                let info = card_info(ctx, card).await.map_err(|r| unresolved(&r))?;
                let backend = time_backend(ctx, info.process_id)
                    .await
                    .map_err(|p| p.to_result())?;
                if let TimeBackend::TimeRecord(backend) = backend {
                    read_time = false;
                    card_ref = backend
                        .card_refs
                        .iter()
                        .find(|r| **r == info.entity_type)
                        .cloned();
                    if card_ref.is_none() {
                        return Err(invalid(format!(
                            "{} entries do not link to a {}",
                            backend.resource.name, info.entity_type
                        )));
                    }
                    record = Some(backend);
                }
            } else {
                record = time_record_backend(ctx)
                    .await
                    .map_err(|p| crate::tools::failure(&p.message, p.error.as_ref(), None))?;
            }

            let mut entries: Vec<Value> = Vec::new();
            let mut truncated = false;
            if read_time {
                let mut where_: Vec<String> = Vec::new();
                if let Some(card) = card {
                    where_.push(format!("(Assignable.Id eq {card})"));
                }
                if let Some(u) = user_id {
                    where_.push(format!("(User.Id eq {u})"));
                }
                if let Some(from) = &from {
                    where_.push(format!("(Date gte '{from}')"));
                }
                if let Some(to) = &to {
                    where_.push(format!("(Date lte '{to}')"));
                }
                let options = list(
                    Some(where_.join(" and ")),
                    "[Id,Spent,Remain,Date,Description,User[Id,FirstName,LastName,Login],Role[Id,Name],Assignable[Id,Name]]",
                    Some("Date"),
                    Some(limit),
                );
                let r = ctx
                    .v1
                    .list("Times", &options)
                    .await
                    .map_err(failed("Could not read times"))?;
                entries.extend(r.items.iter().map(time_entry));
                truncated |= r.truncated;
            }
            if let Some(record) = &record {
                let mut where_: Vec<String> = Vec::new();
                if let (Some(card_ref), Some(card)) = (&card_ref, card) {
                    where_.push(format!("({card_ref}.Id eq {card})"));
                }
                if let Some(u) = user_id {
                    where_.push(format!("(ConnectedUser.Id eq {u})"));
                }
                if let Some(date) = record.date.as_deref().filter(|d| !d.is_empty()) {
                    if let Some(from) = &from {
                        where_.push(format!("(CustomFields.{date} gte '{from}')"));
                    }
                    if let Some(to) = &to {
                        where_.push(format!("(CustomFields.{date} lte '{to}')"));
                    }
                }
                let refs: Vec<String> = record
                    .card_refs
                    .iter()
                    .map(|k| format!("{k}[Id,Name]"))
                    .collect();
                let include = format!(
                    "[Id,Name,Description,ConnectedUser[Id,FirstName,LastName,Login],{},CustomFields]",
                    refs.join(",")
                );
                let options = list(
                    Some(where_.join(" and ")),
                    &include,
                    Some("Id"),
                    Some(limit),
                );
                let r = ctx
                    .v1
                    .list(&record.resource.path, &options)
                    .await
                    .map_err(failed(format!("Could not read {}", record.resource.path)))?;
                entries.extend(r.items.iter().map(|x| record_entry(x, record)));
                truncated |= r.truncated;
            }
            let date_of = |e: &Value| match &e["date"] {
                Value::Null => String::new(),
                v => js_string(v),
            };
            entries.sort_by(|a, b| locale_compare(&date_of(b), &date_of(a)));
            let total: f64 = entries
                .iter()
                .map(|e| e["spent"].as_f64().unwrap_or(0.0))
                .sum();
            let mut out = Map::new();
            out.insert("count".into(), json!(entries.len()));
            out.insert("truncated".into(), json!(truncated));
            out.insert("totalSpent".into(), round2(total));
            if truncated {
                out.insert(
                    "totalsPartial".into(),
                    json!("totalSpent covers only the returned entries"),
                );
            }
            out.insert("entries".into(), Value::Array(entries));
            success(Value::Object(out))
        }),
    }
}

fn read_attachments() -> Tool {
    Tool {
        name: "read_attachments",
        description: "Attachments of a card with name, size, owner, date and URL. URLs need Basic or cookie auth, not the token.",
        fields: vec![id("id")],
        handler: handler!(|ctx, args| {
            let card = args.id("id");
            let options = list(
                Some(format!("(General.Id eq {card})")),
                "[Id,Name,Description,Date,Owner[Id,FirstName,LastName,Login],MimeType,Size,Uri]",
                None,
                None,
            );
            let r = ctx
                .v1
                .list("Attachments", &options)
                .await
                .map_err(failed(format!("Could not read attachments of {card}")))?;
            let attachments: Vec<Value> = r
                .items
                .iter()
                .map(|a| {
                    compact([
                        ("id", num(&a["Id"])),
                        ("name", a["Name"].clone()),
                        ("description", a["Description"].clone()),
                        ("date", a["Date"].clone()),
                        ("owner", person(&a["Owner"])),
                        (
                            "mimeType",
                            or_else(a["MimeType"].clone(), || a["PersistedMimeType"].clone()),
                        ),
                        (
                            "size",
                            or_else(num(&a["Size"]), || num(&a["PersistedSize"])),
                        ),
                        ("uri", a["Uri"].clone()),
                    ])
                })
                .collect();
            success(json!({
                "card": card,
                "count": r.items.len(),
                "truncated": r.truncated,
                "attachments": attachments,
            }))
        }),
    }
}

fn read_test_plan() -> Tool {
    Tool {
        name: "read_test_plan",
        description: "A test plan with its test cases and ordered steps, child test plans, and recent runs.",
        fields: vec![
            id("id"),
            field(
                "runs",
                Kind::Num {
                    int: true,
                    positive: false,
                    min: Some(0.0),
                    max: Some(100.0),
                },
            )
            .optional()
            .describe("How many recent runs to include (default 10)"),
        ],
        handler: handler!(|ctx, args| {
            let plan_id = args.id("id");
            let plan = ctx
                .v1
                .get(
                    "TestPlans",
                    plan_id,
                    &ReadOptions {
                        include: Some("[Id,Name,Description,EntityState[Id,Name],Project[Id,Name],LinkedGeneral[Id,Name],ChildTestPlans[Id,Name]]".into()),
                        inner_take: Some(1000),
                        ..ReadOptions::default()
                    },
                )
                .await
                .map_err(failed(format!("Could not read test plan {plan_id}")))?;
            let cases = ctx
                .v1
                .list(
                    &format!("TestPlans/{plan_id}/TestCases"),
                    &ListOptions {
                        read: ReadOptions {
                            include: Some("[Id,Name,Description,LastStatus,LastRunDate,LastFailureComment,TestSteps[Id,Description,Result,RunOrder]]".into()),
                            inner_take: Some(1000),
                            ..ReadOptions::default()
                        },
                        ..ListOptions::default()
                    },
                )
                .await
                .map_err(failed(format!(
                    "Could not read test cases of test plan {plan_id}"
                )))?;
            let run_limit = args.opt_num("runs").map_or(10, |n| n as usize);
            let mut runs = Value::Null;
            if run_limit > 0 {
                let options = list(
                    Some(format!("(TestPlan.Id eq {plan_id})")),
                    "[Id,Name,EntityState[Id,Name],CreateDate,Build[Id,Name]]",
                    Some("CreateDate"),
                    Some(run_limit),
                );
                let r = ctx
                    .v1
                    .list("TestPlanRuns", &options)
                    .await
                    .map_err(failed(format!(
                        "Could not read runs of test plan {plan_id}"
                    )))?;
                runs = Value::Array(
                    r.items
                        .iter()
                        .map(|x| {
                            compact([
                                ("id", num(&x["Id"])),
                                ("name", x["Name"].clone()),
                                ("state", ref_name(&x["EntityState"])),
                                ("created", x["CreateDate"].clone()),
                                ("build", reference(&x["Build"])),
                            ])
                        })
                        .collect(),
                );
            }
            let order = |s: &Value| s["RunOrder"].as_f64().unwrap_or(0.0);
            let test_cases: Vec<Value> = cases
                .items
                .iter()
                .map(|c| {
                    let mut steps = items(&c["TestSteps"]);
                    steps
                        .sort_by(|a, b| order(a).partial_cmp(&order(b)).unwrap_or(Ordering::Equal));
                    compact([
                        ("id", num(&c["Id"])),
                        ("name", c["Name"].clone()),
                        ("lastStatus", c["LastStatus"].clone()),
                        ("lastRun", c["LastRunDate"].clone()),
                        ("lastFailure", text(&c["LastFailureComment"])),
                        ("description", text(&c["Description"])),
                        (
                            "steps",
                            Value::Array(
                                steps
                                    .iter()
                                    .map(|s| {
                                        compact([
                                            ("id", num(&s["Id"])),
                                            ("order", num(&s["RunOrder"])),
                                            (
                                                "step",
                                                or_else(text(&s["Description"]), || json!("")),
                                            ),
                                            ("expected", text(&s["Result"])),
                                        ])
                                    })
                                    .collect(),
                            ),
                        ),
                    ])
                })
                .collect();
            let cases_value = Value::Array(cases.items);
            let mut pairs: Vec<(String, Value)> = vec![
                ("id".into(), json!(plan_id)),
                ("name".into(), plan["Name"].clone()),
                ("state".into(), reference(&plan["EntityState"])),
                ("project".into(), reference(&plan["Project"])),
                ("linkedCard".into(), reference(&plan["LinkedGeneral"])),
                ("description".into(), text(&plan["Description"])),
                (
                    "childPlans".into(),
                    Value::Array(
                        items(&plan["ChildTestPlans"])
                            .iter()
                            .map(reference)
                            .collect(),
                    ),
                ),
                ("testCases".into(), Value::Array(test_cases)),
                ("truncated".into(), true_or_null(cases.truncated)),
            ];
            pairs.extend(inner_cap_note(&cases_value));
            pairs.push(("runs".into(), runs));
            success(compact(pairs))
        }),
    }
}

pub fn tools() -> Vec<Tool> {
    vec![
        read_card(),
        read_search(),
        read_my_work(),
        read_states(),
        read_people(),
        read_teams(),
        read_roles(),
        read_projects(),
        read_releases(),
        read_iterations(),
        read_custom_field_options(),
        read_comments(),
        read_relations(),
        read_times(),
        read_attachments(),
        read_test_plan(),
    ]
}
