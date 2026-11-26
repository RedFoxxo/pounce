//! Read-only checks against a real instance: the one in TP_BASE_URL, with TP_TOKEN.
//! Nothing is written. Ignored by a plain `cargo test`; run them with
//! `cargo test --bin pounce live:: -- --ignored --test-threads=1`.

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::{Value, json};

use crate::config::load_config;
use crate::http::Network;
use crate::tools::{self, Clock, TOOLSETS, Tool, context};

struct Live {
    ctx: tools::Ctx,
    tools: Vec<Tool>,
}

struct Answer {
    is_error: bool,
    text: String,
    json: Value,
}

impl Live {
    fn connect() -> Live {
        let env: HashMap<String, String> = std::env::vars().collect();
        let config = load_config(&env).expect("TP_BASE_URL and TP_TOKEN name the instance to read");
        let _ = rustls::crypto::ring::default_provider().install_default();
        let ctx = context::context(
            config,
            context::Options {
                transport: Box::new(Network::new()),
                log: Arc::new(|_| {}),
                clock: Clock::system(),
                catalog: None,
                settle_delays_ms: vec![1000, 2000],
            },
        );
        Live {
            ctx,
            tools: tools::tools_for(&TOOLSETS),
        }
    }

    async fn call(&self, name: &str, args: Value) -> Answer {
        let r = tools::call(&self.ctx, &self.tools, name, &args).await;
        let json = if r.is_error {
            Value::Null
        } else {
            serde_json::from_str(&r.text).unwrap_or(Value::Null)
        };
        Answer {
            is_error: r.is_error,
            text: r.text,
            json,
        }
    }

    async fn must(&self, name: &str, args: Value) -> Value {
        let r = self.call(name, args).await;
        assert!(!r.is_error, "{name}: {}", r.text);
        r.json
    }

    async fn newest_story(&self) -> i64 {
        let q = self
            .must(
                "read_query",
                json!({ "resource": "UserStory", "include": "Id", "orderBy": "CreateDate desc", "limit": 1 }),
            )
            .await;
        q["items"][0]["Id"]
            .as_i64()
            .expect("the instance has a user story")
    }
}

fn is_iso(value: &Value) -> bool {
    value
        .as_str()
        .is_some_and(|s| s.len() > 10 && s.as_bytes()[4] == b'-' && s.as_bytes()[10] == b'T')
}

#[tokio::test]
#[ignore = "reads the instance in TP_BASE_URL with TP_TOKEN"]
async fn layer_1_reads() {
    let c = Live::connect();
    let meta = c.must("read_meta", json!({})).await;
    assert_eq!(meta["source"], "live");
    assert!(meta["count"].as_u64().unwrap_or(0) > 100, "{meta}");

    let q = c
        .must(
            "read_query",
            json!({ "resource": "UserStory", "include": "Id,Name,CreateDate", "orderBy": "CreateDate desc", "limit": 3 }),
        )
        .await;
    assert_eq!(q["count"], 3);
    assert_eq!(q["truncated"], true);
    assert!(is_iso(&q["items"][0]["CreateDate"]), "{q}");

    let id = c.newest_story().await;
    let one = c
        .must(
            "read_get",
            json!({ "resource": "UserStory", "id": id, "include": "Id,Name,Assignments[Id,Role,GeneralUser]" }),
        )
        .await;
    assert_eq!(one["Id"], id);
    c.must(
        "read_collection",
        json!({ "resource": "UserStory", "id": id, "collection": "Tasks", "include": "Id,Name" }),
    )
    .await;

    let bad = c
        .call(
            "read_query",
            json!({ "resource": "UserStory", "where": "Nonsense eq", "limit": 1 }),
        )
        .await;
    assert!(bad.is_error);
    assert!(bad.text.contains("status: 400"), "{}", bad.text);
}

#[tokio::test]
#[ignore = "reads the instance in TP_BASE_URL with TP_TOKEN"]
async fn workflow_reads() {
    let c = Live::connect();
    let mine = c
        .must("read_my_work", json!({ "type": "UserStory", "limit": 5 }))
        .await;
    let story = match mine["cards"][0]["id"].as_i64() {
        Some(id) => id,
        None => c
            .must(
                "read_search",
                json!({ "type": "UserStory", "limit": 1, "includeClosed": true }),
            )
            .await["cards"][0]["id"]
            .as_i64()
            .expect("the instance has a user story"),
    };
    let tasks = c
        .must(
            "read_collection",
            json!({ "resource": "UserStory", "id": story, "collection": "Tasks", "include": "Id", "limit": 1 }),
        )
        .await;
    let task = tasks["items"][0]["Id"].as_i64();

    let card = c
        .must("read_card", json!({ "id": task.unwrap_or(story) }))
        .await;
    assert!(card["assignments"].is_array(), "{card}");
    assert!(card["roleEfforts"].is_array(), "{card}");
    if task.is_some() {
        assert_eq!(card["parents"][0]["type"], "UserStory");
    }

    let states = c.must("read_states", json!({ "id": story })).await;
    assert!(
        states["states"].as_array().is_some_and(|s| s.len() > 1),
        "{states}"
    );
    assert!(
        states["current"]["name"]
            .as_str()
            .is_some_and(|n| !n.is_empty())
    );

    for (name, args) in [
        ("read_people", json!({ "query": "a", "limit": 3 })),
        ("read_teams", json!({})),
        ("read_roles", json!({})),
        ("read_projects", json!({})),
        ("read_releases", json!({ "limit": 2 })),
        ("read_iterations", json!({})),
        ("read_custom_field_options", json!({ "id": story })),
        ("read_comments", json!({ "id": story })),
        ("read_relations", json!({ "id": story })),
        ("read_times", json!({ "user": "me", "limit": 3 })),
        ("read_attachments", json!({ "id": story })),
    ] {
        c.must(name, args).await;
    }

    let plans = c
        .must(
            "read_query",
            json!({ "resource": "TestPlan", "include": "Id", "orderBy": "Id desc", "limit": 1 }),
        )
        .await;
    if let Some(plan) = plans["items"][0]["Id"].as_i64() {
        let r = c.must("read_test_plan", json!({ "id": plan })).await;
        assert!(r["testCases"].is_array(), "{r}");
    }
}

#[tokio::test]
#[ignore = "reads the instance in TP_BASE_URL with TP_TOKEN"]
async fn v2_history_context_conversions_storage() {
    let c = Live::connect();
    let story = c.newest_story().await;

    let q = c
        .must(
            "read_v2_query",
            json!({ "entity": "userStory", "select": "{id,name,state:entityState.name}", "where": format!("(id=={story})") }),
        )
        .await;
    assert_eq!(q["items"][0]["id"], story);
    let agg = c
        .must(
            "read_v2_query",
            json!({ "entity": "userStory", "result": "{count:count}", "where": "(entityState.isFinal==false)" }),
        )
        .await;
    assert!(agg["result"]["count"].is_number(), "{agg}");

    let simple = c.must("read_history", json!({ "id": story })).await;
    assert_eq!(simple["kind"], "simple");
    let full = c
        .must(
            "read_history",
            json!({ "id": story, "full": true, "limit": 5 }),
        )
        .await;
    assert!(
        full["entries"][0]["date"]
            .as_str()
            .is_some_and(|d| d.len() > 4),
        "{full}"
    );

    let context = c.must("read_context", json!({})).await;
    assert!(
        context["Version"].as_str().is_some_and(|v| !v.is_empty()),
        "{context}"
    );
    c.must("read_conversions", json!({ "id": 6040 })).await;
    let deleted = c
        .must("read_deleted", json!({ "kind": "projects", "limit": 2 }))
        .await;
    assert!(
        deleted["items"][0]["deleteDate"]
            .as_str()
            .is_some_and(|d| d.len() > 4),
        "{deleted}"
    );

    let groups = c.must("read_storage", json!({})).await;
    assert!(
        groups["items"]
            .as_array()
            .is_some_and(|g| g.contains(&json!("boards"))),
        "{groups}"
    );
    let boards = c
        .must(
            "read_storage",
            json!({ "group": "boards", "select": "{key,publicData.name}", "limit": 2 }),
        )
        .await;
    let key = boards["items"][0]["key"].clone();
    let one = c
        .must("read_storage", json!({ "group": "boards", "key": key }))
        .await;
    assert_eq!(one["key"], key);
}
