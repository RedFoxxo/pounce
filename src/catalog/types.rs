//! What the catalog knows about one instance, with the JSON shape of `snapshot.json`.

use std::sync::Arc;

use serde_json::{Map, Value, json};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogField {
    pub name: String,
    /// Targetprocess type name, e.g. `String`, `Int32`, `UserStory`.
    pub type_name: String,
    pub can_set: bool,
    pub required: bool,
    pub deprecated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogCollection {
    pub name: String,
    /// Type of the items, e.g. `Task` for `UserStory.Tasks`.
    pub type_name: String,
    pub can_set: bool,
    pub required: bool,
    pub deprecated: bool,
    pub can_add: bool,
    pub can_remove: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogResource {
    /// Singular name, e.g. `UserStory`.
    pub name: String,
    /// Plural path below `/api/v1`, e.g. `UserStories`.
    pub path: String,
    pub description: String,
    /// Listed in `/api/v1/Index/meta` (false for probed history/conversion resources).
    pub listed: bool,
    /// False when the index lists the resource but its `/meta` fails (e.g. SickLeave).
    pub available: bool,
    pub can_create: bool,
    pub can_update: bool,
    pub can_delete: bool,
    /// Base resources, e.g. `Assignable` for `UserStory`.
    pub bases: Vec<String>,
    pub values: Vec<CatalogField>,
    pub references: Vec<CatalogField>,
    pub collections: Vec<CatalogCollection>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogData {
    /// Instance the catalog was read from.
    pub instance: String,
    /// Targetprocess version reported by `Context`, when readable.
    pub version: Option<String>,
    pub captured_at: String,
    /// Shared: every catalog the provider builds reuses the resources loaded before.
    pub resources: Vec<Arc<CatalogResource>>,
}

/// A value, reference or collection of a resource, as `Catalog::member` finds it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Member<'a> {
    Value(&'a CatalogField),
    Reference(&'a CatalogField),
    Collection(&'a CatalogCollection),
}

impl<'a> Member<'a> {
    pub fn name(self) -> &'a str {
        match self {
            Member::Value(f) | Member::Reference(f) => &f.name,
            Member::Collection(c) => &c.name,
        }
    }

    pub fn type_name(self) -> &'a str {
        match self {
            Member::Value(f) | Member::Reference(f) => &f.type_name,
            Member::Collection(c) => &c.type_name,
        }
    }

    pub fn can_set(self) -> bool {
        match self {
            Member::Value(f) | Member::Reference(f) => f.can_set,
            Member::Collection(c) => c.can_set,
        }
    }
}

// The snapshot is ours and pinned by tests, so reading is lenient like the
// TypeScript cast: a missing string is empty, a missing flag false, a missing list empty.

fn text(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn flag(v: &Value, key: &str) -> bool {
    v.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn list<'v>(v: &'v Value, key: &str) -> &'v [Value] {
    v.get(key)
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

impl CatalogField {
    pub fn from_json(v: &Value) -> Self {
        CatalogField {
            name: text(v, "name"),
            type_name: text(v, "type"),
            can_set: flag(v, "canSet"),
            required: flag(v, "required"),
            deprecated: flag(v, "deprecated"),
        }
    }

    pub fn to_json(&self) -> Value {
        json!({
            "name": self.name,
            "type": self.type_name,
            "canSet": self.can_set,
            "required": self.required,
            "deprecated": self.deprecated,
        })
    }
}

impl CatalogCollection {
    pub fn from_json(v: &Value) -> Self {
        CatalogCollection {
            name: text(v, "name"),
            type_name: text(v, "type"),
            can_set: flag(v, "canSet"),
            required: flag(v, "required"),
            deprecated: flag(v, "deprecated"),
            can_add: flag(v, "canAdd"),
            can_remove: flag(v, "canRemove"),
        }
    }

    pub fn to_json(&self) -> Value {
        json!({
            "name": self.name,
            "type": self.type_name,
            "canSet": self.can_set,
            "required": self.required,
            "deprecated": self.deprecated,
            "canAdd": self.can_add,
            "canRemove": self.can_remove,
        })
    }
}

impl CatalogResource {
    pub fn from_json(v: &Value) -> Self {
        CatalogResource {
            name: text(v, "name"),
            path: text(v, "path"),
            description: text(v, "description"),
            listed: flag(v, "listed"),
            available: flag(v, "available"),
            can_create: flag(v, "canCreate"),
            can_update: flag(v, "canUpdate"),
            can_delete: flag(v, "canDelete"),
            bases: list(v, "bases")
                .iter()
                .filter_map(|b| b.as_str().map(str::to_string))
                .collect(),
            values: list(v, "values")
                .iter()
                .map(CatalogField::from_json)
                .collect(),
            references: list(v, "references")
                .iter()
                .map(CatalogField::from_json)
                .collect(),
            collections: list(v, "collections")
                .iter()
                .map(CatalogCollection::from_json)
                .collect(),
        }
    }

    pub fn to_json(&self) -> Value {
        json!({
            "name": self.name,
            "path": self.path,
            "description": self.description,
            "listed": self.listed,
            "available": self.available,
            "canCreate": self.can_create,
            "canUpdate": self.can_update,
            "canDelete": self.can_delete,
            "bases": self.bases,
            "values": self.values.iter().map(CatalogField::to_json).collect::<Vec<_>>(),
            "references": self.references.iter().map(CatalogField::to_json).collect::<Vec<_>>(),
            "collections": self.collections.iter().map(CatalogCollection::to_json).collect::<Vec<_>>(),
        })
    }
}

impl CatalogData {
    pub fn from_json(v: &Value) -> Self {
        CatalogData {
            instance: text(v, "instance"),
            version: v.get("version").and_then(Value::as_str).map(str::to_string),
            captured_at: text(v, "capturedAt"),
            resources: list(v, "resources")
                .iter()
                .map(|r| Arc::new(CatalogResource::from_json(r)))
                .collect(),
        }
    }

    /// `instance`, `version` (when known), `capturedAt`, `resources`.
    pub fn to_json(&self) -> Value {
        let mut out = Map::new();
        out.insert("instance".into(), json!(self.instance));
        if let Some(version) = &self.version {
            out.insert("version".into(), json!(version));
        }
        out.insert("capturedAt".into(), json!(self.captured_at));
        out.insert(
            "resources".into(),
            Value::Array(self.resources.iter().map(|r| r.to_json()).collect()),
        );
        Value::Object(out)
    }
}
