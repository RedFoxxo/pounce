//! Everything a tool handler may use. Built once per server.

use std::sync::Arc;

use crate::catalog::Provider;
use crate::config::Config;
use crate::http::v1::V1;
use crate::http::v2::V2;
use crate::http::{Http, Logger, Transport};
use crate::resolve::directory::Directory;

/// The time, so tests can fix it: milliseconds since the epoch, and today's date
/// (`YYYY-MM-DD`) in the server's local time zone.
#[derive(Clone)]
pub struct Clock {
    pub now_ms: Arc<dyn Fn() -> i64 + Send + Sync>,
    pub today: Arc<dyn Fn() -> String + Send + Sync>,
}

impl Clock {
    pub fn system() -> Self {
        Clock {
            now_ms: Arc::new(|| chrono::Utc::now().timestamp_millis()),
            today: Arc::new(|| chrono::Local::now().format("%Y-%m-%d").to_string()),
        }
    }

    /// A clock that never moves: `now_ms`, and `today` as that instant's UTC date.
    #[cfg(test)]
    pub fn fixed(now_ms: i64) -> Self {
        let today = chrono::DateTime::from_timestamp_millis(now_ms)
            .map(|d| d.format("%Y-%m-%d").to_string())
            .unwrap_or_default();
        Clock {
            now_ms: Arc::new(move || now_ms),
            today: Arc::new(move || today.clone()),
        }
    }
}

pub struct Ctx {
    pub config: Config,
    pub http: Arc<Http>,
    pub v1: V1,
    pub v2: V2,
    /// The instance's metadata, read lazily; see `catalog::Provider`.
    pub catalog: Provider,
    /// Cached reference data for resolving names (users, roles, teams, projects, states, custom fields).
    pub directory: Directory,
    /// Pauses (ms) before re-checking a new card for assignments Targetprocess adds
    /// after creation (live: a default Product Owner appeared after the first check).
    pub settle_delays_ms: Vec<u64>,
    pub clock: Clock,
}

pub struct Options {
    pub transport: Box<dyn Transport>,
    pub log: Logger,
    pub clock: Clock,
    /// `None`: the live catalog, read lazily with the snapshot as fallback.
    pub catalog: Option<Provider>,
    pub settle_delays_ms: Vec<u64>,
}

pub fn context(config: Config, options: Options) -> Ctx {
    let http = Arc::new(Http::new(
        config.base_url.clone(),
        config.token.clone(),
        options.transport,
        options.log.clone(),
    ));
    let v1 = V1::new(http.clone());
    let catalog = options
        .catalog
        .unwrap_or_else(|| Provider::live(v1.clone(), options.log.clone(), options.clock.clone()));
    Ctx {
        directory: Directory::new(v1.clone(), options.clock.clone()),
        v2: V2::new(http.clone()),
        v1,
        http,
        catalog,
        config,
        settle_delays_ms: options.settle_delays_ms,
        clock: options.clock,
    }
}
