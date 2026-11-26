//! Creating cards: which types are cards, and how a new card points at its parent.

use serde_json::{Map, Value, json};

use crate::catalog::CatalogResource;
use crate::resolve::card::CardInfo;

/// The part of a create body that links a new card to its parent.
#[derive(Debug, Clone, PartialEq)]
pub struct ParentField {
    /// The member set, e.g. `UserStory`, `TestPlans`, `LinkedGeneral`.
    pub field: String,
    /// Merged into the create body, e.g. `{"UserStory": {"Id": 7}}`.
    pub body: Map<String, Value>,
}

/// Card types a new card can be created under.
const CARD_TYPES: [&str; 8] = [
    "PortfolioEpic",
    "Epic",
    "Feature",
    "UserStory",
    "Task",
    "Bug",
    "Request",
    "TestPlan",
];

fn field(name: &str, value: Value) -> ParentField {
    let mut body = Map::new();
    body.insert(name.to_string(), value);
    ParentField {
        field: name.to_string(),
        body,
    }
}

/// How a new card points at its parent. Most types use the reference named
/// after the parent's type (Task.UserStory, UserStory.Feature, Feature.Epic,
/// Epic.PortfolioEpic, Bug.UserStory/Feature); test cases join a test plan's
/// TestPlans collection; a test plan links the card it covers via LinkedGeneral.
/// `Err` is the refusal, listing the valid parent types.
pub fn parent_link(child: &CatalogResource, parent: &CardInfo) -> Result<ParentField, String> {
    if child.name == "TestCase" && parent.entity_type == "TestPlan" {
        return Ok(field(
            "TestPlans",
            json!({ "Items": [{ "Id": parent.id }] }),
        ));
    }
    if child.name == "TestPlan" {
        return Ok(field("LinkedGeneral", json!({ "Id": parent.id })));
    }
    if let Some(direct) = child.references.iter().find(|r| {
        r.can_set
            && (r.name == parent.entity_type || r.type_name == parent.entity_type)
            && !r.name.starts_with("Linked")
    }) {
        return Ok(field(&direct.name, json!({ "Id": parent.id })));
    }
    let mut valid: Vec<&str> = Vec::new();
    let candidates = child
        .references
        .iter()
        .filter(|r| {
            r.can_set && CARD_TYPES.contains(&r.type_name.as_str()) && !r.name.starts_with("Linked")
        })
        .map(|r| r.type_name.as_str())
        .chain((child.name == "TestCase").then_some("TestPlan"));
    for t in candidates {
        if !valid.contains(&t) {
            valid.push(t);
        }
    }
    let valid = if valid.is_empty() {
        "none".to_string()
    } else {
        valid.join(", ")
    };
    Err(format!(
        "A {} cannot be created under a {}. Valid parent types: {valid}.",
        child.name, parent.entity_type
    ))
}

/// Card types pounce creates through write_create_card: creatable, with a settable Name and Project.
pub fn is_card_resource(resource: &CatalogResource) -> bool {
    resource.can_create
        && resource
            .values
            .iter()
            .any(|f| f.name == "Name" && f.can_set)
        && resource
            .references
            .iter()
            .any(|r| r.name == "Project" && r.can_set)
}
