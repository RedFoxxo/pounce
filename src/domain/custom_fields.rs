//! Custom field values: checked against the field's type and options before
//! anything is sent, and compared with the card read back afterwards.

use serde_json::{Map, Value, json};

use crate::catalog::js_trim;
use crate::format::dates::same_date_value;
use crate::format::shape::number;
use crate::resolve::directory::{TpCustomField, custom_field_options};
use crate::resolve::matching::{Reason, Resolved, Unresolved};
use crate::tools::Ctx;
use crate::tools::generic::validate::{js_json, js_string};

/// One value to write, as v1 takes it in `CustomFields`.
#[derive(Debug, Clone, PartialEq)]
pub struct CustomFieldValue {
    /// The field's exact name.
    pub name: String,
    pub value: Value,
}

impl CustomFieldValue {
    /// `{Name, Value}`.
    pub fn to_json(&self) -> Value {
        json!({ "Name": self.name, "Value": self.value })
    }
}

/// `Number(text)` for a string that is not blank: `NaN` when JavaScript would not parse it.
fn js_to_number(text: &str) -> f64 {
    let t = js_trim(text);
    match t {
        "Infinity" | "+Infinity" => return f64::INFINITY,
        "-Infinity" => return f64::NEG_INFINITY,
        _ => {}
    }
    let radix = match t.get(..2) {
        Some("0x" | "0X") => Some(16),
        Some("0o" | "0O") => Some(8),
        Some("0b" | "0B") => Some(2),
        _ => None,
    };
    if let Some(radix) = radix {
        return u128::from_str_radix(&t[2..], radix).map_or(f64::NAN, |n| n as f64);
    }
    if t.chars()
        .all(|c| c.is_ascii_digit() || matches!(c, '+' | '-' | '.' | 'e' | 'E'))
    {
        t.parse().unwrap_or(f64::NAN)
    } else {
        f64::NAN
    }
}

/// Checks one value against the field's type; dropdown values must be one of its
/// options (matched case-insensitively, answered in the option's own spelling).
/// `Ok` is the value to send; `Err` the reason it is refused.
pub fn check_custom_value(field: &TpCustomField, value: &Value) -> Result<Value, String> {
    let system = field.is_system == Some(true);
    let calculated = matches!(&field.calculation_model, Some(Some(m)) if !m.is_empty());
    if system || calculated {
        return Err(format!(
            "{} is {} field; only Targetprocess sets it",
            field.name,
            if system { "a system" } else { "a calculated" }
        ));
    }
    if value.is_null() {
        return Ok(Value::Null);
    }
    let kind = field.field_type.as_deref().unwrap_or("").to_lowercase();
    if let Some(options) = custom_field_options(field) {
        let wanted: Vec<String> = match value {
            Value::Array(list) => list.iter().map(js_string).collect(),
            Value::String(s) if kind == "multipleselectionlist" => {
                s.split(',').map(str::to_string).collect()
            }
            other => vec![js_string(other)],
        };
        // A string is trimmed; anything else is taken as `String(value)`.
        let trims = |i: usize| match value {
            Value::Array(list) => list[i].is_string(),
            other => other.is_string(),
        };
        let mut canonical: Vec<&str> = Vec::with_capacity(wanted.len());
        for (i, w) in wanted.iter().enumerate() {
            let s = if trims(i) { js_trim(w) } else { w.as_str() };
            let lower = s.to_lowercase();
            let Some(hit) = options.iter().find(|o| o.to_lowercase() == lower) else {
                return Err(format!(
                    "\"{s}\" is not an option of {}; options: {}",
                    field.name,
                    options.join(", ")
                ));
            };
            canonical.push(hit);
        }
        if kind == "dropdown" {
            return match canonical[..] {
                [one] => Ok(json!(one)),
                _ => Err(format!("{} takes exactly one option", field.name)),
            };
        }
        return Ok(json!(canonical.join(",")));
    }
    match kind.as_str() {
        "number" | "money" | "percent" => {
            let n = match value {
                Value::String(s) if !js_trim(s).is_empty() => js_to_number(s),
                Value::Number(n) => return Ok(Value::Number(n.clone())),
                _ => f64::NAN,
            };
            if n.is_nan() {
                return Err(format!(
                    "{} is a {} field; got {}",
                    field.name,
                    field.field_type.as_deref().unwrap_or(""),
                    js_json(value)
                ));
            }
            Ok(number(n))
        }
        "checkbox" => match value {
            Value::Bool(_) => Ok(value.clone()),
            _ => Err(format!("{} is a checkbox; pass true or false", field.name)),
        },
        "url" => match value {
            Value::String(url) => Ok(json!({ "Url": url, "Label": url })),
            Value::Object(o) if o.get("Url").is_some_and(Value::is_string) => {
                let url = &o["Url"];
                let label = o.get("Label").filter(|l| l.is_string()).unwrap_or(url);
                Ok(json!({ "Url": url, "Label": label }))
            }
            _ => Err(format!(
                "{} is a URL field; pass a URL or {{\"Url\",\"Label\"}}",
                field.name
            )),
        },
        "entity" | "multipleentities" => match value {
            Value::Object(o) if o.get("Id").is_some_and(Value::is_number) => Ok(value.clone()),
            _ => Err(format!(
                "{} references an entity; pass {{\"Id\": n, \"Kind\": \"Release\"}}",
                field.name
            )),
        },
        _ => Ok(value.clone()),
    }
}

/// Resolves field names for the card type/process and validates every value before
/// anything is sent. Unknown names and bad values are collected into one refusal
/// (reason `NotFound`); a failed load of the fields stops at once.
pub async fn prepare_custom_fields(
    ctx: &Ctx,
    process_id: i64,
    entity_type: &str,
    values: &Map<String, Value>,
) -> Resolved<Vec<CustomFieldValue>> {
    let mut out = Vec::with_capacity(values.len());
    let mut problems: Vec<String> = Vec::new();
    for (name, value) in values {
        let field = match ctx
            .directory
            .custom_field(process_id, entity_type, name)
            .await
        {
            Ok(field) => field,
            Err(r) if r.reason == Reason::Error => return Err(r),
            Err(r) => {
                problems.push(r.message);
                continue;
            }
        };
        match check_custom_value(&field, value) {
            Ok(value) => out.push(CustomFieldValue {
                name: field.name,
                value,
            }),
            Err(message) => problems.push(message),
        }
    }
    if !problems.is_empty() {
        return Err(Box::new(Unresolved {
            message: format!("Custom fields rejected:\n- {}", problems.join("\n- ")),
            reason: Reason::NotFound,
            candidates: None,
            error: None,
        }));
    }
    Ok(out)
}

fn is_date_start(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() >= 10
        && b[..4].iter().all(u8::is_ascii_digit)
        && b[4] == b'-'
        && b[5..7].iter().all(u8::is_ascii_digit)
        && b[7] == b'-'
        && b[8..10].iter().all(u8::is_ascii_digit)
}

/// A comma list as a sorted set (JavaScript's default sort: UTF-16 order).
fn list_set(s: &str) -> Vec<&str> {
    let mut parts: Vec<&str> = s.split(',').map(js_trim).collect();
    parts.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
    parts
}

/// Whether a read-back value is what was written: entities by id, URLs by URL,
/// dates by day (or instant), comma lists as sets, the rest as JSON.
fn same_custom_value(want: &Value, got: &Value) -> bool {
    if want.is_null() {
        return got.is_null() || got.as_str() == Some("");
    }
    if let Some(url) = want.get("Url").filter(|u| u.is_string()) {
        let got_url = if got.is_object() {
            got.get("Url")
        } else {
            Some(got)
        };
        return got_url == Some(url);
    }
    if let Some(id) = want.get("Id").and_then(Value::as_f64) {
        return got.is_object() && got.get("Id").and_then(Value::as_f64) == Some(id);
    }
    if let (Some(w), Some(g)) = (want.as_str(), got.as_str()) {
        if is_date_start(w) {
            return same_date_value(w, g);
        }
        if w.contains(',') {
            return list_set(w) == list_set(g);
        }
    }
    js_json(got) == js_json(want)
}

/// Compares requested custom field values with a read-back `CustomFields` array;
/// one line per value that did not persist.
pub fn unpersisted_custom_fields(requested: &[CustomFieldValue], read_back: &Value) -> Vec<String> {
    let list: &[Value] = read_back.as_array().map_or(&[], Vec::as_slice);
    requested
        .iter()
        .filter_map(|req| {
            let Some(field) = list
                .iter()
                .filter(|f| f.is_object())
                .find(|f| f.get("Name").and_then(Value::as_str) == Some(req.name.as_str()))
            else {
                return Some(format!(
                    "{}: not present on the card after writing",
                    req.name
                ));
            };
            let got = field.get("Value").unwrap_or(&Value::Null);
            (!same_custom_value(&req.value, got)).then(|| {
                format!(
                    "{}: requested {}, got {}",
                    req.name,
                    js_json(&req.value),
                    js_json(got)
                )
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value(name: &str, value: Value) -> CustomFieldValue {
        CustomFieldValue {
            name: name.into(),
            value,
        }
    }

    #[test]
    fn compares_entities_by_id_urls_by_url_dates_by_day_lists_as_sets() {
        let back = json!([
            { "Name": "Rel", "Value": { "Id": 2, "Kind": "Release" } },
            { "Name": "Link", "Value": { "Url": "https://x", "Label": "x" } },
            { "Name": "Due", "Value": "2026-09-23T00:00:00.000+02:00" },
            { "Name": "Multi", "Value": "b, a" },
        ]);
        let requested = [
            value("Rel", json!({ "Id": 1, "Kind": "Release" })),
            value("Link", json!({ "Url": "https://x", "Label": "x" })),
            value("Due", json!("2026-09-23")),
            value("Multi", json!("a,b")),
            value("Gone", json!("x")),
        ];
        assert_eq!(
            unpersisted_custom_fields(&requested, &back),
            [
                r#"Rel: requested {"Id":1,"Kind":"Release"}, got {"Id":2,"Kind":"Release"}"#,
                "Gone: not present on the card after writing",
            ]
        );
    }
}
