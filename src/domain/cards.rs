//! Reading a card: what to include, its parent chain, and the `read_card` shape.

use std::collections::HashSet;

use serde_json::{Map, Value, json};

use super::{Problem, v1_ref};
use crate::catalog::{Catalog, CatalogResource, Request};
use crate::format::shape::{
    assignment, compact, custom_fields, items, num, person, reference, role_effort, tags,
    team_assignment, text,
};
use crate::http::Res;
use crate::http::v1::ReadOptions;
use crate::resolve::card::{CardInfo, card_info};
use crate::tools::Ctx;

/// Parent references, nearest first. The first one set on a card is its parent (LinkedGeneral: the card a test plan covers).
pub const PARENT_REFS: [&str; 5] = [
    "UserStory",
    "Feature",
    "Epic",
    "PortfolioEpic",
    "LinkedGeneral",
];

/// What a card read asks for, as (member, include expression); members the resource lacks are skipped.
const CARD_INCLUDE: [(&str, &str); 29] = [
    ("Name", "Name"),
    ("Description", "Description"),
    ("EntityType", "EntityType[Id,Name]"),
    ("EntityState", "EntityState[Id,Name,IsInitial,IsFinal]"),
    ("Project", "Project[Id,Name,Process[Id,Name]]"),
    ("Owner", "Owner[Id,FirstName,LastName,Login]"),
    ("CreateDate", "CreateDate"),
    ("ModifyDate", "ModifyDate"),
    ("Tags", "Tags"),
    ("Effort", "Effort"),
    ("EffortCompleted", "EffortCompleted"),
    ("EffortToDo", "EffortToDo"),
    ("TimeSpent", "TimeSpent"),
    ("TimeRemain", "TimeRemain"),
    ("Priority", "Priority[Id,Name]"),
    ("Severity", "Severity[Id,Name]"),
    ("Release", "Release[Id,Name]"),
    ("Iteration", "Iteration[Id,Name]"),
    ("TeamIteration", "TeamIteration[Id,Name]"),
    ("Team", "Team[Id,Name]"),
    ("CustomFields", "CustomFields"),
    (
        "Assignments",
        "Assignments[Id,Role[Id,Name],GeneralUser[Id,FirstName,LastName,Login]]",
    ),
    (
        "RoleEfforts",
        "RoleEfforts[Id,Role[Id,Name],Effort,EffortCompleted,EffortToDo]",
    ),
    (
        "AssignedTeams",
        "AssignedTeams[Id,Team[Id,Name],EntityState[Id,Name]]",
    ),
    ("UserStory", "UserStory[Id,Name]"),
    ("Feature", "Feature[Id,Name]"),
    ("Epic", "Epic[Id,Name]"),
    ("PortfolioEpic", "PortfolioEpic[Id,Name]"),
    ("LinkedGeneral", "LinkedGeneral[Id,Name]"),
];

/// Child collections counted on a card; only those the card's resource has are asked for (live-checked).
const COUNTED: [&str; 11] = [
    "Tasks",
    "Bugs",
    "UserStories",
    "Features",
    "Epics",
    "TestCases",
    "AcceptanceCriteria",
    "Comments",
    "Attachments",
    "Times",
    "TimeRecords",
];

fn has(catalog: &Catalog, resource: &CatalogResource, member: &str) -> bool {
    catalog.member(resource, member).is_some()
}

/// The `include` of a card read: `[Id,...]` with every member the resource has.
pub fn card_include(catalog: &Catalog, resource: &CatalogResource) -> String {
    let members: Vec<&str> = CARD_INCLUDE
        .iter()
        .filter(|(m, _)| has(catalog, resource, m))
        .map(|(_, e)| *e)
        .collect();
    format!("[Id,{}]", members.join(","))
}

/// The `append` of a card read (`[Tasks-Count,...]`), or `None` when the resource has no counted collection.
pub fn card_append(resource: &CatalogResource) -> Option<String> {
    let counts: Vec<String> = COUNTED
        .iter()
        .filter(|c| resource.collections.iter().any(|x| x.name == **c))
        .map(|c| format!("{c}-Count"))
        .collect();
    (!counts.is_empty()).then(|| format!("[{}]", counts.join(",")))
}

/// The nearest parent reference set on a card.
#[derive(Debug, Clone, PartialEq)]
pub struct ParentOf {
    /// The reference it was found in, one of `PARENT_REFS`.
    pub member: &'static str,
    pub id: i64,
    pub name: Option<String>,
    /// The parent's resource name as v1 reports it (`ResourceType`), e.g. `UserStory`.
    pub resource_type: Option<String>,
}

/// The nearest parent reference set on a raw card, with its resource name.
pub fn parent_of(raw: &Value) -> Option<ParentOf> {
    PARENT_REFS.iter().find_map(|&member| {
        let v = raw.get(member).filter(|v| v.is_object())?;
        let id = v.get("Id").filter(|id| id.is_number())?.as_i64()?;
        Some(ParentOf {
            member,
            id,
            name: v.get("Name").and_then(Value::as_str).map(str::to_string),
            resource_type: v
                .get("ResourceType")
                .and_then(Value::as_str)
                .map(str::to_string),
        })
    })
}

/// One card of a parent chain.
#[derive(Debug, Clone, PartialEq)]
pub struct ParentLink {
    pub id: i64,
    /// Resource name; unknown only when the parent could not be resolved.
    pub type_name: Option<String>,
    pub name: Option<String>,
    pub state: Option<String>,
}

impl ParentLink {
    /// `{id, type, name, state}` without the unknown ones.
    pub fn to_json(&self) -> Value {
        compact([
            ("id", json!(self.id)),
            ("type", json!(self.type_name)),
            ("name", json!(self.name)),
            ("state", json!(self.state)),
        ])
    }
}

/// Walks up the parent chain (task → story → feature → epic → portfolio epic), nearest
/// first, at most `max_depth` cards (`read_card` passes 5). A parent that cannot be
/// read ends the chain with what is known of it.
pub async fn parent_chain(ctx: &Ctx, raw: &Value, max_depth: usize) -> Vec<ParentLink> {
    let mut chain = Vec::new();
    let mut seen = HashSet::new();
    let mut current: Option<Value> = None;
    while chain.len() < max_depth {
        let Some(parent) = parent_of(current.as_ref().unwrap_or(raw)) else {
            break;
        };
        if !seen.insert(parent.id) {
            break;
        }
        let names = parent.resource_type.iter().cloned().collect();
        let catalog = ctx.catalog.get(&Request::Names(names)).await;
        let found = parent
            .resource_type
            .as_deref()
            .and_then(|t| catalog.find(t))
            .filter(|r| r.name != "General" && r.name != "Assignable")
            .cloned();
        let resource = match found {
            Some(resource) => resource,
            None => match card_info(ctx, parent.id).await {
                Ok(info) => info.resource,
                Err(_) => {
                    chain.push(ParentLink {
                        id: parent.id,
                        type_name: parent.resource_type,
                        name: parent.name,
                        state: None,
                    });
                    break;
                }
            },
        };
        let mut fields = vec![
            "Id".to_string(),
            "Name".into(),
            "EntityState[Id,Name]".into(),
        ];
        fields.extend(
            PARENT_REFS
                .iter()
                .filter(|p| has(&catalog, &resource, p))
                .map(|p| format!("{p}[Id,Name]")),
        );
        let options = ReadOptions {
            include: Some(format!("[{}]", fields.join(","))),
            ..ReadOptions::default()
        };
        let data = match ctx.v1.get(&resource.path, parent.id, &options).await {
            Ok(data) if !data.is_null() => data,
            _ => {
                chain.push(ParentLink {
                    id: parent.id,
                    type_name: Some(resource.name.clone()),
                    name: parent.name,
                    state: None,
                });
                break;
            }
        };
        chain.push(ParentLink {
            id: parent.id,
            type_name: Some(resource.name.clone()),
            name: data
                .get("Name")
                .and_then(Value::as_str)
                .map(str::to_string)
                .or(parent.name),
            state: data
                .get("EntityState")
                .and_then(v1_ref)
                .and_then(|s| s.name),
        });
        current = Some(data);
    }
    chain
}

/// A card read by id: what it is, and everything `read_card` reports.
#[derive(Debug, Clone)]
pub struct CardRead {
    pub info: CardInfo,
    pub raw: Value,
}

/// Reads a card by id alone: resolves its type, then reads everything read_card reports.
pub async fn read_card_raw(ctx: &Ctx, id: i64) -> Result<CardRead, Problem> {
    let info = card_info(ctx, id).await.map_err(|r| Problem {
        message: r.message,
        error: r.error,
    })?;
    match read_card_as(ctx, &info).await {
        Ok(raw) => Ok(CardRead { info, raw }),
        Err(e) => Err(Problem {
            message: format!("Could not read {} {id}", info.entity_type),
            error: Some(e),
        }),
    }
}

/// Reads everything read_card reports of a card whose type is known.
pub async fn read_card_as(ctx: &Ctx, info: &CardInfo) -> Res<Value> {
    let catalog = ctx.catalog.get(&Request::Names(Vec::new())).await;
    let options = ReadOptions {
        include: Some(card_include(&catalog, &info.resource)),
        append: card_append(&info.resource),
        inner_take: Some(1000),
        ..ReadOptions::default()
    };
    ctx.v1.get(&info.resource.path, info.id, &options).await
}

/// JavaScript truthiness of a JSON value.
fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Value::String(s) => !s.is_empty(),
        _ => true,
    }
}

/// The `read_card` answer: a compact object, keys in a fixed order, unknown values left out.
/// `parents` is the chain from `parent_chain`, when it was read.
pub fn shape_card(info: &CardInfo, raw: &Value, parents: Option<&[ParentLink]>) -> Value {
    let null = Value::Null;
    let field = |key: &str| raw.get(key).unwrap_or(&null);
    let mut counts = Map::new();
    if let Some(object) = raw.as_object() {
        for (key, value) in object {
            if let Some(stem) = key.strip_suffix("-Count").filter(|_| value.is_number()) {
                let mut chars = stem.chars();
                let name: String = match chars.next() {
                    Some(first) => first.to_lowercase().chain(chars).collect(),
                    None => String::new(),
                };
                counts.insert(name, value.clone());
            }
        }
    }
    let state = Some(field("EntityState")).filter(|s| truthy(s)).map(|s| {
        let flag = |key: &str| {
            s.get(key)
                .filter(|v| truthy(v))
                .cloned()
                .unwrap_or(Value::Null)
        };
        compact([
            ("id", num(s.get("Id").unwrap_or(&null))),
            ("name", s.get("Name").cloned().unwrap_or(Value::Null)),
            ("initial", flag("IsInitial")),
            ("final", flag("IsFinal")),
        ])
    });
    let effort = compact([
        ("total", num(field("Effort"))),
        ("completed", num(field("EffortCompleted"))),
        ("toDo", num(field("EffortToDo"))),
        ("timeSpent", num(field("TimeSpent"))),
        ("timeRemaining", num(field("TimeRemain"))),
    ]);
    let listed = |key: &str, shape: fn(&Value) -> Value| {
        if raw.get(key).is_some() {
            Value::Array(items(field(key)).iter().map(shape).collect())
        } else {
            Value::Null
        }
    };
    compact([
        ("id", json!(info.id)),
        ("type", json!(info.entity_type)),
        ("name", field("Name").clone()),
        ("state", state.unwrap_or(Value::Null)),
        ("project", reference(field("Project"))),
        (
            "parents",
            match parents {
                Some(p) if !p.is_empty() => {
                    Value::Array(p.iter().map(ParentLink::to_json).collect())
                }
                _ => Value::Null,
            },
        ),
        ("team", reference(field("Team"))),
        ("teams", listed("AssignedTeams", team_assignment)),
        ("assignments", listed("Assignments", assignment)),
        ("roleEfforts", listed("RoleEfforts", role_effort)),
        (
            "effort",
            if effort.as_object().is_some_and(|o| !o.is_empty()) {
                effort
            } else {
                Value::Null
            },
        ),
        ("release", reference(field("Release"))),
        ("iteration", reference(field("Iteration"))),
        ("teamIteration", reference(field("TeamIteration"))),
        ("priority", reference(field("Priority"))),
        ("severity", reference(field("Severity"))),
        ("owner", person(field("Owner"))),
        ("tags", tags(field("Tags"))),
        ("customFields", custom_fields(field("CustomFields"))),
        (
            "counts",
            if counts.is_empty() {
                Value::Null
            } else {
                Value::Object(counts)
            },
        ),
        ("created", field("CreateDate").clone()),
        ("modified", field("ModifyDate").clone()),
        ("description", text(field("Description"))),
    ])
}
