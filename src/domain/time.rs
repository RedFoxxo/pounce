//! Where time is logged. Standard Targetprocess uses `Time` entries, available
//! only in processes with the "Time Tracking" practice. Some instances turn that
//! off and track time in a custom `TimeRecord` type (live: an "Hours" number and
//! a "Date" custom field, linked to the user and the card; an automation names
//! the record and fills in day/week/month periods). pounce uses it only when it
//! is there and has an hours field.

use std::sync::Arc;

use super::Problem;
use crate::catalog::{CatalogResource, Request};
use crate::resolve::directory::TpCustomField;
use crate::tools::Ctx;

pub const TIME_TRACKING_PRACTICE: &str = "Time Tracking";

/// Card types a record can link to, most specific first (a task's record also carries its story).
const CARD_TYPES: [&str; 7] = [
    "Task",
    "Bug",
    "UserStory",
    "Request",
    "Feature",
    "Epic",
    "PortfolioEpic",
];

/// The instance's custom `TimeRecord` type, as pounce writes it.
#[derive(Debug, Clone)]
pub struct TimeRecordBackend {
    pub resource: Arc<CatalogResource>,
    /// Custom field holding the hours, e.g. "Hours".
    pub hours: String,
    /// Custom field holding the day, e.g. "Date", when the type has one.
    pub date: Option<String>,
    /// Card references a record can carry, most specific first, e.g. Task, Bug, UserStory.
    pub card_refs: Vec<String>,
}

#[derive(Debug, Clone)]
pub enum TimeBackend {
    /// Standard `Time` entries.
    Time,
    TimeRecord(TimeRecordBackend),
}

fn is_one_of(text: &str, names: &[&str]) -> bool {
    names.iter().any(|n| text.eq_ignore_ascii_case(n))
}

/// The TimeRecord backend if this instance has one, whatever the process; `None`
/// when there is no creatable TimeRecord type with an Hours number field.
/// The `Problem` always carries the HTTP error.
pub async fn time_record_backend(ctx: &Ctx) -> Result<Option<TimeRecordBackend>, Problem> {
    let catalog = ctx
        .catalog
        .get(&Request::Names(vec!["TimeRecord".into()]))
        .await;
    let Some(resource) = catalog
        .find("TimeRecord")
        .filter(|r| r.available && r.can_create)
    else {
        return Ok(None);
    };
    let fields = ctx
        .directory
        .entity_custom_fields(&resource.name)
        .await
        .map_err(|e| Problem {
            message: format!("Could not read the {} custom fields", resource.name),
            error: Some(e),
        })?;
    fn field_type(f: &TpCustomField) -> &str {
        f.field_type.as_deref().unwrap_or("")
    }
    let Some(hours) = fields.iter().find(|f| {
        is_one_of(&f.name, &["hour", "hours"])
            && is_one_of(field_type(f), &["number", "money", "percent"])
    }) else {
        return Ok(None);
    };
    let date = fields
        .iter()
        .find(|f| is_one_of(&f.name, &["date"]) && is_one_of(field_type(f), &["date"]));
    let rank = |t: &str| CARD_TYPES.iter().position(|c| *c == t);
    let mut refs: Vec<(usize, &str)> = resource
        .references
        .iter()
        .filter(|r| r.can_set)
        .filter_map(|r| Some((rank(&r.type_name)?, r.name.as_str())))
        .collect();
    refs.sort_by_key(|(rank, _)| *rank);
    Ok(Some(TimeRecordBackend {
        resource: resource.clone(),
        hours: hours.name.clone(),
        date: date.map(|d| d.name.clone()),
        card_refs: refs.into_iter().map(|(_, name)| name.to_string()).collect(),
    }))
}

/// The backend for a card's process: `Time` where the process tracks time,
/// otherwise `TimeRecord` when the instance has it. Without a process, `Time`
/// unless the instance has a TimeRecord type.
pub async fn time_backend(ctx: &Ctx, process_id: Option<i64>) -> Result<TimeBackend, Problem> {
    if let Some(process_id) = process_id {
        let practices = ctx
            .directory
            .practices(process_id)
            .await
            .map_err(|e| Problem {
                message: format!("Could not read the practices of process {process_id}"),
                error: Some(e),
            })?;
        if practices.iter().any(|p| p == TIME_TRACKING_PRACTICE) {
            return Ok(TimeBackend::Time);
        }
    }
    if let Some(record) = time_record_backend(ctx).await? {
        return Ok(TimeBackend::TimeRecord(record));
    }
    if process_id.is_none() {
        return Ok(TimeBackend::Time);
    }
    Err(Problem {
        message: format!(
            "Time tracking is off in this card's process (no \"{TIME_TRACKING_PRACTICE}\" practice) and the instance has no TimeRecord type with an Hours field"
        ),
        error: None,
    })
}
