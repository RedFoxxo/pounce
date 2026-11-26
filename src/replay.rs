//! Replays the tool transcripts recorded from the TypeScript build
//! (`tests/golden/transcripts/`): every tool call with its arguments, every request
//! it sent to Targetprocess with the answer it got, and the tool's result. pounce
//! must send the same requests (in any order within a call) and give the same
//! answers. Tests that drove the context directly or overlapped calls are skipped
//! here and covered by unit tests instead.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};

use crate::config::Config;
use crate::http::{Body, BoxFuture, FormField, Request, Response, Transport};
use crate::tools::{self, Clock, TOOLSETS, Toolset, context};

const BASE: &str = "https://tp.example";
const TOKEN: &str = "test-token-0000";

/// Equal JSON, numbers compared by value (6 and 6.0) and objects in any key order.
pub fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(p, q)| same(p, q))
        }
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len() && x.iter().all(|(k, v)| y.get(k).is_some_and(|w| same(v, w)))
        }
        _ => a == b,
    }
}

/// A recorded request with its answer.
#[derive(Debug, Clone)]
struct Exchange {
    method: String,
    path: String,
    query: Vec<(String, String)>,
    body: Value,
    response: Value,
}

impl Exchange {
    fn from_event(e: &Value) -> Self {
        let mut query: Vec<(String, String)> = e["query"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|p| {
                (
                    p[0].as_str().unwrap_or_default().to_string(),
                    p[1].as_str().unwrap_or_default().to_string(),
                )
            })
            .collect();
        query.sort();
        Exchange {
            method: e["method"].as_str().unwrap_or_default().to_string(),
            path: e["path"].as_str().unwrap_or_default().to_string(),
            query,
            body: e["body"].clone(),
            response: e["response"].clone(),
        }
    }

    fn describe(&self) -> String {
        let query: Vec<String> = self.query.iter().map(|(k, v)| format!("{k}={v}")).collect();
        let body = if self.body.is_null() {
            String::new()
        } else {
            format!(" {}", self.body)
        };
        format!("{} {}?{}{body}", self.method, self.path, query.join("&"))
    }
}

/// The request as the transcript keeps it.
fn recorded_form(request: &Request) -> Exchange {
    let url = url::Url::parse(&request.url).expect("pounce builds valid URLs");
    let mut query: Vec<(String, String)> = url
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    query.sort();
    let body = match &request.body {
        Body::Empty => Value::Null,
        Body::Json(v) => json!({ "json": v }),
        Body::Form(fields) => {
            use base64::Engine;
            let fields: Vec<Value> = fields
                .iter()
                .map(|f| match f {
                    FormField::Text { name, value } => json!({ "name": name, "value": value }),
                    FormField::File {
                        name,
                        filename,
                        content_type,
                        bytes,
                    } => json!({
                        "name": name,
                        "filename": filename,
                        "type": content_type,
                        "base64": base64::engine::general_purpose::STANDARD.encode(bytes),
                    }),
                })
                .collect();
            json!({ "form": fields })
        }
    };
    Exchange {
        method: request.method.as_str().to_string(),
        path: url.path().to_string(),
        query,
        body,
        response: Value::Null,
    }
}

/// Answers from the exchanges recorded for the current call, each once.
#[derive(Default)]
struct Replay {
    expected: Mutex<Vec<(Exchange, bool)>>,
    unexpected: Mutex<Vec<String>>,
}

#[derive(Clone)]
struct Shared(Arc<Replay>);

impl Transport for Shared {
    fn send(
        &self,
        request: Request,
        _timeout: Duration,
    ) -> BoxFuture<'_, Result<Response, String>> {
        Box::pin(async move {
            let got = recorded_form(&request);
            let mut expected = self.0.expected.lock().unwrap();
            let hit = expected.iter_mut().find(|(e, used)| {
                !*used
                    && e.method == got.method
                    && e.path == got.path
                    && e.query == got.query
                    && same(&e.body, &got.body)
            });
            let Some((exchange, used)) = hit else {
                self.0.unexpected.lock().unwrap().push(got.describe());
                return Ok(Response {
                    status: 599,
                    location: None,
                    body: Ok(format!("UNSTUBBED {}", got.describe())),
                });
            };
            *used = true;
            let r = &exchange.response;
            if let Some(reason) = r.get("networkError").and_then(Value::as_str) {
                return Err(format!("TypeError: {reason}"));
            }
            let location = r["headers"]
                .as_object()
                .and_then(|h| h.iter().find(|(k, _)| k.eq_ignore_ascii_case("location")))
                .and_then(|(_, v)| v.as_str())
                .map(str::to_string);
            Ok(Response {
                status: r["status"].as_u64().unwrap_or(200) as u16,
                location,
                body: Ok(r["text"].as_str().unwrap_or_default().to_string()),
            })
        })
    }
}

fn toolsets(config: &Value) -> Option<Vec<Toolset>> {
    let names = config.get("toolsets")?.as_array()?;
    Some(
        TOOLSETS
            .into_iter()
            .filter(|t| names.iter().any(|n| n.as_str() == Some(t.name())))
            .collect(),
    )
}

struct Server {
    ctx: tools::Ctx,
    tools: Vec<tools::Tool>,
    replay: Arc<Replay>,
}

fn start(settings: &Value, live_catalog: bool, now_ms: i64) -> Server {
    let mut config = Config::new(BASE, TOKEN);
    config.default_project_id = settings.get("defaultProjectId").and_then(Value::as_i64);
    config.default_team_id = settings.get("defaultTeamId").and_then(Value::as_i64);
    config.toolsets = toolsets(settings);
    let sets = config.toolsets.clone().unwrap_or(TOOLSETS.to_vec());
    let replay = Arc::new(Replay::default());
    let catalog =
        (!live_catalog).then(|| crate::catalog::Provider::fixed(crate::catalog::snapshot()));
    let ctx = context::context(
        config,
        context::Options {
            transport: Box::new(Shared(replay.clone())),
            log: Arc::new(|_| {}),
            clock: Clock::fixed(now_ms),
            catalog,
            settle_delays_ms: vec![0],
        },
    );
    Server {
        ctx,
        tools: tools::tools_for(&sets),
        replay,
    }
}

fn result_matches(want: &Value, got: &tools::ToolResult) -> bool {
    let text = want["text"].as_str().unwrap_or_default();
    if want["isError"].as_bool().unwrap_or(false) != got.is_error {
        return false;
    }
    match (
        serde_json::from_str::<Value>(text),
        serde_json::from_str::<Value>(&got.text),
    ) {
        (Ok(a), Ok(b)) => same(&a, &b),
        _ => text == got.text,
    }
}

/// Replays one transcript file; `Err` lists every difference.
async fn replay_file(name: &str) -> Result<usize, Vec<String>> {
    let path = format!(
        "{}/tests/golden/transcripts/{name}.json",
        env!("CARGO_MANIFEST_DIR")
    );
    let file: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let now_ms = chrono::DateTime::parse_from_rfc3339(file["now"].as_str().unwrap())
        .unwrap()
        .timestamp_millis();
    let mut problems = Vec::new();
    let mut replayed = 0;
    for test in file["tests"].as_array().unwrap() {
        if test["direct"] == true || test["concurrent"] == true {
            continue;
        }
        replayed += 1;
        let mut label = test["name"].as_str().unwrap_or_default().to_string();
        let events = test["events"].as_array().unwrap();
        let mut current: Option<Server> = None;
        let mut i = 0;
        while i < events.len() {
            let e = &events[i];
            match e["type"].as_str().unwrap_or_default() {
                "test" => label = e["name"].as_str().unwrap_or_default().to_string(),
                "harness" => {
                    current = Some(start(&e["config"], e["liveCatalog"] == true, now_ms));
                }
                "listTools" => {
                    let s = current.as_ref().expect("a harness comes first");
                    let listed = tools::list(&s.tools);
                    if !same(&listed["tools"], &e["tools"]) {
                        problems.push(format!("{name} > {label}: tools/list differs"));
                    }
                }
                "call" => {
                    let s = current.as_ref().expect("a harness comes first");
                    let tool = e["tool"].as_str().unwrap_or_default();
                    let mut j = i + 1;
                    let mut batch = Vec::new();
                    while events[j]["type"] == "http" {
                        batch.push((Exchange::from_event(&events[j]), false));
                        j += 1;
                    }
                    let want = &events[j];
                    *s.replay.expected.lock().unwrap() = batch;
                    let got = tools::call(&s.ctx, &s.tools, tool, &e["args"]).await;
                    let mut lines = Vec::new();
                    for u in s.replay.unexpected.lock().unwrap().drain(..) {
                        lines.push(format!("  unexpected request: {u}"));
                    }
                    for (x, used) in s.replay.expected.lock().unwrap().iter() {
                        if !used {
                            lines.push(format!("  missing request: {}", x.describe()));
                        }
                    }
                    if !result_matches(want, &got) {
                        lines.push(format!(
                            "  result differs\n    want: {}\n    got:  {}",
                            want["text"].as_str().unwrap_or_default(),
                            got.text
                        ));
                    }
                    if !lines.is_empty() {
                        problems.push(format!(
                            "{name} > {label}: {tool} {}\n{}",
                            e["args"],
                            lines.join("\n")
                        ));
                    }
                    i = j;
                }
                _ => {}
            }
            i += 1;
        }
    }
    if problems.is_empty() {
        Ok(replayed)
    } else {
        Err(problems)
    }
}

macro_rules! replays {
    ($($test:ident => $file:literal),* $(,)?) => {$(
        #[tokio::test]
        async fn $test() {
            if let Err(problems) = replay_file($file).await {
                panic!("{} difference(s):\n\n{}", problems.len(), problems.join("\n\n"));
            }
        }
    )*};
}

replays! {
    generic => "tools/generic",
    extras => "tools/extras",
    workflow_read => "tools/workflow-read",
    workflow_write => "tools/workflow-write",
    workflow_delete => "tools/workflow-delete",
    acceptance => "tools/acceptance",
    coverage => "tools/coverage",
    server_tools => "server",
}

#[test]
fn same_ignores_key_order_and_number_form() {
    assert!(same(
        &json!({ "a": 6, "b": [1.5] }),
        &json!({ "b": [1.5], "a": 6.0 })
    ));
    assert!(!same(&json!({ "a": 6 }), &json!({ "a": 6, "b": null })));
}
