//! The HTTP layer against a stub transport (`tests/http.test.ts` in the TypeScript build).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Map, Value, json};

use super::redact::redact;
use super::v1::{ListOptions, ReadOptions, V1, WriteOptions};
use super::{Body, BoxFuture, Http, Request, Response, Transport, encode_component};
use crate::format::dates::{normalize_v1_dates, v1_date_to_iso};

const TOKEN: &str = "SECRETTOKEN123==";

type Answerer = Box<dyn Fn(&Sent) -> Option<Result<Response, String>> + Send + Sync>;

/// A request as the stub saw it.
#[derive(Debug, Clone)]
struct Sent {
    method: String,
    path: String,
    query: Vec<(String, String)>,
    headers: Vec<(&'static str, String)>,
    body: Value,
}

impl Sent {
    fn param(&self, key: &str) -> Option<&str> {
        self.query
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }
}

#[derive(Default)]
struct State {
    routes: Mutex<Vec<Answerer>>,
    calls: Mutex<Vec<Sent>>,
}

/// Routes tried in order; an unmatched request answers 599.
#[derive(Clone, Default)]
struct Stub(Arc<State>);

fn reply(status: u16, body: impl Into<String>) -> Result<Response, String> {
    Ok(Response {
        status,
        location: None,
        body: Ok(body.into()),
    })
}

impl Stub {
    fn on(
        self,
        f: impl Fn(&Sent) -> Option<Result<Response, String>> + Send + Sync + 'static,
    ) -> Self {
        self.0.routes.lock().unwrap().push(Box::new(f));
        self
    }

    /// `method path` (and optional `key=value` query matches) answers `status, body`.
    fn route(
        self,
        method: &'static str,
        path: &'static str,
        query: &'static [(&str, &str)],
        status: u16,
        body: Value,
    ) -> Self {
        let text = match &body {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        self.on(move |s| {
            (s.method == method
                && s.path.eq_ignore_ascii_case(path)
                && query.iter().all(|(k, v)| s.param(k) == Some(v)))
            .then(|| reply(status, text.clone()))
        })
    }

    fn calls(&self) -> Vec<Sent> {
        self.0.calls.lock().unwrap().clone()
    }

    fn v1(&self, log: Arc<Mutex<Vec<String>>>) -> V1 {
        let http = Http::new(
            "https://tp.example".into(),
            TOKEN.into(),
            Box::new(self.clone()),
            Arc::new(move |line| log.lock().unwrap().push(line.to_string())),
        );
        V1::new(Arc::new(http))
    }
}

impl Transport for Stub {
    fn send(
        &self,
        request: Request,
        _timeout: Duration,
    ) -> BoxFuture<'_, Result<Response, String>> {
        Box::pin(async move {
            let url = url::Url::parse(&request.url).unwrap();
            let sent = Sent {
                method: request.method.as_str().to_string(),
                path: url.path().to_string(),
                query: url
                    .query_pairs()
                    .map(|(k, v)| (k.into_owned(), v.into_owned()))
                    .collect(),
                headers: request.headers.clone(),
                body: match &request.body {
                    Body::Json(v) => v.clone(),
                    _ => Value::Null,
                },
            };
            self.0.calls.lock().unwrap().push(sent.clone());
            let routes = self.0.routes.lock().unwrap();
            routes
                .iter()
                .find_map(|r| r(&sent))
                .unwrap_or_else(|| reply(599, format!("UNSTUBBED {} {}", sent.method, sent.path)))
        })
    }
}

fn client(stub: &Stub) -> V1 {
    stub.v1(Arc::new(Mutex::new(Vec::new())))
}

#[test]
fn redact_removes_the_token_from_urls_and_bodies() {
    assert_eq!(
        redact(
            &format!("https://x/api?format=json&access_token={TOKEN}&take=1"),
            TOKEN
        ),
        "https://x/api?format=json&access_token=***&take=1"
    );
    assert_eq!(
        redact(
            &format!("echo {TOKEN} and {}", encode_component(TOKEN)),
            TOKEN
        ),
        "echo *** and ***"
    );
}

#[test]
fn converts_v1_dates_to_iso_8601() {
    assert_eq!(
        v1_date_to_iso("/Date(1789125687000+0200)/"),
        "2026-09-11T13:21:27.000+02:00"
    );
    assert_eq!(
        v1_date_to_iso("/Date(1789682400000+0200)/"),
        "2026-09-18T00:00:00.000+02:00"
    );
    assert_eq!(v1_date_to_iso("not a date"), "not a date");
    assert_eq!(
        normalize_v1_dates(json!({ "a": ["/Date(0)/"], "b": { "c": "/Date(0+0000)/" }, "d": 1 })),
        json!({ "a": ["1970-01-01T00:00:00.000Z"], "b": { "c": "1970-01-01T00:00:00.000+00:00" }, "d": 1 })
    );
}

#[tokio::test]
async fn sends_the_token_format_json_and_accept_and_logs_a_redacted_line() {
    let stub = Stub::default().route(
        "GET",
        "/api/v1/Users/LoggedUser",
        &[],
        200,
        json!({ "Id": 2286 }),
    );
    let log = Arc::new(Mutex::new(Vec::new()));
    let result = stub
        .v1(log.clone())
        .get_path("Users/LoggedUser", Default::default())
        .await;
    assert_eq!(result, Ok(json!({ "Id": 2286 })));
    let call = &stub.calls()[0];
    assert_eq!(call.param("access_token"), Some(TOKEN));
    assert_eq!(call.param("format"), Some("json"));
    assert!(
        call.headers
            .contains(&("Accept", "application/json".into()))
    );
    let log = log.lock().unwrap().clone();
    assert_eq!(
        log,
        ["GET https://tp.example/api/v1/Users/LoggedUser?format=json&access_token=***"]
    );
    assert!(!log.join(",").contains(TOKEN));
}

#[tokio::test]
async fn carries_targetprocess_json_error_message_and_the_raw_body() {
    let body = json!({
        "Status": "NotFound",
        "Message": "General with Id 36410 not found or access is forbidden.",
        "Type": "Presentational",
        "ErrorId": "e2a27ff8",
    });
    let stub = Stub::default().route("GET", "/api/v1/Generals/36410", &[], 404, body.clone());
    let e = client(&stub)
        .get("Generals", 36410, &ReadOptions::default())
        .await
        .unwrap_err();
    assert_eq!(e.status, 404);
    assert_eq!(
        e.message,
        "404: General with Id 36410 not found or access is forbidden."
    );
    assert_eq!(serde_json::from_str::<Value>(&e.body).unwrap(), body);
}

#[tokio::test]
async fn reads_the_message_out_of_an_xml_error() {
    let xml = "<Error>\n  <Status>BadRequest</Status>\n  <Message>Error during parameters parsing.</Message>\n</Error>";
    let stub = Stub::default().route("GET", "/api/v1/UserStories", &[], 400, json!(xml));
    let e = client(&stub)
        .list("UserStories", &ListOptions::default())
        .await
        .unwrap_err();
    assert_eq!(e.message, "400: Error during parameters parsing.");
}

#[tokio::test]
async fn redacts_the_token_from_error_bodies() {
    let stub = Stub::default().route(
        "GET",
        "/api/v1/Bugs/1",
        &[],
        401,
        json!(format!("bad token {TOKEN}")),
    );
    let e = client(&stub)
        .get("Bugs", 1, &ReadOptions::default())
        .await
        .unwrap_err();
    assert_eq!(e.body, "bad token ***");
    assert!(!e.message.contains(TOKEN));
}

#[tokio::test]
async fn reports_network_failures_with_status_0() {
    let stub = Stub::default().on(|_| Some(Err("TypeError: fetch failed".into())));
    let e = client(&stub)
        .get("Bugs", 1, &ReadOptions::default())
        .await
        .unwrap_err();
    assert_eq!(e.status, 0);
    assert!(
        e.message.contains("No response from Targetprocess"),
        "{}",
        e.message
    );
}

#[tokio::test]
async fn treats_a_redirect_as_a_failure_instead_of_following_it() {
    let stub = Stub::default().on(|_| {
        Some(Ok(Response {
            status: 302,
            location: Some("/404.aspx".into()),
            body: Ok(String::new()),
        }))
    });
    let e = client(&stub)
        .get("Bugs", 1, &ReadOptions::default())
        .await
        .unwrap_err();
    assert_eq!(e.status, 302);
}

#[tokio::test]
async fn rejects_an_html_page_where_json_was_expected() {
    let stub = Stub::default().route(
        "GET",
        "/api/v1/Bugs/1",
        &[],
        200,
        json!("<!DOCTYPE html><html></html>"),
    );
    let e = client(&stub)
        .get("Bugs", 1, &ReadOptions::default())
        .await
        .unwrap_err();
    assert!(e.message.contains("got an HTML page"), "{}", e.message);
}

#[tokio::test]
async fn encodes_path_segments_and_drops_dot_segments() {
    let stub = Stub::default().on(|_| Some(reply(200, "{}")));
    let _ = client(&stub)
        .get("Bugs/../Users?x", 1, &ReadOptions::default())
        .await;
    assert_eq!(stub.calls()[0].path, "/api/v1/Bugs/Users%3Fx/1");
}

fn page(ids: impl IntoIterator<Item = i64>, next: bool) -> Value {
    let items: Vec<Value> = ids
        .into_iter()
        .map(|id| json!({ "ResourceType": "User", "Id": id, "CreateDate": "/Date(0+0000)/" }))
        .collect();
    let mut page = Map::new();
    if next {
        page.insert(
            "Next".into(),
            json!("https://tp.example/api/v1/Users/?take=1000&skip=1000"),
        );
    }
    page.insert("Items".into(), json!(items));
    Value::Object(page)
}

#[tokio::test]
async fn pages_with_take_1000_until_next_is_absent() {
    let stub = Stub::default()
        .route(
            "GET",
            "/api/v1/Users",
            &[("skip", "0"), ("take", "1000")],
            200,
            page(1..=1000, true),
        )
        .route(
            "GET",
            "/api/v1/Users",
            &[("skip", "1000"), ("take", "1000")],
            200,
            page([1001, 1002], false),
        );
    let options = ListOptions {
        where_: Some("(IsActive eq 'true')".into()),
        ..Default::default()
    };
    let r = client(&stub).list("Users", &options).await.unwrap();
    assert_eq!(r.items.len(), 1002);
    assert!(!r.truncated);
    assert_eq!(r.items[0]["CreateDate"], "1970-01-01T00:00:00.000+00:00");
    let wheres: Vec<Option<String>> = stub
        .calls()
        .iter()
        .map(|c| c.param("where").map(str::to_string))
        .collect();
    assert_eq!(
        wheres,
        [
            Some("(IsActive eq 'true')".to_string()),
            Some("(IsActive eq 'true')".to_string())
        ]
    );
}

#[tokio::test]
async fn reports_truncated_when_the_cap_is_reached_with_pages_left() {
    let stub = Stub::default().route(
        "GET",
        "/api/v1/Users",
        &[("take", "3")],
        200,
        page([1, 2, 3], true),
    );
    let options = ListOptions {
        limit: Some(3),
        ..Default::default()
    };
    let r = client(&stub).list("Users", &options).await.unwrap();
    assert!(r.truncated);
    assert_eq!(stub.calls().len(), 1);
}

#[tokio::test]
async fn passes_inner_take_and_order_by_desc_through() {
    let stub = Stub::default().route("GET", "/api/v1/UserStories", &[], 200, page([1], false));
    let options = ListOptions {
        read: ReadOptions {
            include: Some("[Id,Tasks]".into()),
            inner_take: Some(1000),
            ..Default::default()
        },
        order_by_desc: Some("CreateDate".into()),
        ..Default::default()
    };
    client(&stub).list("UserStories", &options).await.unwrap();
    let call = &stub.calls()[0];
    assert_eq!(call.param("innerTake"), Some("1000"));
    assert_eq!(call.param("orderByDesc"), Some("CreateDate"));
    assert_eq!(call.param("include"), Some("[Id,Tasks]"));
}

#[tokio::test]
async fn returns_the_failure_of_any_page() {
    let stub = Stub::default()
        .route(
            "GET",
            "/api/v1/Users",
            &[("skip", "0")],
            200,
            page([1], true),
        )
        .route(
            "GET",
            "/api/v1/Users",
            &[("skip", "1")],
            500,
            json!({ "Message": "boom" }),
        );
    let e = client(&stub)
        .list("Users", &ListOptions::default())
        .await
        .unwrap_err();
    assert_eq!((e.status, e.message.as_str()), (500, "500: boom"));
}

#[tokio::test]
async fn update_posts_the_id_in_the_body_and_asks_for_a_json_result() {
    let stub = Stub::default().route(
        "POST",
        "/api/v1/UserStories/5",
        &[],
        200,
        json!({ "Id": 5, "Name": "x" }),
    );
    let body = json!({ "Name": "x" }).as_object().unwrap().clone();
    let options = WriteOptions {
        result_include: Some("[Id,Name]".into()),
        ..Default::default()
    };
    client(&stub)
        .update("UserStories", 5, body, &options)
        .await
        .unwrap();
    let call = &stub.calls()[0];
    assert_eq!(call.body, json!({ "Name": "x", "Id": 5 }));
    assert_eq!(call.param("resultFormat"), Some("json"));
    assert_eq!(call.param("resultInclude"), Some("[Id,Name]"));
}

#[tokio::test]
async fn remove_from_collection_deletes_each_child_by_path() {
    let stub = Stub::default().on(|s| (s.method == "DELETE").then(|| reply(200, "")));
    let r = client(&stub)
        .remove_from_collection("UserStories", 5, "Assignments", &[7, 8])
        .await;
    let paths: Vec<String> = stub.calls().iter().map(|c| c.path.clone()).collect();
    assert_eq!(
        paths,
        [
            "/api/v1/UserStories/5/Assignments/7",
            "/api/v1/UserStories/5/Assignments/8"
        ]
    );
    assert_eq!((r.removed, r.error), (vec![7, 8], None));
}

#[tokio::test]
async fn remove_from_collection_reports_the_children_removed_before_a_failure() {
    let stub = Stub::default()
        .route(
            "DELETE",
            "/api/v1/UserStories/5/Assignments/7",
            &[],
            200,
            json!(""),
        )
        .route(
            "DELETE",
            "/api/v1/UserStories/5/Assignments/8",
            &[],
            500,
            json!({ "Message": "boom" }),
        );
    let r = client(&stub)
        .remove_from_collection("UserStories", 5, "Assignments", &[7, 8, 9])
        .await;
    assert!(r.error.is_some());
    assert_eq!(r.removed, [7]);
    assert_eq!(stub.calls().len(), 2);
}

#[tokio::test]
async fn delete_bulk_sends_ids() {
    let stub = Stub::default().route("DELETE", "/api/v1/Tasks/bulk", &[], 200, json!(""));
    client(&stub).delete_bulk("Tasks", &[1, 2]).await.unwrap();
    assert_eq!(stub.calls()[0].body, json!([{ "Id": 1 }, { "Id": 2 }]));
}

#[tokio::test]
async fn redaction_never_corrupts_json_bodies_or_rewrites_user_links() {
    let body = r#"{"Id":1,"Description":"<a href=\"https://x.io/?token=abc\">link</a>"}"#;
    let stub = Stub::default().route("GET", "/api/v1/Bugs/1", &[], 200, json!(body));
    let r = client(&stub)
        .get("Bugs", 1, &ReadOptions::default())
        .await
        .unwrap();
    assert_eq!(
        r["Description"],
        r#"<a href="https://x.io/?token=abc">link</a>"#
    );
}

#[tokio::test]
async fn still_removes_the_exact_token_from_bodies() {
    let stub = Stub::default().route(
        "GET",
        "/api/v1/Bugs/1",
        &[],
        200,
        json!({ "Echo": format!("access_token={TOKEN}") }),
    );
    let r = client(&stub)
        .get("Bugs", 1, &ReadOptions::default())
        .await
        .unwrap();
    assert_eq!(r["Echo"], "access_token=***");
}

#[tokio::test]
async fn a_write_whose_response_body_cannot_be_read_says_it_was_most_likely_applied() {
    let stub = Stub::default().on(|_| {
        Some(Ok(Response {
            status: 201,
            location: None,
            body: Err("socket hang up".into()),
        }))
    });
    let e = client(&stub)
        .create("Bugs", json!({ "Name": "x" }), &WriteOptions::default())
        .await
        .unwrap_err();
    assert!(
        e.message
            .contains("answered 201, so the write was most likely applied: check before retrying"),
        "{}",
        e.message
    );
}

#[tokio::test]
async fn caps_large_error_bodies() {
    let page = format!("<html>{}</html>", "x".repeat(10_000));
    let stub = Stub::default().route("GET", "/api/v1/Bugs/1", &[], 500, json!(page));
    let e = client(&stub)
        .get("Bugs", 1, &ReadOptions::default())
        .await
        .unwrap_err();
    assert!(e.body.len() < 4100);
    assert!(e.body.ends_with("more characters)"), "{}", e.body);
}
