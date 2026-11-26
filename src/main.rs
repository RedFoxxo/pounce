//! pounce: an MCP server for Targetprocess.

mod catalog;
mod config;
mod domain;
mod format;
mod http;
mod mcp;
mod resolve;
mod schema;
mod tools;

#[cfg(test)]
mod live;
#[cfg(test)]
mod replay;

use std::collections::HashMap;
use std::sync::Arc;

use http::{Logger, Network};
use tools::{Clock, TOOLSETS, context};

const HELP: &str = "pounce: MCP server for Targetprocess, spoken over stdio by an MCP client.

Settings come from the environment:
  TP_BASE_URL            instance root, e.g. https://yourcompany.tpondemand.com (required)
  TP_TOKEN               a Targetprocess access token (required)
  TP_DEFAULT_PROJECT_ID  project for cards with nothing to inherit one from
  TP_DEFAULT_TEAM_ID     team for new cards when none is given
  POUNCE_TOOLSETS        cards, raw, admin (default: all three)

Options:
  --version   print the version
  --help      print this help
  --snapshot  (maintainers) rewrite src/catalog/snapshot.json from TP_BASE_URL, read-only

https://github.com/RedFoxxo/pounce";

/// Milliseconds before re-checking a new card for assignments Targetprocess adds late.
const SETTLE_DELAYS_MS: [u64; 2] = [1000, 2000];

// One thread: pounce waits on the network and stdio, it does not compute.
#[tokio::main(flavor = "current_thread")]
async fn main() {
    let snapshot = match std::env::args().nth(1).as_deref() {
        None => false,
        Some("--version" | "-V") => return println!("pounce {}", env!("CARGO_PKG_VERSION")),
        Some("--help" | "-h") => return println!("{HELP}"),
        Some("--snapshot") => true,
        Some(other) => {
            eprintln!("pounce: unknown option {other}\n\n{HELP}");
            std::process::exit(2);
        }
    };

    let env: HashMap<String, String> = std::env::vars_os()
        .filter_map(|(k, v)| Some((k.into_string().ok()?, v.into_string().ok()?)))
        .collect();
    let config = match config::load_config(&env) {
        Ok(config) => config,
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(1);
        }
    };

    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("no other TLS provider is installed");
    // stdout belongs to the MCP transport; everything else goes to stderr.
    let log: Logger = Arc::new(|line| eprintln!("pounce: {line}"));
    if snapshot {
        return write_snapshot(config, log).await;
    }
    let sets = config.toolsets.clone().unwrap_or(TOOLSETS.to_vec());
    let tools = Arc::new(tools::tools_for(&sets));
    let ctx = Arc::new(context::context(
        config,
        context::Options {
            transport: Box::new(Network::new()),
            log,
            clock: Clock::system(),
            catalog: None,
            settle_delays_ms: SETTLE_DELAYS_MS.to_vec(),
        },
    ));
    eprintln!("pounce: ready on stdio");
    mcp::serve(ctx, tools).await;
}

/// Reads the whole catalog of the configured instance and rewrites the committed snapshot.
async fn write_snapshot(config: config::Config, log: Logger) {
    let http = http::Http::new(
        config.base_url,
        config.token,
        Box::new(Network::new()),
        log.clone(),
    );
    let v1 = http::v1::V1::new(Arc::new(http));
    match catalog::snapshot::capture(&v1, log, Clock::system()).await {
        Ok(json) => {
            if let Err(e) = std::fs::write("src/catalog/snapshot.json", json) {
                eprintln!("snapshot: failed: cannot write src/catalog/snapshot.json ({e})");
                std::process::exit(1);
            }
        }
        Err(line) => {
            eprintln!("{line}");
            std::process::exit(1);
        }
    }
}
