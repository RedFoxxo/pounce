//! What the workflow tools share: failed resolutions as answers, and where a
//! card type's states and custom fields come from.

use std::sync::Arc;

use crate::catalog::{CatalogResource, Request};
use crate::resolve::card::{ProjectRef, card_info};
use crate::resolve::matching::Unresolved;
use crate::tools::{Ctx, ToolResult, failure, invalid};

/// Turns a failed resolution into a tool error: HTTP failures carry status/body, the rest is "Not sent".
pub fn unresolved(r: &Unresolved) -> ToolResult {
    match &r.error {
        Some(e) => failure(&r.message, Some(e), None),
        None => invalid(&r.message),
    }
}

/// The card type and process whose workflow (states, custom fields) a call is about.
#[derive(Debug, Clone)]
pub struct TypeScope {
    pub entity_type: String,
    pub resource: Arc<CatalogResource>,
    pub process_id: i64,
    pub project: Option<ProjectRef>,
    /// Set when the scope came from a card.
    pub card_id: Option<i64>,
}

/// Where states/custom fields come from: a card (its own type and project), or
/// a project (name, abbreviation or id) plus an entity type.
pub async fn type_scope(
    ctx: &Ctx,
    id: Option<i64>,
    project: Option<&str>,
    entity_type: Option<&str>,
) -> Result<TypeScope, ToolResult> {
    if let Some(id) = id {
        let info = card_info(ctx, id).await.map_err(|r| unresolved(&r))?;
        let Some(process_id) = info.process_id else {
            return Err(invalid(format!(
                "{} {id} has no project, so it has no process workflow.",
                info.entity_type
            )));
        };
        return Ok(TypeScope {
            entity_type: info.entity_type,
            resource: info.resource,
            process_id,
            project: info.project,
            card_id: Some(id),
        });
    }
    let (Some(project), Some(entity_type)) = (project, entity_type.filter(|t| !t.is_empty()))
    else {
        return Err(invalid(
            "pass a card id, or both project and type (e.g. project \"SBP\", type \"UserStory\")",
        ));
    };
    let catalog = ctx
        .catalog
        .get(&Request::Names(vec![entity_type.to_string()]))
        .await;
    let resource = catalog.resource(entity_type).map_err(invalid)?.clone();
    let project = ctx
        .directory
        .project(project)
        .await
        .map_err(|r| unresolved(&r))?;
    let Some(process) = project.process else {
        return Err(invalid(format!("Project {} has no process.", project.name)));
    };
    Ok(TypeScope {
        entity_type: resource.name.clone(),
        resource,
        process_id: process.id,
        project: Some(ProjectRef {
            id: project.id,
            name: project.name,
        }),
        card_id: None,
    })
}
