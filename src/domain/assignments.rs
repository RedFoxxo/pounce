//! A card's assignments (person + role), read and written one row at a time
//! (domain rules 3 and 9).

use serde_json::{Value, json};

use super::v1_ref;
use crate::format::shape::person;
use crate::http::Res;
use crate::http::v1::{ListOptions, ReadOptions, WriteOptions};
use crate::resolve::directory::V1Ref;
use crate::tools::Ctx;

/// The assigned person, as `shape::person` names them.
#[derive(Debug, Clone, PartialEq)]
pub struct AssignedUser {
    pub id: i64,
    /// Display name: full name, else login, else `User {id}`.
    pub name: String,
    pub login: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AssignmentRow {
    /// The `Assignment` entity id.
    pub id: i64,
    pub user: AssignedUser,
    pub role: V1Ref,
}

fn row(a: &Value) -> Option<AssignmentRow> {
    let id = a.get("Id")?.as_i64()?;
    let user = person(&a["GeneralUser"]);
    let user = AssignedUser {
        id: user.get("id")?.as_i64()?,
        name: user["name"].as_str().unwrap_or_default().to_string(),
        login: user
            .get("login")
            .and_then(Value::as_str)
            .map(str::to_string),
    };
    let role = v1_ref(&a["Role"])?;
    Some(AssignmentRow { id, user, role })
}

/// Every assignment of a card, read fresh from `/Assignments` (fully paged).
pub async fn list_assignments(ctx: &Ctx, card_id: i64) -> Res<Vec<AssignmentRow>> {
    let options = ListOptions {
        read: ReadOptions {
            include: Some("[Id,Role[Id,Name],GeneralUser[Id,FirstName,LastName,Login]]".into()),
            ..ReadOptions::default()
        },
        where_: Some(format!("(Assignable.Id eq {card_id})")),
        ..ListOptions::default()
    };
    let list = ctx.v1.list("Assignments", &options).await?;
    Ok(list
        .items
        .iter()
        .filter(|a| a.is_object())
        .filter_map(row)
        .collect())
}

/// Creates one assignment. Posting to `/Assignments` adds exactly this row and nothing else.
pub async fn add_assignment(ctx: &Ctx, card_id: i64, user_id: i64, role_id: i64) -> Res<Value> {
    let body = json!({
        "Assignable": { "Id": card_id },
        "GeneralUser": { "Id": user_id },
        "Role": { "Id": role_id },
    });
    ctx.v1
        .create("Assignments", body, &WriteOptions::default())
        .await
}

/// Deletes one assignment by its id.
pub async fn remove_assignment(ctx: &Ctx, assignment_id: i64) -> Res<Value> {
    ctx.v1.delete("Assignments", assignment_id).await
}

/// `{id, user: "Name (id)", role}` for answers; the role is its name, or its id as text.
pub fn describe_assignment(a: &AssignmentRow) -> Value {
    json!({
        "id": a.id,
        "user": format!("{} ({})", a.user.name, a.user.id),
        "role": a.role.name.clone().unwrap_or_else(|| a.role.id.to_string()),
    })
}
