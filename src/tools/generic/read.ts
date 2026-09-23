import { z } from 'zod'
import type { CatalogResource } from '../../catalog/types.js'
import { INNER_CAP, innerCapNote } from '../../format/shape.js'
import { failure, invalid, success } from '../respond.js'
import { bracket, id, include, limit, resource } from '../schema.js'
import { defineTool } from '../types.js'
import { isAdminResource, READ_ONLY_IN_PRACTICE } from './policy.js'

/** Inner collections are capped at 25 unless asked; ask for the maximum. */
export const INNER_TAKE = INNER_CAP

const QUERY_DEFAULT = 500
const QUERY_MAX = 5000

function operations(r: CatalogResource): string[] {
  return [r.canCreate && 'create', r.canUpdate && 'update', r.canDelete && 'delete'].filter((o): o is string => Boolean(o))
}

/** `CreateDate desc` → orderByDesc, since v1 rejects the suffix. */
export function sortParams(orderBy?: string, orderByDesc?: string): { orderBy?: string; orderByDesc?: string } {
  const out: { orderBy?: string; orderByDesc?: string } = {}
  if (orderBy) {
    const m = /^\s*(\S+)\s+(asc|desc)\s*$/i.exec(orderBy)
    if (m?.[2]?.toLowerCase() === 'desc') out.orderByDesc = m[1] as string
    else out.orderBy = m ? (m[1] as string) : orderBy.trim()
  }
  if (orderByDesc) out.orderByDesc = orderByDesc.trim()
  return out
}

const queryShape = {
  where: z
    .string()
    .optional()
    .describe("v1 filter, e.g. (EntityState.Name eq 'Open') and (Project.Id eq 2). Operators: eq, ne, gt, gte, lt, lte, contains, in; no 'or'."),
  include: include.describe('Fields to return, e.g. [Id,Name,Project[Name],Tasks[Id,Name]]'),
  exclude: include.describe('Fields to omit'),
  append: include.describe('Calculated values, e.g. [Tasks-Count,Bugs-Count]'),
  orderBy: z.string().optional().describe('Sort field; "Field desc" is accepted'),
  orderByDesc: z.string().optional().describe('Sort field, descending'),
  limit: limit(QUERY_DEFAULT, QUERY_MAX),
}

export const readMeta = defineTool({
  name: 'read_meta',
  description:
    'Describe the Targetprocess resource catalog read from this instance. Without a resource: every resource with its operations. ' +
    'With a resource: its fields (settable/required), references and collections (addable/removable). Use it before read_query/write_create.',
  input: { resource: resource.optional() },
  handler: async ({ resource: name }, ctx) => {
    const catalog = await ctx.catalog()
    const header = {
      source: catalog.source,
      instance: catalog.data.instance,
      version: catalog.data.version,
      capturedAt: catalog.data.capturedAt,
    }
    if (!name) {
      return success({
        ...header,
        count: catalog.resources.length,
        resources: catalog.resources.map((r) => ({
          name: r.name,
          path: r.path,
          operations: r.available ? operations(r) : 'unavailable',
          ...(r.listed ? {} : { listed: false }),
          ...(isAdminResource(r) ? { admin: true } : {}),
        })),
      })
    }
    const lookup = catalog.resource(name)
    if (!lookup.ok) return invalid(lookup.message)
    const r = lookup.value
    return success({
      ...header,
      name: r.name,
      path: r.path,
      description: r.description,
      operations: operations(r),
      writeTools: READ_ONLY_IN_PRACTICE[r.name] ? 'none (read-only in practice)' : isAdminResource(r) ? 'admin_*' : 'write_*/delete_*',
      bases: r.bases,
      values: r.values.map((f) => ({ name: f.name, type: f.type, set: f.canSet, ...(f.required ? { required: true } : {}), ...(f.deprecated ? { deprecated: true } : {}) })),
      references: r.references.map((f) => ({ name: f.name, type: f.type, set: f.canSet, ...(f.required ? { required: true } : {}) })),
      collections: r.collections.map((c) => ({ name: c.name, type: c.type, add: c.canAdd, remove: c.canRemove })),
    })
  },
})

export const readGet = defineTool({
  name: 'read_get',
  description:
    'GET one entity of any resource by id (layer 1). Inner collections in include return up to 1000 items. Prefer read_card for cards.',
  input: {
    resource,
    id,
    include: queryShape.include,
    exclude: queryShape.exclude,
    append: queryShape.append,
  },
  handler: async (args, ctx) => {
    const lookup = (await ctx.catalog()).resource(args.resource)
    if (!lookup.ok) return invalid(lookup.message)
    const r = lookup.value
    const result = await ctx.v1.get(r.path, args.id, {
      include: bracket(args.include),
      exclude: bracket(args.exclude),
      append: bracket(args.append),
      innerTake: INNER_TAKE,
    })
    if (!result.ok) return failure(`Could not read ${r.name} ${args.id}`, result)
    const note = innerCapNote(result.data)
    return success(Object.keys(note).length ? { ...(result.data as Record<string, unknown>), ...note } : result.data)
  },
})

export const readQuery = defineTool({
  name: 'read_query',
  description:
    'Query a collection of any resource with where/include/orderBy (layer 1), fully paged. ' +
    `Returns {count, truncated, items}; default limit ${QUERY_DEFAULT}, max ${QUERY_MAX}. Use include to keep results small.`,
  input: { resource, ...queryShape },
  handler: async (args, ctx) => {
    const lookup = (await ctx.catalog()).resource(args.resource)
    if (!lookup.ok) return invalid(lookup.message)
    const r = lookup.value
    const result = await ctx.v1.list(r.path, {
      where: args.where,
      include: bracket(args.include),
      exclude: bracket(args.exclude),
      append: bracket(args.append),
      ...sortParams(args.orderBy, args.orderByDesc),
      innerTake: INNER_TAKE,
      limit: args.limit ?? QUERY_DEFAULT,
    })
    if (!result.ok) return failure(`Query on ${r.path} failed`, result)
    return success({ resource: r.name, count: result.data.items.length, truncated: result.data.truncated, ...innerCapNote(result.data.items), items: result.data.items })
  },
})

export const readCollection = defineTool({
  name: 'read_collection',
  description:
    'GET an inner collection of an entity, e.g. UserStories/{id}/Tasks or Bugs/{id}/Comments (layer 1), fully paged. See read_meta for collection names.',
  input: {
    resource,
    id,
    collection: z.string().min(1).describe('Collection name, e.g. Tasks, Comments, Assignments'),
    ...queryShape,
  },
  handler: async (args, ctx) => {
    const catalog = await ctx.catalog()
    const lookup = catalog.resource(args.resource)
    if (!lookup.ok) return invalid(lookup.message)
    const r = lookup.value
    const coll = catalog.collection(r, args.collection)
    if (!coll.ok) return invalid(coll.message)
    const result = await ctx.v1.list(`${r.path}/${args.id}/${coll.value.name}`, {
      where: args.where,
      include: bracket(args.include),
      exclude: bracket(args.exclude),
      append: bracket(args.append),
      ...sortParams(args.orderBy, args.orderByDesc),
      innerTake: INNER_TAKE,
      limit: args.limit ?? QUERY_DEFAULT,
    })
    if (!result.ok) return failure(`Could not read ${r.path}/${args.id}/${coll.value.name}`, result)
    return success({
      resource: r.name,
      id: args.id,
      collection: coll.value.name,
      count: result.data.items.length,
      truncated: result.data.truncated,
      ...innerCapNote(result.data.items),
      items: result.data.items,
    })
  },
})

export const genericReadTools = [readMeta, readGet, readQuery, readCollection]
