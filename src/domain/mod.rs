//! The Targetprocess rules pounce enforces (CLAUDE.md, "Domain rules").

pub mod assignments;
pub mod cards;
pub mod create;
pub mod custom_fields;
pub mod efforts;
pub mod parent;
pub mod time;

use serde_json::Value;

use crate::http::TpErr;
use crate::resolve::directory::V1Ref;
use crate::tools::{ToolResult, failure, invalid};

/// `{Id, Name}` as a reference (`shape.ts` `ref`): an object with a numeric `Id`; the name only when it is a string.
pub fn v1_ref(value: &Value) -> Option<V1Ref> {
    Some(V1Ref {
        id: value.get("Id")?.as_i64()?,
        name: value
            .get("Name")
            .and_then(Value::as_str)
            .map(str::to_string),
    })
}

/// A number field (`shape.ts` `num`).
pub fn num(value: &Value) -> Option<f64> {
    value.as_f64()
}

/// Why a domain step could not answer: a message, and the HTTP failure when there was one.
#[derive(Debug, Clone, PartialEq)]
pub struct Problem {
    pub message: String,
    pub error: Option<TpErr>,
}

impl Problem {
    /// The tool error: HTTP failures carry status/body, the rest is "Not sent".
    pub fn to_result(&self) -> ToolResult {
        match &self.error {
            Some(e) => failure(&self.message, Some(e), None),
            None => invalid(&self.message),
        }
    }
}
