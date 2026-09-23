import { z } from 'zod'
import { BULK_LIMIT } from '../../http/v1.js'
import type { ToolContext } from '../context.js'
import { failure, invalid, success } from '../respond.js'
import { fields, id, ids, resource } from '../schema.js'
import { defineTool, type ToolOutput } from '../types.js'
import { resolveWritable, type WriteTier } from './policy.js'
import { prepareBody, prepareItems, unpersisted } from './validate.js'

type EditTier = Exclude<WriteTier, 'delete'>
type RemoveTier = Exclude<WriteTier, 'write'>

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

export async function createEntity(
  ctx: ToolContext,
  tier: EditTier,
  args: { resource: string; fields: Record<string, unknown> },
): Promise<ToolOutput> {
  const catalog = await ctx.catalog()
  const lookup = resolveWritable(catalog, args.resource, 'create', tier)
  if (!lookup.ok) return invalid(lookup.message)
  const r = lookup.value
  const body = prepareBody(catalog, r, args.fields, 'create')
  if (!body.ok) return invalid(body.message)
  if (Object.keys(body.value).length === 0) return invalid(`no fields given for the new ${r.name}`)

  const result = await ctx.v1.create<Record<string, unknown>>(r.path, body.value)
  if (!result.ok) return failure(`Could not create ${r.name}`, result)
  const notPersisted = unpersisted(body.value, result.data)
  return success({ created: r.name, id: result.data?.Id, entity: result.data, ...(notPersisted.length ? { notPersisted } : {}) })
}

export async function updateEntity(
  ctx: ToolContext,
  tier: EditTier,
  args: { resource: string; id: number; fields: Record<string, unknown> },
): Promise<ToolOutput> {
  const catalog = await ctx.catalog()
  const lookup = resolveWritable(catalog, args.resource, 'update', tier)
  if (!lookup.ok) return invalid(lookup.message)
  const r = lookup.value
  const body = prepareBody(catalog, r, args.fields, 'update')
  if (!body.ok) return invalid(body.message)
  if (Object.keys(body.value).length === 0) return invalid(`no fields given to update on ${r.name} ${args.id}`)

  const result = await ctx.v1.update<Record<string, unknown>>(r.path, args.id, body.value)
  if (!result.ok) return failure(`Could not update ${r.name} ${args.id}`, result)
  const notPersisted = unpersisted(body.value, result.data)
  return success({ updated: r.name, id: args.id, entity: result.data, ...(notPersisted.length ? { notPersisted } : {}) })
}

export async function addToCollection(
  ctx: ToolContext,
  tier: EditTier,
  args: { resource: string; id: number; collection: string; items: unknown[] },
): Promise<ToolOutput> {
  const catalog = await ctx.catalog()
  const lookup = resolveWritable(catalog, args.resource, 'add', tier)
  if (!lookup.ok) return invalid(lookup.message)
  const r = lookup.value
  const coll = catalog.collection(r, args.collection)
  if (!coll.ok) return invalid(coll.message)
  const items = prepareItems(catalog, r, coll.value, args.items, coll.value.name)
  if (!items.ok) return invalid(items.message)

  const result = await ctx.v1.update(r.path, args.id, { [coll.value.name]: items.value })
  if (!result.ok) return failure(`Could not add to ${r.name} ${args.id} ${coll.value.name}`, result)
  return success({ added: args.items.length, resource: r.name, id: args.id, collection: coll.value.name, entity: result.data })
}

export async function deleteEntity(
  ctx: ToolContext,
  tier: RemoveTier,
  args: { resource: string; id: number },
): Promise<ToolOutput> {
  const catalog = await ctx.catalog()
  const lookup = resolveWritable(catalog, args.resource, 'delete', tier)
  if (!lookup.ok) return invalid(lookup.message)
  const r = lookup.value

  const before = await ctx.v1.get<Record<string, unknown>>(r.path, args.id)
  if (!before.ok) return failure(`Could not read ${r.name} ${args.id} before deleting it; nothing was deleted`, before)
  const result = await ctx.v1.delete(r.path, args.id)
  if (!result.ok) return failure(`Could not delete ${r.name} ${args.id}`, result)
  const name = isRecord(before.data) ? before.data.Name : undefined
  return success({ deleted: r.name, id: args.id, ...(typeof name === 'string' ? { name } : {}) })
}

export async function removeFromCollection(
  ctx: ToolContext,
  tier: RemoveTier,
  args: { resource: string; id: number; collection: string; childIds: number[] },
): Promise<ToolOutput> {
  const catalog = await ctx.catalog()
  const lookup = resolveWritable(catalog, args.resource, 'remove', tier)
  if (!lookup.ok) return invalid(lookup.message)
  const r = lookup.value
  const coll = catalog.collection(r, args.collection)
  if (!coll.ok) return invalid(coll.message)
  if (!coll.value.canRemove) return invalid(`items cannot be removed from ${r.name}.${coll.value.name}`)

  const result = await ctx.v1.removeFromCollection(r.path, args.id, coll.value.name, args.childIds)
  if (!result.ok) {
    return failure(
      `Could not remove from ${r.name} ${args.id} ${coll.value.name}`,
      result,
      result.removed.length ? { alreadyRemoved: result.removed } : undefined,
    )
  }
  return success({ removed: args.childIds, resource: r.name, id: args.id, collection: coll.value.name })
}

const collectionName = z.string().min(1).describe('Collection name from read_meta, e.g. Assignments, TagObjects, Builds')
const collectionItems = z
  .array(z.record(z.string(), z.unknown()))
  .min(1)
  .describe('Items to add, e.g. [{"GeneralUser":{"Id":1},"Role":{"Id":13}}] or [{"Id":42}] to link an existing entity')

export const writeCreate = defineTool({
  name: 'write_create',
  description:
    'Create an entity of any resource that supports create (layer 1), validated against the catalog before sending. ' +
    'Nested children may be included, e.g. {"Name":"Story","Project":{"Id":2},"Tasks":{"Items":[{"Name":"T1"}]}}. ' +
    'Never include Id. Custom fields go in "CustomFields":[{"Name","Value"}]. Prefer write_create_card for cards: it applies the team rules.',
  input: { resource, fields },
  handler: (args, ctx) => createEntity(ctx, 'write', args),
})

export const writeUpdate = defineTool({
  name: 'write_update',
  description:
    'Update fields of an existing entity (layer 1), validated against the catalog before sending. The response is read back and ' +
    'any requested value that did not persist is reported. Writing a card Effort here bypasses role efforts; use write_set_role_effort instead.',
  input: { resource, id, fields },
  handler: (args, ctx) => updateEntity(ctx, 'write', args),
})

export const writeBulk = defineTool({
  name: 'write_bulk',
  description: `Create or update up to ${BULK_LIMIT} entities of one resource in one call (layer 1). Items with "Id" update, items without create. Every item is validated first; nothing is sent if any item is invalid.`,
  input: {
    resource,
    items: z.array(z.record(z.string(), z.unknown())).min(1).max(BULK_LIMIT),
  },
  handler: async (args, ctx) => {
    const catalog = await ctx.catalog()
    const lookup = catalog.resource(args.resource)
    if (!lookup.ok) return invalid(lookup.message)
    const problems: string[] = []
    const payload: Record<string, unknown>[] = []
    for (const [i, item] of args.items.entries()) {
      const idKey = Object.keys(item).find((k) => k.toLowerCase() === 'id')
      const rawId = idKey ? item[idKey] : undefined
      const mode = idKey ? 'update' : 'create'
      const check = resolveWritable(catalog, args.resource, mode, 'write')
      if (!check.ok) {
        problems.push(`items[${i}]: ${check.message}`)
        continue
      }
      let itemId: number | undefined
      if (idKey) {
        const parsed = id.safeParse(rawId)
        if (!parsed.success) {
          problems.push(`items[${i}].Id: must be a numeric id`)
          continue
        }
        itemId = parsed.data
      }
      const rest = Object.fromEntries(Object.entries(item).filter(([k]) => k !== idKey))
      const body = prepareBody(catalog, check.value, rest, mode, `items[${i}]`)
      if (!body.ok) problems.push(body.message)
      else payload.push(itemId === undefined ? body.value : { Id: itemId, ...body.value })
    }
    if (problems.length > 0) return invalid(`bulk payload rejected:\n- ${problems.join('\n- ')}`)
    const r = lookup.value
    const result = await ctx.v1.bulk(r.path, payload)
    if (!result.ok) return failure(`Bulk write to ${r.path} failed`, result)
    return success({ resource: r.name, sent: payload.length, result: result.data })
  },
})

export const writeCollectionAdd = defineTool({
  name: 'write_collection_add',
  description:
    'Add items to an addable collection of an entity (layer 1). Collection posts APPEND to what is there; nothing is replaced. ' +
    'For assignments, teams and role efforts prefer write_assign, write_team and write_set_role_effort.',
  input: { resource, id, collection: collectionName, items: collectionItems },
  handler: (args, ctx) => addToCollection(ctx, 'write', args),
})

export const deleteEntityTool = defineTool({
  name: 'delete_entity',
  description: 'Delete one entity of any resource that supports delete (layer 1). Reads it first and reports what was deleted.',
  input: { resource, id },
  handler: (args, ctx) => deleteEntity(ctx, 'delete', args),
})

export const deleteBulk = defineTool({
  name: 'delete_bulk',
  description: `Delete up to ${BULK_LIMIT} entities of one resource by id (layer 1). Targetprocess bulk delete works by id only.`,
  input: { resource, ids: ids(BULK_LIMIT) },
  handler: async (args, ctx) => {
    const catalog = await ctx.catalog()
    const lookup = resolveWritable(catalog, args.resource, 'delete', 'delete')
    if (!lookup.ok) return invalid(lookup.message)
    const r = lookup.value
    const result = await ctx.v1.deleteBulk(r.path, args.ids)
    if (!result.ok) return failure(`Bulk delete on ${r.path} failed`, result)
    return success({ deleted: r.name, ids: args.ids, result: result.data })
  },
})

export const deleteCollectionRemove = defineTool({
  name: 'delete_collection_remove',
  description:
    'Remove items from a removable collection of an entity by child id (layer 1), e.g. unlink test cases from a test plan. ' +
    'To unassign a person prefer write_unassign.',
  input: { resource, id, collection: collectionName, childIds: ids(BULK_LIMIT) },
  handler: (args, ctx) => removeFromCollection(ctx, 'delete', args),
})

export const genericWriteTools = [writeCreate, writeUpdate, writeBulk, writeCollectionAdd]
export const genericDeleteTools = [deleteEntityTool, deleteBulk, deleteCollectionRemove]

const adminNote = ' Administration resources only (Project, Team, User, Process, Workflow, EntityState, Role, CustomField, ...). Requires an administrator token.'

export const adminTools = [
  defineTool({
    name: 'admin_create',
    description: `Create a configuration/administration entity.${adminNote}`,
    input: { resource, fields },
    handler: (args, ctx) => createEntity(ctx, 'admin', args),
  }),
  defineTool({
    name: 'admin_update',
    description: `Update a configuration/administration entity.${adminNote}`,
    input: { resource, id, fields },
    handler: (args, ctx) => updateEntity(ctx, 'admin', args),
  }),
  defineTool({
    name: 'admin_delete',
    description: `Delete a configuration/administration entity.${adminNote}`,
    input: { resource, id },
    handler: (args, ctx) => deleteEntity(ctx, 'admin', args),
  }),
  defineTool({
    name: 'admin_collection_add',
    description: `Add items to a collection of a configuration/administration entity (e.g. Team TeamMembers).${adminNote}`,
    input: { resource, id, collection: collectionName, items: collectionItems },
    handler: (args, ctx) => addToCollection(ctx, 'admin', args),
  }),
  defineTool({
    name: 'admin_collection_remove',
    description: `Remove items from a collection of a configuration/administration entity.${adminNote}`,
    input: { resource, id, collection: collectionName, childIds: ids(BULK_LIMIT) },
    handler: (args, ctx) => removeFromCollection(ctx, 'admin', args),
  }),
]
