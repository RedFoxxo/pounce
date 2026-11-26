//! Effort belongs to a role: writes go to `RoleEffort` rows, never to the card's
//! computed `Effort` total (domain rule 1).

use serde_json::{Map, Value, json};

use super::{num, v1_ref};
use crate::format::shape::{items, number};
use crate::http::TpErr;
use crate::http::v1::WriteOptions;
use crate::resolve::directory::V1Ref;
use crate::tools::Ctx;
use crate::tools::generic::validate::js_number;

#[derive(Debug, Clone, PartialEq)]
pub struct RoleEffortRow {
    /// The `RoleEffort` entity id.
    pub id: i64,
    pub role: V1Ref,
    /// 0 when the row carries none.
    pub effort: f64,
}

/// The role effort rows of a raw card's `RoleEfforts` collection.
pub fn role_effort_rows(raw: &Value) -> Vec<RoleEffortRow> {
    items(&raw["RoleEfforts"])
        .iter()
        .filter_map(|e| {
            Some(RoleEffortRow {
                id: e.get("Id")?.as_i64()?,
                role: v1_ref(&e["Role"])?,
                effort: num(&e["Effort"]).unwrap_or(0.0),
            })
        })
        .collect()
}

/// One role's requested effort.
#[derive(Debug, Clone, PartialEq)]
pub struct EffortRequest {
    pub role_id: i64,
    pub role_name: String,
    pub effort: f64,
}

/// One effort written (or already in place).
#[derive(Debug, Clone, PartialEq)]
pub struct AppliedEffort {
    pub role: String,
    /// The row's effort before; `None` when the row was created.
    pub before: Option<f64>,
    pub after: f64,
    /// The `RoleEffort` row id (0 when a create answered without one).
    pub row: i64,
    pub created: bool,
}

impl AppliedEffort {
    /// `{role, before, after, row, created}`.
    pub fn to_json(&self) -> Value {
        let mut out = Map::new();
        out.insert("role".into(), json!(self.role));
        if let Some(b) = self.before {
            out.insert("before".into(), number(b));
        }
        out.insert("after".into(), number(self.after));
        out.insert("row".into(), json!(self.row));
        out.insert("created".into(), json!(self.created));
        Value::Object(out)
    }
}

/// The write that stopped `apply_role_efforts`.
#[derive(Debug, Clone, PartialEq)]
pub struct EffortFailure {
    pub message: String,
    pub error: TpErr,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct EffortOutcome {
    /// What was done before any failure.
    pub applied: Vec<AppliedEffort>,
    pub error: Option<EffortFailure>,
}

/// Writes effort to the card's RoleEffort rows (never the card's Effort total,
/// which Targetprocess computes). Updates the existing row for a role, or
/// creates one when the card has none. Sequential: one card, one write at a
/// time; stops at the first failure.
pub async fn apply_role_efforts(
    ctx: &Ctx,
    card_id: i64,
    rows: &[RoleEffortRow],
    requests: &[EffortRequest],
) -> EffortOutcome {
    let mut outcome = EffortOutcome::default();
    for req in requests {
        let row = rows.iter().find(|r| r.role.id == req.role_id);
        if let Some(row) = row.filter(|r| r.effort == req.effort) {
            outcome.applied.push(AppliedEffort {
                role: req.role_name.clone(),
                before: Some(row.effort),
                after: req.effort,
                row: row.id,
                created: false,
            });
            continue;
        }
        let written = match row {
            Some(row) => {
                let mut body = Map::new();
                body.insert("Effort".into(), number(req.effort));
                ctx.v1
                    .update("RoleEfforts", row.id, body, &WriteOptions::default())
                    .await
            }
            None => {
                let body = json!({
                    "Assignable": { "Id": card_id },
                    "Role": { "Id": req.role_id },
                    "Effort": number(req.effort),
                });
                ctx.v1
                    .create("RoleEfforts", body, &WriteOptions::default())
                    .await
            }
        };
        let data = match written {
            Ok(data) => data,
            Err(error) => {
                outcome.error = Some(EffortFailure {
                    message: format!(
                        "Could not set {} effort to {}",
                        req.role_name,
                        js_number(req.effort)
                    ),
                    error,
                });
                return outcome;
            }
        };
        outcome.applied.push(AppliedEffort {
            role: req.role_name.clone(),
            before: row.map(|r| r.effort),
            after: req.effort,
            row: row
                .map(|r| r.id)
                .or_else(|| data.get("Id").and_then(Value::as_i64))
                .unwrap_or(0),
            created: row.is_none(),
        });
    }
    outcome
}
