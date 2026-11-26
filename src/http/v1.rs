//! REST v1. Every method returns a `Res`; dates are ISO 8601. Only this module reads
//! a page's `Items`/`Next`.

use std::sync::Arc;

use serde_json::{Map, Value, json};

use super::{Http, Method, Query, Res, Spec, TpErr, encode_component};
use crate::format::dates::normalize_v1_dates;

/// Targetprocess ignores anything above 1000.
pub const PAGE_SIZE: usize = 1000;
pub const DEFAULT_LIST_LIMIT: usize = 5000;
pub const BULK_LIMIT: usize = 500;

#[derive(Debug, Clone, Default)]
pub struct ReadOptions {
    pub include: Option<String>,
    pub exclude: Option<String>,
    pub append: Option<String>,
    pub inner_take: Option<u32>,
}

#[derive(Debug, Clone, Default)]
pub struct ListOptions {
    pub read: ReadOptions,
    pub where_: Option<String>,
    pub order_by: Option<String>,
    pub order_by_desc: Option<String>,
    /// Hard cap on items returned. Default 5000. Reaching it with more pages left sets `truncated`.
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Default)]
pub struct WriteOptions {
    pub result_include: Option<String>,
    pub result_exclude: Option<String>,
    pub result_append: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ListResult {
    pub items: Vec<Value>,
    /// True when more items exist than were returned.
    pub truncated: bool,
}

/// A string literal for a v1 `where` clause; quotes and backslashes are backslash-escaped (**live**-verified).
pub fn v1_string(value: &str) -> String {
    format!("'{}'", value.replace('\\', "\\\\").replace('\'', "\\'"))
}

/// `UserStories/123/Tasks` → `/api/v1/UserStories/123/Tasks`, each segment encoded.
pub fn v1_path(parts: &[&str]) -> String {
    let segments: Vec<String> = parts
        .iter()
        .flat_map(|p| p.split('/'))
        .filter(|p| !p.is_empty() && *p != "." && *p != "..")
        .map(encode_component)
        .collect();
    format!("/api/v1/{}", segments.join("/"))
}

fn read_query(options: &ReadOptions) -> Query {
    Query::new()
        .with("format", "json")
        .set("include", options.include.as_ref())
        .set("exclude", options.exclude.as_ref())
        .set("append", options.append.as_ref())
        .set("innerTake", options.inner_take)
}

fn write_query(options: &WriteOptions) -> Query {
    Query::new()
        .with("format", "json")
        .with("resultFormat", "json")
        .set("resultInclude", options.result_include.as_ref())
        .set("resultExclude", options.result_exclude.as_ref())
        .set("resultAppend", options.result_append.as_ref())
}

/// Children removed before a collection removal stopped.
#[derive(Debug, Clone, PartialEq)]
pub struct Removal {
    pub removed: Vec<i64>,
    /// The failure that stopped it, if any.
    pub error: Option<TpErr>,
}

#[derive(Clone)]
pub struct V1 {
    pub http: Arc<Http>,
}

impl V1 {
    pub fn new(http: Arc<Http>) -> Self {
        V1 { http }
    }

    async fn normalized(&self, spec: Spec) -> Res<Value> {
        self.http.request(spec).await.map(normalize_v1_dates)
    }

    /// GET any path below `/api/v1`, e.g. `Users/LoggedUser`, `Context`, `UserStories/meta`.
    pub async fn get_path(&self, path: &str, query: Query) -> Res<Value> {
        let query = Query::new().with("format", "json").merge(&query);
        self.normalized(Spec::new(Method::Get, v1_path(&[path]), query))
            .await
    }

    /// GET the raw text of a path (for payloads a JSON parser cannot represent, e.g. `Index/meta`).
    pub async fn get_text(&self, path: &str, query: Query) -> Res<String> {
        let query = Query::new().with("format", "json").merge(&query);
        let r = self
            .http
            .request(Spec::new(Method::Get, v1_path(&[path]), query).text())
            .await?;
        Ok(r.as_str().unwrap_or_default().to_string())
    }

    pub async fn get(&self, collection: &str, id: i64, options: &ReadOptions) -> Res<Value> {
        let path = v1_path(&[collection, &id.to_string()]);
        self.normalized(Spec::new(Method::Get, path, read_query(options)))
            .await
    }

    /// GET a collection (or inner collection path), following pages until exhausted or `limit` is reached.
    pub async fn list(&self, path: &str, options: &ListOptions) -> Res<ListResult> {
        let limit = options.limit.unwrap_or(DEFAULT_LIST_LIMIT).max(1);
        let mut items: Vec<Value> = Vec::new();
        let mut skip = 0;
        loop {
            let take = PAGE_SIZE.min(limit - items.len());
            let query = read_query(&options.read)
                .set("where", options.where_.as_ref())
                .set("orderBy", options.order_by.as_ref())
                .set("orderByDesc", options.order_by_desc.as_ref())
                .with("take", take)
                .with("skip", skip);
            let page = self
                .http
                .request(Spec::new(Method::Get, v1_path(&[path]), query))
                .await?;
            let page_items = match page.get("Items") {
                Some(Value::Array(a)) => a.clone(),
                _ => Vec::new(),
            };
            let got = page_items.len();
            items.extend(page_items.into_iter().map(normalize_v1_dates));
            let next = page.get("Next").is_some_and(|n| match n {
                Value::String(s) => !s.is_empty(),
                Value::Null | Value::Bool(false) => false,
                _ => true,
            });
            if !(next && got > 0) {
                return Ok(ListResult {
                    items,
                    truncated: false,
                });
            }
            if items.len() >= limit {
                return Ok(ListResult {
                    items,
                    truncated: true,
                });
            }
            skip += got;
        }
    }

    /// POST a new entity. The body must not carry an `Id` (that would update instead).
    pub async fn create(
        &self,
        collection: &str,
        body: Value,
        options: &WriteOptions,
    ) -> Res<Value> {
        let spec = Spec::new(Method::Post, v1_path(&[collection]), write_query(options)).json(body);
        self.normalized(spec).await
    }

    /// POST an update to an existing entity.
    pub async fn update(
        &self,
        collection: &str,
        id: i64,
        body: Map<String, Value>,
        options: &WriteOptions,
    ) -> Res<Value> {
        let mut body = body;
        body.insert("Id".into(), json!(id));
        let path = v1_path(&[collection, &id.to_string()]);
        let spec = Spec::new(Method::Post, path, write_query(options)).json(Value::Object(body));
        self.normalized(spec).await
    }

    /// POST up to 500 entities to `/{collection}/bulk` (creates and updates).
    pub async fn bulk(
        &self,
        collection: &str,
        items: Vec<Value>,
        options: &WriteOptions,
    ) -> Res<Value> {
        let path = v1_path(&[collection, "bulk"]);
        let spec = Spec::new(Method::Post, path, write_query(options)).json(Value::Array(items));
        self.normalized(spec).await
    }

    pub async fn delete(&self, collection: &str, id: i64) -> Res<Value> {
        let path = v1_path(&[collection, &id.to_string()]);
        self.http
            .request(Spec::new(
                Method::Delete,
                path,
                Query::new().with("format", "json"),
            ))
            .await
    }

    pub async fn delete_bulk(&self, collection: &str, ids: &[i64]) -> Res<Value> {
        let path = v1_path(&[collection, "bulk"]);
        let body = Value::Array(ids.iter().map(|id| json!({ "Id": id })).collect());
        self.http
            .request(
                Spec::new(Method::Delete, path, Query::new().with("format", "json")).json(body),
            )
            .await
    }

    /// DELETE items from an inner collection, one `/{collection}/{id}/{inner}/{childId}`
    /// per child. The documented `?childrenIds=1,2` form answers 500 on a live
    /// instance; the path form works. Stops at the first failure and reports the
    /// children already removed.
    pub async fn remove_from_collection(
        &self,
        collection: &str,
        id: i64,
        inner: &str,
        child_ids: &[i64],
    ) -> Removal {
        let mut removed = Vec::new();
        for child in child_ids {
            let path = v1_path(&[collection, &id.to_string(), inner, &child.to_string()]);
            let r = self
                .http
                .request(Spec::new(
                    Method::Delete,
                    path,
                    Query::new().with("format", "json"),
                ))
                .await;
            if let Err(e) = r {
                return Removal {
                    removed,
                    error: Some(e),
                };
            }
            removed.push(*child);
        }
        Removal {
            removed,
            error: None,
        }
    }

    /// POST any path below `/api/v1` (e.g. `undelete`).
    pub async fn post_path(&self, path: &str, body: Value, query: Query) -> Res<Value> {
        let query = Query::new().with("format", "json").merge(&query);
        self.normalized(Spec::new(Method::Post, v1_path(&[path]), query).json(body))
            .await
    }
}
