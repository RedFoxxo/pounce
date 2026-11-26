//! Which tier may write which resource: administration resources only through
//! `admin_*`, everything else only through the generic write/delete tools.

use std::sync::Arc;

use crate::catalog::{Catalog, CatalogResource, Lookup};

/// Configuration and administration resources. Only `admin_*` tools write them;
/// the generic write/delete tools refuse them. Review against real usage.
pub const ADMIN_RESOURCES: [&str; 21] = [
    "CustomRule",
    "CustomField",
    "EntityPermission",
    "EntityState",
    "GlobalSettings",
    "Priority",
    "Process",
    "Program",
    "Project",
    "ProjectMember",
    "RequestType",
    "Role",
    "RoleEntityType",
    "RoleEntityTypeProcessSetting",
    "Severity",
    "Team",
    "TeamMember",
    "TeamProject",
    "Term",
    "User",
    "Workflow",
];

/// Resources whose metadata claims write operations that do not exist in practice.
pub fn read_only_in_practice(name: &str) -> Option<&'static str> {
    match name {
        "Context" => Some(
            "Context is computed per request; its metadata lists create/update/delete but it is read-only in practice. Use read_context.",
        ),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteOperation {
    Create,
    Update,
    Delete,
    Add,
    Remove,
}

impl WriteOperation {
    pub fn as_str(self) -> &'static str {
        match self {
            WriteOperation::Create => "create",
            WriteOperation::Update => "update",
            WriteOperation::Delete => "delete",
            WriteOperation::Add => "add",
            WriteOperation::Remove => "remove",
        }
    }

    /// The generic tool and the admin tool for the operation.
    fn tools(self) -> (&'static str, &'static str) {
        match self {
            WriteOperation::Create => ("write_create", "admin_create"),
            WriteOperation::Update => ("write_update", "admin_update"),
            WriteOperation::Delete => ("delete_entity", "admin_delete"),
            WriteOperation::Add => ("write_collection_add", "admin_collection_add"),
            WriteOperation::Remove => ("delete_collection_remove", "admin_collection_remove"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteTier {
    Write,
    Delete,
    Admin,
}

pub fn is_admin_resource(resource: &CatalogResource) -> bool {
    ADMIN_RESOURCES.contains(&resource.name.as_str())
}

/// Resolves the resource and checks that the operation exists on it and that
/// the calling tier is the right one (admin resources only through admin_*).
pub fn resolve_writable<'c>(
    catalog: &'c Catalog,
    name: &str,
    op: WriteOperation,
    tier: WriteTier,
) -> Lookup<&'c Arc<CatalogResource>> {
    let resource = catalog.resource(name)?;

    if let Some(read_only) = read_only_in_practice(&resource.name) {
        return Err(read_only.to_string());
    }

    let admin = is_admin_resource(resource);
    let (write_tool, admin_tool) = op.tools();
    if admin && tier != WriteTier::Admin {
        return Err(format!(
            "{} is a configuration/administration resource; use {admin_tool} instead.",
            resource.name
        ));
    }
    if !admin && tier == WriteTier::Admin {
        return Err(format!(
            "{} is not an administration resource; use {write_tool} instead.",
            resource.name
        ));
    }

    // Collection add/remove is governed by the collection's own CanAdd/CanRemove.
    let allowed = match op {
        WriteOperation::Create => resource.can_create,
        WriteOperation::Update => resource.can_update,
        WriteOperation::Delete => resource.can_delete,
        WriteOperation::Add | WriteOperation::Remove => true,
    };
    if !allowed {
        let ops: Vec<&str> = [
            (resource.can_create, "create"),
            (resource.can_update, "update"),
            (resource.can_delete, "delete"),
        ]
        .into_iter()
        .filter_map(|(can, op)| can.then_some(op))
        .collect();
        let supported = ops.join(", ");
        return Err(format!(
            "{} does not support {} (supported: {}).",
            resource.name,
            op.as_str(),
            if supported.is_empty() {
                "read only"
            } else {
                &supported
            }
        ));
    }
    Ok(resource)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::snapshot;

    #[test]
    fn refuses_administration_resources_and_points_at_admin_tools() {
        let catalog = snapshot();
        let e = resolve_writable(
            &catalog,
            "Project",
            WriteOperation::Update,
            WriteTier::Write,
        )
        .unwrap_err();
        assert!(e.ends_with("use admin_update instead."), "{e}");
        let e = resolve_writable(
            &catalog,
            "UserStory",
            WriteOperation::Update,
            WriteTier::Admin,
        )
        .unwrap_err();
        assert!(e.ends_with("use write_update instead."), "{e}");
        assert_eq!(
            resolve_writable(
                &catalog,
                "Project",
                WriteOperation::Update,
                WriteTier::Admin
            )
            .unwrap()
            .name,
            "Project"
        );
    }

    #[test]
    fn refuses_operations_the_resource_does_not_support() {
        let e = resolve_writable(
            &snapshot(),
            "EntityType",
            WriteOperation::Create,
            WriteTier::Write,
        )
        .unwrap_err();
        assert!(
            e.starts_with("EntityType does not support create (supported: "),
            "{e}"
        );
    }

    #[test]
    fn refuses_context_writes() {
        let e = resolve_writable(
            &snapshot(),
            "Context",
            WriteOperation::Create,
            WriteTier::Write,
        )
        .unwrap_err();
        assert!(e.contains("read-only in practice"), "{e}");
    }

    #[test]
    fn collection_operations_skip_the_resource_flags() {
        let catalog = snapshot();
        assert!(resolve_writable(&catalog, "Build", WriteOperation::Add, WriteTier::Write).is_ok());
        assert!(
            resolve_writable(
                &catalog,
                "TestPlan",
                WriteOperation::Remove,
                WriteTier::Delete
            )
            .is_ok()
        );
    }

    #[test]
    fn passes_unknown_resources_through_with_suggestions() {
        let e = resolve_writable(
            &snapshot(),
            "UserStorie",
            WriteOperation::Create,
            WriteTier::Write,
        )
        .unwrap_err();
        assert!(
            e.starts_with("Unknown resource \"UserStorie\". Did you mean: UserStory"),
            "{e}"
        );
    }
}
