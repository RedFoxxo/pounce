//! Side effects on a card's parent: its state and role efforts before and after a
//! write (domain rules 2 and 4).

use serde_json::{Map, Value, json};

use super::cards::parent_of;
use super::{num, v1_ref};
use crate::catalog::Request;
use crate::format::shape::{items, number};
use crate::http::v1::ReadOptions;
use crate::resolve::card::card_info;
use crate::resolve::directory::V1Ref;
use crate::tools::Ctx;

/// One role effort row of a parent.
#[derive(Debug, Clone, PartialEq)]
pub struct ParentEffort {
    /// Role name, or its id as text when the name is unknown.
    pub role: String,
    pub role_id: i64,
    pub effort: f64,
}

/// State and efforts of a parent at one moment.
#[derive(Debug, Clone, PartialEq)]
pub struct ParentSnapshot {
    pub id: i64,
    /// Resource name, e.g. `UserStory`.
    pub type_name: String,
    pub name: Option<String>,
    pub state: Option<V1Ref>,
    pub effort: Option<f64>,
    pub role_efforts: Vec<ParentEffort>,
}

/// A card to watch: its id, and its resource name when known (v1 `ResourceType`).
#[derive(Debug, Clone, PartialEq)]
pub struct ParentRef {
    pub id: i64,
    pub resource_type: Option<String>,
}

/// A parent that could not be read: never mistaken for "no parent".
#[derive(Debug, Clone, PartialEq)]
pub struct UnreadParent {
    pub id: i64,
    pub type_name: Option<String>,
    pub message: String,
}

/// A parent read: its snapshot, or why it could not be read.
pub type ParentRead = Result<ParentSnapshot, UnreadParent>;

/// State and role efforts of a card's nearest parent; `None` only when the card has no parent.
pub async fn parent_snapshot(ctx: &Ctx, raw: &Value) -> Option<ParentRead> {
    let parent = parent_of(raw)?;
    let parent = ParentRef {
        id: parent.id,
        resource_type: parent.resource_type,
    };
    Some(snapshot_of(ctx, &parent).await)
}

/// State and role efforts of one specific card (the parent being watched).
pub async fn snapshot_of(ctx: &Ctx, parent: &ParentRef) -> ParentRead {
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
            Err(r) => {
                return Err(UnreadParent {
                    id: parent.id,
                    type_name: None,
                    message: r.message,
                });
            }
        },
    };
    let mut fields = String::from("[Id,Name,EntityState[Id,Name]");
    if catalog.member(&resource, "Effort").is_some() {
        fields.push_str(",Effort");
    }
    if catalog.member(&resource, "RoleEfforts").is_some() {
        fields.push_str(",RoleEfforts[Id,Role[Id,Name],Effort]");
    }
    fields.push(']');
    let options = ReadOptions {
        include: Some(fields),
        inner_take: Some(1000),
        ..ReadOptions::default()
    };
    let data = ctx
        .v1
        .get(&resource.path, parent.id, &options)
        .await
        .map_err(|e| UnreadParent {
            id: parent.id,
            type_name: Some(resource.name.clone()),
            message: e.message,
        })?;
    let role_efforts = items(&data["RoleEfforts"])
        .iter()
        .filter_map(|e| {
            let role = v1_ref(&e["Role"])?;
            let effort = num(&e["Effort"])?;
            Some(ParentEffort {
                role: role.name.unwrap_or_else(|| role.id.to_string()),
                role_id: role.id,
                effort,
            })
        })
        .collect();
    Ok(ParentSnapshot {
        id: parent.id,
        type_name: resource.name.clone(),
        name: data["Name"].as_str().map(str::to_string),
        state: v1_ref(&data["EntityState"]),
        effort: num(&data["Effort"]),
        role_efforts,
    })
}

/// Re-reads the parent captured in `before`, so before/after always describe the same card.
pub async fn snapshot_again(ctx: &Ctx, before: Option<&ParentRead>) -> Option<ParentRead> {
    let parent = match before? {
        Ok(s) => ParentRef {
            id: s.id,
            resource_type: Some(s.type_name.clone()).filter(|t| !t.is_empty()),
        },
        Err(u) => ParentRef {
            id: u.id,
            resource_type: u.type_name.clone().filter(|t| !t.is_empty()),
        },
    };
    Some(snapshot_of(ctx, &parent).await)
}

/// A state before and after; `changed` compares the state ids.
#[derive(Debug, Clone, PartialEq)]
pub struct StateChange {
    pub before: Option<String>,
    pub after: Option<String>,
    pub changed: bool,
}

/// A number before and after (either may be unknown).
#[derive(Debug, Clone, PartialEq)]
pub struct EffortChange {
    pub before: Option<f64>,
    pub after: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RoleEffortChange {
    pub role: String,
    pub before: Option<f64>,
    pub after: Option<f64>,
}

/// What changed on a parent between two reads.
#[derive(Debug, Clone, PartialEq)]
pub struct ParentChange {
    pub id: i64,
    pub type_name: Option<String>,
    pub name: Option<String>,
    pub state: Option<StateChange>,
    pub effort: Option<EffortChange>,
    pub role_efforts: Option<Vec<RoleEffortChange>>,
    /// Set when the parent could not be read before or after: no change is claimed either way.
    pub unknown: Option<String>,
}

fn before_after(before: Option<f64>, after: Option<f64>) -> Map<String, Value> {
    let mut out = Map::new();
    if let Some(b) = before {
        out.insert("before".into(), number(b));
    }
    if let Some(a) = after {
        out.insert("after".into(), number(a));
    }
    out
}

impl ParentChange {
    /// `{id, type, name, state: {changed, before, after}, effort, roleEfforts, unknown}`, unknown values left out.
    pub fn to_json(&self) -> Value {
        let mut out = Map::new();
        out.insert("id".into(), json!(self.id));
        if let Some(t) = &self.type_name {
            out.insert("type".into(), json!(t));
        }
        if let Some(n) = &self.name {
            out.insert("name".into(), json!(n));
        }
        if let Some(s) = &self.state {
            let mut state = Map::new();
            state.insert("changed".into(), json!(s.changed));
            if let Some(b) = &s.before {
                state.insert("before".into(), json!(b));
            }
            if let Some(a) = &s.after {
                state.insert("after".into(), json!(a));
            }
            out.insert("state".into(), Value::Object(state));
        }
        if let Some(e) = &self.effort {
            out.insert(
                "effort".into(),
                Value::Object(before_after(e.before, e.after)),
            );
        }
        if let Some(list) = &self.role_efforts {
            let list = list
                .iter()
                .map(|e| {
                    let mut row = Map::new();
                    row.insert("role".into(), json!(e.role));
                    row.extend(before_after(e.before, e.after));
                    Value::Object(row)
                })
                .collect();
            out.insert("roleEfforts".into(), Value::Array(list));
        }
        if let Some(u) = &self.unknown {
            out.insert("unknown".into(), json!(u));
        }
        Value::Object(out)
    }
}

/// What changed on the parent between two reads; always reports the state, even when unchanged.
/// `None` only when neither read exists (the card has no parent).
pub fn parent_change(
    before: Option<&ParentRead>,
    after: Option<&ParentRead>,
) -> Option<ParentChange> {
    if before.is_none() && after.is_none() {
        return None;
    }
    let (Some(Ok(b)), Some(Ok(a))) = (before, after) else {
        let failed = [before, after]
            .into_iter()
            .flatten()
            .find_map(|r| r.as_ref().err());
        let known = [before, after]
            .into_iter()
            .flatten()
            .find_map(|r| r.as_ref().ok());
        let type_name = known
            .map(|k| k.type_name.clone())
            .or_else(|| failed.and_then(|f| f.type_name.clone()))
            .filter(|t| !t.is_empty());
        let when = if matches!(before, Some(Ok(_))) {
            "after"
        } else {
            "before"
        };
        return Some(ParentChange {
            id: known.map(|k| k.id).or(failed.map(|f| f.id)).unwrap_or(0),
            type_name,
            name: known.and_then(|k| k.name.clone()).filter(|n| !n.is_empty()),
            state: None,
            effort: None,
            role_efforts: None,
            unknown: Some(format!(
                "the parent could not be read {when} the write ({}); side effects are unknown",
                failed.map_or("no response", |f| f.message.as_str())
            )),
        });
    };
    let state_name = |s: &ParentSnapshot| {
        s.state
            .as_ref()
            .and_then(|s| s.name.clone())
            .filter(|n| !n.is_empty())
    };
    let effort = (b.effort != a.effort).then_some(EffortChange {
        before: b.effort,
        after: a.effort,
    });
    let mut roles: Vec<&str> = Vec::new();
    for e in b.role_efforts.iter().chain(&a.role_efforts) {
        if !roles.contains(&e.role.as_str()) {
            roles.push(&e.role);
        }
    }
    let find =
        |list: &[ParentEffort], role: &str| list.iter().find(|e| e.role == role).map(|e| e.effort);
    let efforts: Vec<RoleEffortChange> = roles
        .into_iter()
        .filter_map(|role| {
            let eb = find(&b.role_efforts, role);
            let ea = find(&a.role_efforts, role);
            (ea != eb).then(|| RoleEffortChange {
                role: role.to_string(),
                before: eb,
                after: ea,
            })
        })
        .collect();
    Some(ParentChange {
        id: a.id,
        type_name: Some(a.type_name.clone()),
        name: a.name.clone().filter(|n| !n.is_empty()),
        state: Some(StateChange {
            before: state_name(b),
            after: state_name(a),
            changed: b.state.as_ref().map(|s| s.id) != a.state.as_ref().map(|s| s.id),
        }),
        effort,
        role_efforts: (!efforts.is_empty()).then_some(efforts),
        unknown: None,
    })
}
