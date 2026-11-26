//! MCP over stdio: one JSON-RPC message per line. pounce only serves tools, so it answers
//! `initialize`, `ping`, `tools/list` and `tools/call`; tool calls run concurrently.

use std::sync::Arc;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;

use crate::tools::{self, Ctx, Tool, ToolResult};

const LATEST_PROTOCOL_VERSION: &str = "2025-11-25";
const SUPPORTED_PROTOCOL_VERSIONS: [&str; 5] = [
    LATEST_PROTOCOL_VERSION,
    "2025-06-18",
    "2025-03-26",
    "2024-11-05",
    "2024-10-07",
];

fn reply(id: Value, result: Value) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "result": result }).to_string()
}

fn error(id: Value, code: i64, message: &str) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }).to_string()
}

/// Serves until stdin closes, then waits for the calls still running.
pub async fn serve(ctx: Arc<Ctx>, tools: Arc<Vec<Tool>>) {
    // The list is built once and says the same on every request.
    let listed = tools::list(&tools);
    let (out, mut lines_out) = mpsc::unbounded_channel::<String>();
    let writer = tokio::spawn(async move {
        let mut stdout = tokio::io::stdout();
        while let Some(line) = lines_out.recv().await {
            let written = async {
                stdout.write_all(line.as_bytes()).await?;
                stdout.write_all(b"\n").await?;
                stdout.flush().await
            };
            if written.await.is_err() {
                break;
            }
        }
    });

    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        if line.trim().is_empty() {
            continue;
        }
        let Ok(message) = serde_json::from_str::<Value>(&line) else {
            let _ = out.send(error(Value::Null, -32700, "Parse error"));
            continue;
        };
        // Notifications (no id) and the client's answers (no method) need nothing back.
        let (Some(method), Some(id)) = (
            message.get("method").and_then(Value::as_str),
            message.get("id").cloned(),
        ) else {
            continue;
        };
        let params = message.get("params").cloned().unwrap_or(Value::Null);
        match method {
            "initialize" => {
                let asked = params
                    .get("protocolVersion")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let version = if SUPPORTED_PROTOCOL_VERSIONS.contains(&asked) {
                    asked
                } else {
                    LATEST_PROTOCOL_VERSION
                };
                let result = json!({
                    "protocolVersion": version,
                    "capabilities": { "tools": { "listChanged": true } },
                    "serverInfo": { "name": "pounce", "version": env!("CARGO_PKG_VERSION") },
                });
                let _ = out.send(reply(id, result));
            }
            "ping" => {
                let _ = out.send(reply(id, json!({})));
            }
            "tools/list" => {
                let _ = out.send(reply(id, listed.clone()));
            }
            "tools/call" => {
                let Some(name) = params
                    .get("name")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                else {
                    let _ = out.send(error(
                        id,
                        -32602,
                        "Invalid params: tools/call needs a tool name",
                    ));
                    continue;
                };
                let args = params.get("arguments").cloned().unwrap_or(Value::Null);
                let (ctx, tools, out) = (ctx.clone(), tools.clone(), out.clone());
                tokio::spawn(async move {
                    // A panic in one tool answers that call with an error instead of leaving it unanswered.
                    let run =
                        tokio::spawn(async move { tools::call(&ctx, &tools, &name, &args).await });
                    let result = run.await.unwrap_or_else(|_| ToolResult {
                        text: "pounce: internal error".into(),
                        is_error: true,
                    });
                    let _ = out.send(reply(id, result.to_json()));
                });
            }
            _ => {
                let _ = out.send(error(id, -32601, "Method not found"));
            }
        }
    }
    drop(out);
    let _ = writer.await;
}
