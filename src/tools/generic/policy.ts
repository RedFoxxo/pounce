import type { Catalog, Lookup } from '../../catalog/catalog.js'
import type { CatalogResource } from '../../catalog/types.js'

/**
 * Configuration and administration resources. Only `admin_*` tools write them;
 * the generic write/delete tools refuse them. Review against real usage.
 */
export const ADMIN_RESOURCES: ReadonlySet<string> = new Set([
  'CustomRule',
  'CustomField',
  'EntityPermission',
  'EntityState',
  'GlobalSettings',
  'Priority',
  'Process',
  'Program',
  'Project',
  'ProjectMember',
  'RequestType',
  'Role',
  'RoleEntityType',
  'RoleEntityTypeProcessSetting',
  'Severity',
  'Team',
  'TeamMember',
  'TeamProject',
  'Term',
  'User',
  'Workflow',
])

/** Resources whose metadata claims write operations that do not exist in practice. */
export const READ_ONLY_IN_PRACTICE: Readonly<Record<string, string>> = {
  Context: 'Context is computed per request; its metadata lists create/update/delete but it is read-only in practice. Use read_context.',
}

export type WriteOperation = 'create' | 'update' | 'delete' | 'add' | 'remove'
export type WriteTier = 'write' | 'delete' | 'admin'

export function isAdminResource(resource: CatalogResource): boolean {
  return ADMIN_RESOURCES.has(resource.name)
}

const TOOL_FOR: Record<WriteOperation, { write: string; admin: string }> = {
  create: { write: 'write_create', admin: 'admin_create' },
  update: { write: 'write_update', admin: 'admin_update' },
  delete: { write: 'delete_entity', admin: 'admin_delete' },
  add: { write: 'write_collection_add', admin: 'admin_collection_add' },
  remove: { write: 'delete_collection_remove', admin: 'admin_collection_remove' },
}

/**
 * Resolves the resource and checks that the operation exists on it and that
 * the calling tier is the right one (admin resources only through admin_*).
 */
export function resolveWritable(
  catalog: Catalog,
  name: string,
  op: WriteOperation,
  tier: WriteTier,
): Lookup<CatalogResource> {
  const lookup = catalog.resource(name)
  if (!lookup.ok) return lookup
  const resource = lookup.value

  const readOnly = READ_ONLY_IN_PRACTICE[resource.name]
  if (readOnly) return { ok: false, message: readOnly }

  const admin = isAdminResource(resource)
  if (admin && tier !== 'admin') {
    return {
      ok: false,
      message: `${resource.name} is a configuration/administration resource; use ${TOOL_FOR[op].admin} instead.`,
    }
  }
  if (!admin && tier === 'admin') {
    return {
      ok: false,
      message: `${resource.name} is not an administration resource; use ${TOOL_FOR[op].write} instead.`,
    }
  }

  // Collection add/remove is governed by the collection's own CanAdd/CanRemove.
  const allowed =
    op === 'create' ? resource.canCreate : op === 'update' ? resource.canUpdate : op === 'delete' ? resource.canDelete : true
  if (!allowed) {
    const ops = [resource.canCreate && 'create', resource.canUpdate && 'update', resource.canDelete && 'delete'].filter(Boolean)
    return {
      ok: false,
      message: `${resource.name} does not support ${op} (supported: ${ops.join(', ') || 'read only'}).`,
    }
  }
  return { ok: true, value: resource }
}
