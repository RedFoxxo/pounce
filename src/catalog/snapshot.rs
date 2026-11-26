//! Regenerating `src/catalog/snapshot.json` from the configured instance.

use serde_json::Value;

use super::loader::load_catalog;
use crate::http::Logger;
use crate::http::v1::V1;
use crate::tools::Clock;

/// Reads the whole catalog and answers `snapshot.json`'s new content, formatted as
/// `JSON.stringify(data, null, 1)` plus a newline. Logs a one-line summary. `Err`
/// is the line to print before exiting with status 1.
pub async fn capture(v1: &V1, log: Logger, clock: Clock) -> Result<String, String> {
    let mut data = load_catalog(v1, &log, (clock.now_ms)(), None)
        .await
        .map_err(|e| format!("snapshot: failed: {}", e.message))?;
    // The snapshot ships in the package: never record which instance it came from.
    data.instance = "baseline snapshot".to_string();

    let listed: Vec<_> = data.resources.iter().filter(|r| r.listed).collect();
    let collections: Vec<_> = data.resources.iter().flat_map(|r| &r.collections).collect();
    log(&format!(
        "snapshot: {} listed ({} unavailable), {} unlisted, {} collections, {} addable/removable",
        listed.len(),
        listed.iter().filter(|r| !r.available).count(),
        data.resources.len() - listed.len(),
        collections.len(),
        collections
            .iter()
            .filter(|c| c.can_add || c.can_remove)
            .count()
    ));
    Ok(format!("{}\n", stringify(&data.to_json())))
}

/// `JSON.stringify(value, null, 1)`: one space per level, empty lists and objects inline.
fn stringify(value: &Value) -> String {
    let mut out = String::new();
    write(&mut out, value, 0);
    out
}

fn write(out: &mut String, value: &Value, depth: usize) {
    let newline = |out: &mut String, depth: usize| {
        out.push('\n');
        out.extend(std::iter::repeat_n(' ', depth));
    };
    match value {
        Value::Array(items) if !items.is_empty() => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                newline(out, depth + 1);
                write(out, item, depth + 1);
            }
            newline(out, depth);
            out.push(']');
        }
        Value::Object(map) if !map.is_empty() => {
            out.push('{');
            for (i, (key, item)) in map.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                newline(out, depth + 1);
                out.push_str(&serde_json::to_string(key).unwrap_or_default());
                out.push_str(": ");
                write(out, item, depth + 1);
            }
            newline(out, depth);
            out.push('}');
        }
        // Scalars, `[]` and `{}`: the catalog holds strings, booleans and lists only.
        other => out.push_str(&other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::catalog::tests::instance;

    #[tokio::test]
    async fn capture_writes_the_snapshot_as_the_typescript_cli_did() {
        let stub = instance();
        let lines = Arc::new(Mutex::new(Vec::new()));
        let sink = lines.clone();
        let log: Logger = Arc::new(move |l| sink.lock().unwrap().push(l.to_string()));
        let now = chrono::DateTime::parse_from_rfc3339("2026-09-23T11:17:59.713Z")
            .unwrap()
            .timestamp_millis();
        let text = capture(&stub.v1(), log, Clock::fixed(now)).await.unwrap();
        assert!(text.starts_with(
            "{\n \"instance\": \"baseline snapshot\",\n \"version\": \"2609.2.0.3492\",\n \"capturedAt\": \"2026-09-23T11:17:59.713Z\",\n \"resources\": [\n  {\n   \"name\": \"AcceptanceCriterion\",\n"
        ), "{text}");
        assert!(text.contains("   \"bases\": [],\n"));
        assert!(text.ends_with("}\n"));
        assert_eq!(
            lines.lock().unwrap().last().unwrap(),
            "snapshot: 5 listed (1 unavailable), 2 unlisted, 35 collections, 16 addable/removable"
        );
    }

    #[tokio::test]
    async fn capture_fails_when_the_index_cannot_be_read() {
        let stub = crate::catalog::tests::Stub::default();
        let e = capture(&stub.v1(), Arc::new(|_| {}), Clock::fixed(0))
            .await
            .unwrap_err();
        assert!(e.starts_with("snapshot: failed: 599: "), "{e}");
    }

    #[test]
    fn stringify_formats_as_json_stringify_with_one_space() {
        let file = include_str!("snapshot.json");
        let raw: Value = serde_json::from_str(file).unwrap();
        assert_eq!(format!("{}\n", stringify(&raw)), file);
    }
}
