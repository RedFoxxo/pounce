//! Parameters shared by many tools (`src/tools/schema.ts` in the TypeScript build).

use crate::schema::{Field, Kind, field, list};

/// A positive integer id; numbers and numeric strings are accepted, published as `{"type":"number"}`.
pub fn id(name: &'static str) -> Field {
    field(name, Kind::Id)
}

/// One to `max` ids.
pub fn ids(name: &'static str, max: usize) -> Field {
    field(name, list(Kind::Id, Some(1), Some(max)))
}

/// A name, or an id; published as a plain string, numbers are converted.
pub fn name_or_id(name: &'static str) -> Field {
    field(name, Kind::NameOrId)
}

pub fn resource() -> Field {
    field("resource", crate::schema::nonempty())
        .describe("Resource name (UserStory) or plural path (UserStories); see read_meta")
}

/// `Id,Name,Project[Name]` or `[Id,Name,Project[Name]]`; optional.
pub fn include(name: &'static str) -> Field {
    field(name, crate::schema::nonempty()).optional()
}

pub fn fields() -> Field {
    field("fields", Kind::Record(None))
        .describe(r#"Entity fields as Targetprocess JSON, e.g. {"Name":"x","Project":{"Id":2}}"#)
}

/// Wraps a field list in the brackets Targetprocess expects.
pub fn bracket(list: Option<&str>) -> Option<String> {
    let trimmed = list?.trim();
    if trimmed.is_empty() {
        None
    } else if trimmed.starts_with('[') {
        Some(trimmed.to_string())
    } else {
        Some(format!("[{trimmed}]"))
    }
}
