//! Tool parameters: one definition gives both the JSON schema the model reads and the
//! validation of the arguments it sends, with zod's messages (the TypeScript build's
//! validator). The schema is as lean as that build's: no `$schema`, no defaults, no
//! `propertyNames`, since every byte is model tokens. `tests/golden/tools.json` pins it.

use std::borrow::Cow;

use regex::Regex;
use serde_json::{Map, Value, json};

/// A check zod ran with `.refine()`: the value passes, or the message is the issue.
pub type Refine = (fn(&str) -> bool, &'static str);

#[derive(Clone)]
pub enum Kind {
    Str {
        trim: bool,
        min: Option<usize>,
        /// With a message, or zod's default "must match pattern".
        pattern: Option<(Regex, Option<&'static str>)>,
        refine: Option<Refine>,
    },
    /// A name or an id: numbers become strings, blank is refused. Published as a plain string.
    NameOrId,
    OneOf(&'static [&'static str]),
    /// A positive integer id; numeric strings are converted. Published as `{"type":"number"}`.
    Id,
    /// An id, or `null` to clear the reference.
    IdOrNull,
    Num {
        int: bool,
        /// Exclusive lower bound 0 (`positive()`).
        positive: bool,
        min: Option<f64>,
        max: Option<f64>,
    },
    Bool,
    List {
        item: Box<Kind>,
        min: Option<usize>,
        max: Option<usize>,
    },
    Object(Vec<Field>),
    /// Any JSON object; with a message, it must have at least one key.
    Record(Option<&'static str>),
}

#[derive(Clone)]
pub struct Field {
    pub name: &'static str,
    pub kind: Kind,
    pub optional: bool,
    pub description: Option<Cow<'static, str>>,
}

pub fn field(name: &'static str, kind: Kind) -> Field {
    Field {
        name,
        kind,
        optional: false,
        description: None,
    }
}

pub fn string() -> Kind {
    Kind::Str {
        trim: false,
        min: None,
        pattern: None,
        refine: None,
    }
}

/// A string of at least one character.
pub fn nonempty() -> Kind {
    Kind::Str {
        trim: false,
        min: Some(1),
        pattern: None,
        refine: None,
    }
}

pub fn pattern(re: &str, message: Option<&'static str>) -> Kind {
    Kind::Str {
        trim: false,
        min: None,
        pattern: Some((Regex::new(re).expect("valid pattern"), message)),
        refine: None,
    }
}

pub fn list(item: Kind, min: Option<usize>, max: Option<usize>) -> Kind {
    Kind::List {
        item: Box::new(item),
        min,
        max,
    }
}

/// `z.number().int().min(1).max(max)`, described as "Default N".
pub fn limit(name: &'static str, default: u32, max: u32) -> Field {
    field(
        name,
        Kind::Num {
            int: true,
            positive: false,
            min: Some(1.0),
            max: Some(f64::from(max)),
        },
    )
    .optional()
    .describe(format!("Default {default}"))
}

impl Field {
    pub fn optional(mut self) -> Self {
        self.optional = true;
        self
    }

    pub fn describe(mut self, description: impl Into<Cow<'static, str>>) -> Self {
        self.description = Some(description.into());
        self
    }
}

fn number(n: f64) -> Value {
    if n.fract() == 0.0 && n.abs() < 9_007_199_254_740_992.0 {
        json!(n as i64)
    } else {
        json!(n)
    }
}

fn kind_schema(kind: &Kind) -> Map<String, Value> {
    let mut s = Map::new();
    match kind {
        Kind::Str { min, pattern, .. } => {
            s.insert("type".into(), json!("string"));
            if let Some(n) = min {
                s.insert("minLength".into(), json!(n));
            }
            if let Some((re, _)) = pattern {
                s.insert("pattern".into(), json!(re.as_str()));
            }
        }
        Kind::NameOrId => {
            s.insert("type".into(), json!("string"));
        }
        Kind::OneOf(values) => {
            s.insert("type".into(), json!("string"));
            s.insert("enum".into(), json!(values));
        }
        Kind::Id => {
            s.insert("type".into(), json!("number"));
        }
        Kind::IdOrNull => {
            s.insert("type".into(), json!(["number", "null"]));
        }
        Kind::Num {
            int,
            positive,
            min,
            max,
        } => {
            s.insert(
                "type".into(),
                json!(if *int { "integer" } else { "number" }),
            );
            if *positive {
                s.insert("exclusiveMinimum".into(), json!(0));
            }
            if let Some(n) = min {
                s.insert("minimum".into(), number(*n));
            }
            if let Some(n) = max {
                s.insert("maximum".into(), number(*n));
            }
        }
        Kind::Bool => {
            s.insert("type".into(), json!("boolean"));
        }
        Kind::List { item, min, max } => {
            if let Some(n) = min {
                s.insert("minItems".into(), json!(n));
            }
            if let Some(n) = max {
                s.insert("maxItems".into(), json!(n));
            }
            s.insert("type".into(), json!("array"));
            s.insert("items".into(), Value::Object(kind_schema(item)));
        }
        Kind::Object(fields) => {
            s = object_schema(fields);
        }
        Kind::Record(_) => {
            s.insert("type".into(), json!("object"));
        }
    }
    s
}

fn object_schema(fields: &[Field]) -> Map<String, Value> {
    let mut properties = Map::new();
    for f in fields {
        let mut s = kind_schema(&f.kind);
        if let Some(d) = &f.description {
            s.insert("description".into(), json!(d));
        }
        properties.insert(f.name.into(), Value::Object(s));
    }
    let required: Vec<&str> = fields
        .iter()
        .filter(|f| !f.optional)
        .map(|f| f.name)
        .collect();
    let mut schema = Map::new();
    schema.insert("type".into(), json!("object"));
    schema.insert("properties".into(), Value::Object(properties));
    if !required.is_empty() {
        schema.insert("required".into(), json!(required));
    }
    schema
}

/// The `inputSchema` of a tool.
pub fn input_schema(fields: &[Field]) -> Value {
    Value::Object(object_schema(fields))
}

fn type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// zod's name for what a kind expects, in "expected X, received Y".
fn expected(kind: &Kind) -> &'static str {
    match kind {
        Kind::Str { .. } | Kind::NameOrId | Kind::OneOf(_) => "string",
        Kind::Id | Kind::Num { .. } => "number",
        Kind::IdOrNull => "number",
        Kind::Bool => "boolean",
        Kind::List { .. } => "array",
        Kind::Object(_) => "object",
        Kind::Record(_) => "record",
    }
}

fn is_safe_integer(n: f64) -> bool {
    n.fract() == 0.0 && n.abs() <= 9_007_199_254_740_991.0
}

const ID_PATTERN: &str = r"^\d+$";

/// The value as the handler gets it (trimmed, converted), or the problems with it.
fn check(kind: &Kind, value: &Value, path: &str, issues: &mut Vec<String>) -> Value {
    let at = |message: String| {
        if path.is_empty() {
            message
        } else {
            format!("{message} at {path}")
        }
    };
    let wrong_type = |issues: &mut Vec<String>| {
        issues.push(at(format!(
            "Invalid input: expected {}, received {}",
            expected(kind),
            type_name(value)
        )));
        Value::Null
    };
    match kind {
        Kind::Str {
            trim,
            min,
            pattern,
            refine,
        } => {
            let Value::String(s) = value else {
                return wrong_type(issues);
            };
            let s = if *trim { s.trim() } else { s.as_str() };
            if let Some(n) = min.filter(|n| s.encode_utf16().count() < *n) {
                issues.push(at(format!(
                    "Too small: expected string to have >={n} characters"
                )));
            }
            if let Some((re, message)) = pattern
                && !re.is_match(s)
            {
                issues.push(at(match message {
                    Some(m) => (*m).to_string(),
                    None => format!("Invalid string: must match pattern /{}/", re.as_str()),
                }));
            }
            if let Some((ok, message)) = refine
                && !ok(s)
            {
                issues.push(at((*message).to_string()));
            }
            json!(s)
        }
        Kind::NameOrId => {
            let s = match value {
                Value::Number(n) => n.to_string(),
                Value::String(s) => s.clone(),
                _ => return wrong_type(issues),
            };
            if s.trim().is_empty() {
                issues.push(at("must not be empty".into()));
            }
            json!(s)
        }
        Kind::OneOf(values) => {
            if !value.as_str().is_some_and(|s| values.contains(&s)) {
                let options: Vec<String> = values.iter().map(|v| format!("\"{v}\"")).collect();
                issues.push(at(format!(
                    "Invalid option: expected one of {}",
                    options.join("|")
                )));
            }
            value.clone()
        }
        Kind::Id => {
            let n = match value {
                Value::String(s) if Regex::new(ID_PATTERN).unwrap().is_match(s) => {
                    s.parse::<f64>().unwrap_or(f64::NAN)
                }
                Value::Number(n) => n.as_f64().unwrap_or(f64::NAN),
                _ => {
                    issues.push(at(r"must be a numeric id (^\d+$)".into()));
                    return Value::Null;
                }
            };
            if !(is_safe_integer(n) && n > 0.0) {
                issues.push(at("must be a positive integer id".into()));
                return Value::Null;
            }
            json!(n as i64)
        }
        Kind::IdOrNull => {
            if value.is_null() {
                return Value::Null;
            }
            let mut inner = Vec::new();
            let v = check(&Kind::Id, value, "", &mut inner);
            if !inner.is_empty() {
                issues.push(at("Invalid input".into()));
            }
            v
        }
        Kind::Num {
            int,
            positive,
            min,
            max,
        } => {
            let Some(n) = value.as_f64() else {
                return wrong_type(issues);
            };
            if *int && !is_safe_integer(n) {
                issues.push(at("Invalid input: expected int, received number".into()));
                return Value::Null;
            }
            if *positive && n <= 0.0 {
                issues.push(at("Too small: expected number to be >0".into()));
            }
            if let Some(m) = min.filter(|m| n < *m) {
                issues.push(at(format!(
                    "Too small: expected number to be >={}",
                    number(m)
                )));
            }
            if let Some(m) = max.filter(|m| n > *m) {
                issues.push(at(format!(
                    "Too big: expected number to be <={}",
                    number(m)
                )));
            }
            value.clone()
        }
        Kind::Bool => {
            if !value.is_boolean() {
                return wrong_type(issues);
            }
            value.clone()
        }
        Kind::List { item, min, max } => {
            let Value::Array(items) = value else {
                return wrong_type(issues);
            };
            if let Some(n) = min.filter(|n| items.len() < *n) {
                issues.push(at(format!("Too small: expected array to have >={n} items")));
            }
            if let Some(n) = max.filter(|n| items.len() > *n) {
                issues.push(at(format!("Too big: expected array to have <={n} items")));
            }
            Value::Array(
                items
                    .iter()
                    .enumerate()
                    .map(|(i, v)| check(item, v, &format!("{path}[{i}]"), issues))
                    .collect(),
            )
        }
        Kind::Object(fields) => {
            let Value::Object(given) = value else {
                return wrong_type(issues);
            };
            Value::Object(check_fields(fields, given, path, issues))
        }
        Kind::Record(nonempty) => {
            let Value::Object(map) = value else {
                return wrong_type(issues);
            };
            if let Some(message) = nonempty
                && map.is_empty()
            {
                issues.push(at((*message).to_string()));
            }
            value.clone()
        }
    }
}

fn check_fields(
    fields: &[Field],
    given: &Map<String, Value>,
    path: &str,
    issues: &mut Vec<String>,
) -> Map<String, Value> {
    let mut out = Map::new();
    for f in fields {
        let at = if path.is_empty() {
            f.name.to_string()
        } else {
            format!("{path}.{}", f.name)
        };
        match given.get(f.name) {
            None if f.optional => {}
            None => issues.push(format!(
                "Invalid input: expected {}, received undefined at {at}",
                expected(&f.kind)
            )),
            Some(v) => {
                let checked = check(&f.kind, v, &at, issues);
                out.insert(f.name.into(), checked);
            }
        }
    }
    out
}

/// Checks `args` against `fields`; unknown keys are dropped. `Err` lists every problem.
pub fn validate(fields: &[Field], args: &Value) -> Result<Args, String> {
    let empty = Map::new();
    let given = match args {
        Value::Object(map) => map,
        Value::Null => &empty,
        other => {
            return Err(format!(
                "Invalid input: expected object, received {}",
                type_name(other)
            ));
        }
    };
    let mut issues = Vec::new();
    let out = check_fields(fields, given, "", &mut issues);
    if issues.is_empty() {
        Ok(Args(out))
    } else {
        Err(issues.join("; "))
    }
}

/// Validated arguments. Getters return `None` for a parameter that was left out.
#[derive(Debug, Clone, Default)]
pub struct Args(pub Map<String, Value>);

impl Args {
    pub fn get(&self, name: &str) -> Option<&Value> {
        self.0.get(name)
    }

    pub fn has(&self, name: &str) -> bool {
        self.0.contains_key(name)
    }

    pub fn opt_str(&self, name: &str) -> Option<String> {
        self.0.get(name).and_then(Value::as_str).map(str::to_string)
    }

    /// A required string; validation guarantees it is there.
    pub fn str(&self, name: &str) -> String {
        self.opt_str(name).unwrap_or_default()
    }

    pub fn opt_id(&self, name: &str) -> Option<i64> {
        self.0.get(name).and_then(Value::as_i64)
    }

    /// A required id; validation guarantees it is there.
    pub fn id(&self, name: &str) -> i64 {
        self.opt_id(name).unwrap_or_default()
    }

    pub fn opt_num(&self, name: &str) -> Option<f64> {
        self.0.get(name).and_then(Value::as_f64)
    }

    pub fn opt_bool(&self, name: &str) -> Option<bool> {
        self.0.get(name).and_then(Value::as_bool)
    }

    pub fn bool(&self, name: &str) -> bool {
        self.opt_bool(name).unwrap_or(false)
    }

    pub fn opt_list(&self, name: &str) -> Option<&Vec<Value>> {
        self.0.get(name).and_then(Value::as_array)
    }

    pub fn opt_strs(&self, name: &str) -> Option<Vec<String>> {
        self.opt_list(name).map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
    }

    pub fn opt_ids(&self, name: &str) -> Option<Vec<i64>> {
        self.opt_list(name)
            .map(|a| a.iter().filter_map(Value::as_i64).collect())
    }

    pub fn opt_map(&self, name: &str) -> Option<&Map<String, Value>> {
        self.0.get(name).and_then(Value::as_object)
    }
}
