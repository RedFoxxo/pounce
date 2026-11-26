//! REST v2 (read-only): camelCase, null fields omitted (absence means null), ISO
//! dates requested with `isoDate=true`. Filter syntax (`==`, `and`, `or`) differs
//! from v1; never share filter strings. Only this module reads `items`/`next`.

use std::sync::Arc;

use serde_json::Value;

use super::v1::ListResult;
use super::{Http, Method, Query, Res, Spec, encode_component};
use crate::format::dates::normalize_v1_dates;

const PAGE: usize = 1000;

#[derive(Debug, Clone, Default)]
pub struct QueryOptions {
    pub select: Option<String>,
    pub where_: Option<String>,
    pub order_by: Option<String>,
    pub filter: Option<String>,
    pub include_deleted: bool,
    /// Hard cap on items returned. Default 1000.
    pub limit: Option<usize>,
}

fn seg(part: &str) -> String {
    // WHATWG URL resolves "." and ".." even when encoded; never let them through.
    if part == "." || part == ".." {
        encode_component(&format!("_{part}"))
    } else {
        encode_component(part)
    }
}

#[derive(Clone)]
pub struct V2 {
    pub http: Arc<Http>,
}

impl V2 {
    pub fn new(http: Arc<Http>) -> Self {
        V2 { http }
    }

    /// `/api/v2/{entity}` fully paged; the query is sent unchanged on every page (TP's `next` drops some parameters).
    pub async fn query(&self, entity: &str, options: &QueryOptions) -> Res<ListResult> {
        self.query_at("/api/v2", entity, options).await
    }

    async fn query_at(&self, base: &str, entity: &str, options: &QueryOptions) -> Res<ListResult> {
        let limit = options.limit.unwrap_or(PAGE).max(1);
        let mut items: Vec<Value> = Vec::new();
        let mut skip = 0;
        loop {
            let take = PAGE.min(limit - items.len());
            let query = Query::new()
                .set("select", options.select.as_ref())
                .set("where", options.where_.as_ref())
                // Skip-based paging needs a stable order; v2 has no documented default.
                .with("orderBy", options.order_by.as_deref().unwrap_or("id"))
                .set("filter", options.filter.as_ref())
                .set("includeDeleted", options.include_deleted.then_some("true"))
                .with("isoDate", "true")
                .with("take", take)
                .with("skip", skip);
            let path = format!("{base}/{}", seg(entity));
            let page = self
                .http
                .request(Spec::new(Method::Get, path, query))
                .await?;
            let got = match page.get("items") {
                Some(Value::Array(a)) => a.clone(),
                _ => Vec::new(),
            };
            let count = got.len();
            items.extend(got.into_iter().map(normalize_v1_dates));
            let next = page.get("next").is_some_and(|n| match n {
                Value::String(s) => !s.is_empty(),
                Value::Null | Value::Bool(false) => false,
                _ => true,
            });
            if !next || count == 0 {
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
            skip += count;
        }
    }

    /// An aggregation (`result=`) returns one object rather than items.
    pub async fn aggregate(
        &self,
        entity: &str,
        result: &str,
        where_: Option<&str>,
        filter: Option<&str>,
    ) -> Res<Value> {
        let query = Query::new()
            .with("result", result)
            .set("where", where_)
            .set("filter", filter)
            .with("isoDate", "true");
        let path = format!("/api/v2/{}", seg(entity));
        self.http
            .request(Spec::new(Method::Get, path, query))
            .await
            .map(normalize_v1_dates)
    }
}
