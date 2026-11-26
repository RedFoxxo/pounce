//! How tools answer: compact JSON on success; on failure a one-line summary, then
//! Targetprocess's `status` and raw `body`.

use serde_json::{Map, Value, json};

use crate::http::TpErr;

/// What a tool answers: text, and whether it is an error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolResult {
    pub text: String,
    pub is_error: bool,
}

impl ToolResult {
    pub fn to_json(&self) -> Value {
        let mut result = json!({ "content": [{ "type": "text", "text": self.text }] });
        if self.is_error {
            result["isError"] = json!(true);
        }
        result
    }
}

/// `Err` is an early answer (a failure, or a refusal before anything was sent).
pub type Answer = Result<ToolResult, ToolResult>;

/// Compact JSON success response.
pub fn success(data: Value) -> Answer {
    Ok(ToolResult {
        text: data.to_string(),
        is_error: false,
    })
}

/// Failure response: a one-line summary, then Targetprocess's `status` and raw
/// `body` when the failure came from a request, then `details` when given.
pub fn failure(
    summary: &str,
    error: Option<&TpErr>,
    extra: Option<&Map<String, Value>>,
) -> ToolResult {
    let mut parts = summary.splitn(2, '\n');
    let mut lines = vec![parts.next().unwrap_or("").to_string()];
    let rest = parts.next().unwrap_or("").trim();
    if !rest.is_empty() {
        lines.push(rest.to_string());
    }
    if let Some(e) = error {
        if !e.message.is_empty() && !summary.contains(&e.message) {
            lines.push(format!("message: {}", e.message));
        }
        lines.push(format!("status: {}", e.status));
        lines.push(format!(
            "body: {}",
            if e.body.is_empty() {
                "(empty)"
            } else {
                &e.body
            }
        ));
        if let Some(request) = &e.request {
            lines.push(format!("request: {request}"));
        }
    }
    if let Some(extra) = extra.filter(|m| !m.is_empty()) {
        lines.push(format!("details: {}", Value::Object(extra.clone())));
    }
    ToolResult {
        text: lines.join("\n"),
        is_error: true,
    }
}

/// `failure` for `map_err`: the summary, then Targetprocess's side.
pub fn failed(summary: impl Into<String>) -> impl FnOnce(TpErr) -> ToolResult {
    let summary = summary.into();
    move |e| failure(&summary, Some(&e), None)
}

/// A call rejected before anything was sent.
pub fn invalid(message: impl AsRef<str>) -> ToolResult {
    failure(&format!("Not sent: {}", message.as_ref()), None, None)
}
