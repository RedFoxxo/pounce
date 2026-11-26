//! Targetprocess dates: v1 sends `/Date(ms+offset)/`, pounce answers ISO 8601.

use std::sync::LazyLock;

use chrono::{DateTime, Local, NaiveDateTime, TimeZone};
use regex::Regex;
use serde_json::{Map, Value};

static V1_DATE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^/Date\((-?\d+)([+-])(\d{2})(\d{2})\)/$|^/Date\((-?\d+)\)/$").unwrap()
});
static DATE_ONLY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\d{4}-\d{2}-\d{2}$").unwrap());

/// `2026-09-18T00:00:00.000`, from milliseconds since the epoch, read as UTC.
fn iso_of(ms: i64) -> Option<String> {
    DateTime::from_timestamp_millis(ms).map(|d| d.format("%Y-%m-%dT%H:%M:%S%.3f").to_string())
}

/// `/Date(1789682400000+0200)/` → `2026-09-18T00:00:00.000+02:00`. The offset
/// Targetprocess sends is kept, so date-only values stored as local midnight
/// (release and iteration dates, time entries) keep their calendar day. Values
/// without an offset become UTC (`Z`). Anything else is returned unchanged.
pub fn v1_date_to_iso(value: &str) -> String {
    let Some(m) = V1_DATE.captures(value) else {
        return value.to_string();
    };
    if let Some(ms) = m.get(5) {
        return ms
            .as_str()
            .parse::<i64>()
            .ok()
            .and_then(iso_of)
            .map(|iso| format!("{iso}Z"))
            .unwrap_or_else(|| value.to_string());
    }
    let (Ok(ms), Ok(hours), Ok(minutes)) = (
        m[1].parse::<i64>(),
        m[3].parse::<i64>(),
        m[4].parse::<i64>(),
    ) else {
        return value.to_string();
    };
    let sign = if &m[2] == "-" { -1 } else { 1 };
    let offset_ms = sign * (hours * 60 + minutes) * 60_000;
    match iso_of(ms + offset_ms) {
        Some(local) => format!("{local}{}{}:{}", &m[2], &m[3], &m[4]),
        None => value.to_string(),
    }
}

/// Deep-converts every v1 date string in a parsed payload to ISO 8601.
pub fn normalize_v1_dates(value: Value) -> Value {
    match value {
        Value::String(s) => Value::String(v1_date_to_iso(&s)),
        Value::Array(items) => Value::Array(items.into_iter().map(normalize_v1_dates).collect()),
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(k, v)| (k, normalize_v1_dates(v)))
                .collect::<Map<String, Value>>(),
        ),
        other => other,
    }
}

/// Milliseconds since the epoch of a date/time string, as `Date.parse` reads the
/// forms Targetprocess and models use: RFC 3339, or a local date-time without an offset.
fn parse_instant(text: &str) -> Option<i64> {
    if let Ok(d) = DateTime::parse_from_rfc3339(text) {
        return Some(d.timestamp_millis());
    }
    for format in [
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%dT%H:%M",
    ] {
        if let Ok(naive) = NaiveDateTime::parse_from_str(text, format) {
            return Local
                .from_local_datetime(&naive)
                .earliest()
                .map(|d| d.timestamp_millis());
        }
    }
    None
}

/// Whether a requested date/time and a read-back value mean the same thing:
/// a date-only request (`YYYY-MM-DD`) compares calendar days, anything else
/// compares instants to the second.
pub fn same_date_value(want: &str, got: &str) -> bool {
    if DATE_ONLY.is_match(want) {
        return got.get(..10) == Some(want);
    }
    match (parse_instant(want), parse_instant(got)) {
        (Some(a), Some(b)) => (a - b).abs() < 1000,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_offset_targetprocess_sent() {
        assert_eq!(
            v1_date_to_iso("/Date(1789682400000+0200)/"),
            "2026-09-18T00:00:00.000+02:00"
        );
        assert_eq!(v1_date_to_iso("/Date(0)/"), "1970-01-01T00:00:00.000Z");
        assert_eq!(v1_date_to_iso("not a date"), "not a date");
    }

    #[test]
    fn normalize_v1_dates_keeps_a_proto_key_as_data() {
        let input: Value =
            serde_json::from_str(r#"{"__proto__":{"x":"/Date(0)/"},"y":1}"#).unwrap();
        assert_eq!(
            normalize_v1_dates(input).to_string(),
            r#"{"__proto__":{"x":"1970-01-01T00:00:00.000Z"},"y":1}"#
        );
    }
}
