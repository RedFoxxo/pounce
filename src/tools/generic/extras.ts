import { z } from 'zod'
import { htmlToText } from '../../format/html.js'
import { compact, num, person, ref } from '../../format/shape.js'
import { cardInfo } from '../../resolve/card.js'
import { unresolved } from '../workflow/common.js'
import { failure, invalid, success } from '../respond.js'
import { id, limit, resource as resourceArg } from '../schema.js'
import { defineTool } from '../types.js'

type Raw = Record<string, unknown>

// ---------------------------------------------------------------- v2

export const readV2Query = defineTool({
  name: 'read_v2_query',
  description:
    'Targetprocess REST v2 query (read-only): select with projections and nested collections, where with ==, !=, and, or, ' +
    'aggregations via result, board filter DSL via filter. Entity is singular (userStory, bug, feature). v2 omits null fields: absence means null. ' +
    'Filters use v2 syntax, e.g. where=(entityState.isFinal==false and project.id==2), select={id,name,tasks.count as tasks}.',
  input: {
    entity: z.string().regex(/^[A-Za-z]+$/, 'entity name, e.g. userStory'),
    select: z.string().optional(),
    where: z.string().optional(),
    result: z.string().optional().describe('Aggregation, e.g. {count:count,effort:sum(effort)}; returns one object'),
    orderBy: z.string().optional().describe('e.g. "createDate desc"'),
    filter: z.string().optional(),
    includeDeleted: z.boolean().optional(),
    limit: limit(500, 5000),
  },
  handler: async (args, ctx) => {
    if (args.result) {
      const r = await ctx.v2.aggregate(args.entity, args.result, {
        ...(args.where ? { where: args.where } : {}),
        ...(args.filter ? { filter: args.filter } : {}),
      })
      if (!r.ok) return failure(`v2 aggregation on ${args.entity} failed`, r)
      return success({ entity: args.entity, result: r.data })
    }
    const r = await ctx.v2.query<Raw>(args.entity, {
      ...(args.select ? { select: args.select } : {}),
      ...(args.where ? { where: args.where } : {}),
      ...(args.orderBy ? { orderBy: args.orderBy } : {}),
      ...(args.filter ? { filter: args.filter } : {}),
      ...(args.includeDeleted ? { includeDeleted: true } : {}),
      limit: args.limit ?? 500,
    })
    if (!r.ok) return failure(`v2 query on ${args.entity} failed`, r)
    return success({ entity: args.entity, count: r.data.items.length, truncated: r.data.truncated, items: r.data.items })
  },
})

export const readDeleted = defineTool({
  name: 'read_deleted',
  description: 'Deleted projects or users (v2 includeDeleted), with their delete date. Restoring them needs admin_undelete and an administrator token.',
  input: { kind: z.enum(['projects', 'users']), limit: limit(500, 5000) },
  handler: async (args, ctx) => {
    const select = args.kind === 'users' ? '{id,firstName,lastName,login,email,deleteDate}' : '{id,name,abbreviation,deleteDate}'
    const r = await ctx.v2.query<Raw>(args.kind, { where: '(deleteDate!=null)', select, includeDeleted: true, limit: args.limit ?? 500 })
    if (!r.ok) return failure(`Could not read deleted ${args.kind}`, r)
    return success({ kind: args.kind, count: r.data.items.length, truncated: r.data.truncated, items: r.data.items })
  },
})

// ---------------------------------------------------------------- history

function historyEntry(raw: Raw) {
  const changes = typeof raw.Changes === 'string' ? raw.Changes.split(',').map((c) => c.trim()).filter((c) => c && c !== 'ModifyDate') : []
  const values: Record<string, unknown> = {}
  for (const field of changes) {
    const v = raw[field]
    if (v === undefined) continue
    values[field] = v && typeof v === 'object' ? ref(v)?.name ?? ref(v)?.id ?? v : typeof v === 'string' && /<[A-Za-z/][^>]*>/.test(v) ? htmlToText(v) : v
  }
  return compact({ id: num(raw.Id), date: raw.Date, modification: raw.Modification, by: person(raw.Modifier), changed: changes, values: Object.keys(values).length ? values : undefined })
}

function simpleEntry(raw: Raw) {
  return compact({
    id: num(raw.Id),
    date: raw.Date,
    by: person(raw.Modifier),
    state: ref(raw.EntityState)?.name,
    effort: num(raw.Effort),
    effortCompleted: num(raw.EffortCompleted),
    effortToDo: num(raw.EffortToDo),
    release: ref(raw.Release)?.name,
    iteration: ref(raw.Iteration)?.name,
    project: ref(raw.Project)?.name,
  })
}

export const readHistory = defineTool({
  name: 'read_history',
  description:
    'Change history of an entity, oldest first (when capped, the most recent entries are kept). Simple history (default) records state, ' +
    'effort, release and iteration changes; full: true ' +
    'returns every change with the changed fields and their new values. resource is resolved from the id for cards; pass it for other entities (e.g. Comment).',
  input: { id, resource: resourceArg.optional(), full: z.boolean().optional(), limit: limit(200, 5000) },
  handler: async (args, ctx) => {
    const catalog = await ctx.catalog()
    let r
    if (args.resource) {
      const lookup = catalog.resource(args.resource)
      if (!lookup.ok) return invalid(lookup.message)
      r = lookup.value
    } else {
      const info = await cardInfo(ctx, args.id)
      if (!info.ok) return unresolved(info)
      r = info.value.resource
    }
    const max = args.limit ?? 200
    if (args.full) {
      const hist = catalog.find(`${r.name}Histories`)
      if (!hist?.available) return invalid(`${r.name} has no full history resource (${r.name}Histories) on this instance`)
      const res = await ctx.v1.list<Raw>(hist.path, { where: `(SourceEntityId eq ${args.id})`, orderByDesc: 'Date', limit: max })
      if (!res.ok) return failure(`Could not read the full history of ${r.name} ${args.id}`, res)
      return success({ resource: r.name, id: args.id, kind: 'full', count: res.data.items.length, truncated: res.data.truncated, entries: [...res.data.items].reverse().map(historyEntry) })
    }
    const inner = r.collections.some((c) => c.name === 'History')
    const simple = catalog.find(`${r.name}SimpleHistories`)
    if (!inner && !simple?.available) return invalid(`${r.name} has no simple history; try full: true`)
    const res = inner
      ? await ctx.v1.list<Raw>(`${r.path}/${args.id}/History`, { orderByDesc: 'Date', limit: max })
      : await ctx.v1.list<Raw>(simple!.path, { where: `(${r.name}.Id eq ${args.id})`, orderByDesc: 'Date', limit: max })
    if (!res.ok) return failure(`Could not read the history of ${r.name} ${args.id}`, res)
    return success({ resource: r.name, id: args.id, kind: 'simple', count: res.data.items.length, truncated: res.data.truncated, entries: [...res.data.items].reverse().map(simpleEntry) })
  },
})

// ---------------------------------------------------------------- context, conversions

export const readContext = defineTool({
  name: 'read_context',
  description:
    'Targetprocess Context for entity ids, or for projects and teams: processes and their practices (e.g. IsStoryEffortEqualsSumTasksEffort), ' +
    'terms, custom field definitions, selected projects/teams, logged user, version. Without arguments: the global context.',
  input: {
    ids: z.array(id).optional().describe('Entity ids'),
    projectIds: z.array(id).optional(),
    teamIds: z.array(id).optional(),
    acid: z.string().regex(/^[0-9A-Fa-f]+$/).optional(),
  },
  handler: async (args, ctx) => {
    const query: Record<string, string> = {}
    if (args.ids?.length) query.ids = args.ids.join(',')
    if (args.projectIds?.length) query.projectIds = args.projectIds.join(',')
    if (args.teamIds?.length) query.teamIds = args.teamIds.join(',')
    if (args.acid) query.acid = args.acid
    const r = await ctx.v1.getPath<Raw>('Context', query)
    if (!r.ok) return failure('Could not read the context', r)
    return success(r.data)
  },
})

export const readConversions = defineTool({
  name: 'read_conversions',
  description: 'Type conversions of a card (e.g. a bug converted to a user story): the id it has now if it was converted, and the ids it was converted from.',
  input: { id },
  handler: async (args, ctx) => {
    const include = '[Id,FromGeneralID,FromGeneralType[Id,Name],ActualGeneral[Id,Name,EntityType[Name]]]'
    const [from, to] = await Promise.all([
      ctx.v1.list<Raw>('GeneralConversions', { where: `(FromGeneralID eq ${args.id})`, include }),
      ctx.v1.list<Raw>('GeneralConversions', { where: `(ActualGeneral.Id eq ${args.id})`, include }),
    ])
    if (!from.ok) return failure(`Could not read conversions of ${args.id}`, from)
    if (!to.ok) return failure(`Could not read conversions of ${args.id}`, to)
    const shape = (c: Raw) =>
      compact({
        from: { id: num(c.FromGeneralID), type: ref(c.FromGeneralType)?.name },
        now: compact({ id: ref(c.ActualGeneral)?.id, name: ref(c.ActualGeneral)?.name, type: ((c.ActualGeneral as Raw | undefined)?.EntityType as Raw | undefined)?.Name }),
      })
    return success({ id: args.id, convertedTo: from.data.items.map(shape), convertedFrom: to.data.items.map(shape) })
  },
})

// ---------------------------------------------------------------- storage

const storageName = z.string().regex(/^[\w.\-]+$/, 'letters, digits, _ . - only')

export const readStorage = defineTool({
  name: 'read_storage',
  description:
    'RESTful storage, where views, boards and settings live. No group: list groups. Group: its storages (select/where in storage syntax, ' +
    'e.g. select={key,publicData.name} where=(scope == "Public")). Group + key: one storage with publicData and userData.',
  input: { group: storageName.optional(), key: storageName.optional(), select: z.string().optional(), where: z.string().optional(), limit: limit(100, 1000) },
  handler: async (args, ctx) => {
    if (args.key && !args.group) return invalid('key needs a group')
    const path = `/storage/v1/${args.group ? encodeURIComponent(args.group) : ''}${args.key ? `/${encodeURIComponent(args.key)}` : ''}`
    const query = args.key ? {} : { select: args.select, where: args.where, take: args.group ? (args.limit ?? 100) : 1000 }
    const r = await ctx.http.request<Raw>({ method: 'GET', path, query })
    if (!r.ok) return failure(`Could not read storage ${args.group ?? ''}${args.key ? `/${args.key}` : ''}`, r)
    if (args.key) return success(r.data)
    const list = Array.isArray(r.data?.items) ? r.data.items : []
    return success({ group: args.group, count: list.length, truncated: Boolean(r.data?.next), items: list })
  },
})

export const writeStorage = defineTool({
  name: 'write_storage',
  description:
    'Create or update a storage entry. MERGE semantics: posted publicData/userData keys are added or overwritten, other keys are kept, a null ' +
    'value deletes that key. A missing group or key is created. Scope and publicData can be changed only by the owner or an administrator.',
  input: {
    group: storageName,
    key: storageName.optional().describe('Omit to let Targetprocess generate one'),
    scope: z.enum(['Public', 'Private']).optional(),
    publicData: z.record(z.string(), z.unknown()).optional(),
    userData: z.record(z.string(), z.unknown()).optional(),
  },
  handler: async (args, ctx) => {
    if (!args.scope && !args.publicData && !args.userData) return invalid('pass scope, publicData and/or userData')
    const body: Raw = {}
    if (args.scope) body.scope = args.scope
    if (args.publicData) body.publicData = args.publicData
    if (args.userData) body.userData = args.userData
    const path = `/storage/v1/${encodeURIComponent(args.group)}${args.key ? `/${encodeURIComponent(args.key)}` : ''}`
    const r = await ctx.http.request<Raw>({ method: 'POST', path, json: body })
    if (!r.ok) return failure(`Could not write storage ${args.group}${args.key ? `/${args.key}` : ''}`, r)
    const key = args.key ?? (typeof r.data?.key === 'string' ? r.data.key : undefined)
    const back = key ? await ctx.http.request<Raw>({ method: 'GET', path: `/storage/v1/${encodeURIComponent(args.group)}/${encodeURIComponent(key)}` }) : undefined
    const notPersisted: string[] = []
    if (back?.ok) {
      for (const [section, data] of [['publicData', args.publicData], ['userData', args.userData]] as const) {
        const got = (back.data?.[section] ?? {}) as Raw
        for (const [k, v] of Object.entries(data ?? {})) {
          if (v === null ? k in got && got[k] !== null : JSON.stringify(got[k]) !== JSON.stringify(v)) notPersisted.push(`${section}.${k}`)
        }
      }
    }
    return success({ group: args.group, key, storage: back?.ok ? back.data : r.data, ...(notPersisted.length ? { notPersisted } : {}) })
  },
})

export const deleteStorage = defineTool({
  name: 'delete_storage',
  description: 'Delete one storage entry (only its owner or an administrator can).',
  input: { group: storageName, key: storageName },
  handler: async (args, ctx) => {
    const path = `/storage/v1/${encodeURIComponent(args.group)}/${encodeURIComponent(args.key)}`
    const before = await ctx.http.request<Raw>({ method: 'GET', path })
    if (!before.ok) return failure(`Could not read storage ${args.group}/${args.key}; nothing was deleted`, before)
    const r = await ctx.http.request({ method: 'DELETE', path })
    if (!r.ok) return failure(`Could not delete storage ${args.group}/${args.key}`, r)
    return success({ deleted: { group: args.group, key: args.key } })
  },
})

// ---------------------------------------------------------------- attachments

const MAX_UPLOAD_BYTES = 50 * 1024 * 1024

export const writeAttachment = defineTool({
  name: 'write_attachment',
  description:
    'Upload files to a card (UploadFile.ashx). Each file is given as base64 content with a name; pounce never reads local files itself, ' +
    'so the client decides what may be uploaded. Attachments are read back to confirm they arrived.',
  input: {
    id,
    files: z
      .array(z.object({ name: z.string().min(1), contentBase64: z.string().min(1), mimeType: z.string().optional() }))
      .min(1)
      .max(10),
  },
  handler: async (args, ctx) => {
    const form = new FormData()
    form.set('generalId', String(args.id))
    const names: string[] = []
    for (const f of args.files) {
      if (!/^[A-Za-z0-9+/]+={0,2}$/.test(f.contentBase64.replace(/\s+/g, ''))) return invalid(`${f.name}: contentBase64 is not valid base64`)
      const bytes = Buffer.from(f.contentBase64, 'base64')
      if (bytes.byteLength > MAX_UPLOAD_BYTES) return invalid(`${f.name} is larger than ${MAX_UPLOAD_BYTES / 1024 / 1024} MB`)
      names.push(f.name)
      form.append('file', new Blob([bytes], { type: f.mimeType ?? 'application/octet-stream' }), f.name)
    }
    // Existing attachments first, so an older file with the same name cannot pass for a failed upload.
    const listAttachments = () => ctx.v1.list<Raw>('Attachments', { where: `(General.Id eq ${args.id})`, include: '[Id,Name,Date]', limit: 20_000 })
    const before = await listAttachments()
    if (!before.ok) return failure(`Could not read the attachments of ${args.id}; nothing was uploaded`, before)
    const known = new Set(before.data.items.map((a) => num(a.Id)))
    const r = await ctx.http.request<string>({ method: 'POST', path: '/UploadFile.ashx', form, expect: 'text' })
    if (!r.ok) return failure(`Could not upload to ${args.id}`, r)
    const back = await listAttachments()
    const fresh = back.ok ? back.data.items.filter((a) => !known.has(num(a.Id))) : []
    const missing = [...names]
    for (const a of fresh) {
      const i = missing.indexOf(String(a.Name))
      if (i >= 0) missing.splice(i, 1)
    }
    return success({
      card: args.id,
      uploaded: names,
      attachments: fresh.filter((a) => names.includes(String(a.Name))).map((a) => ({ id: num(a.Id), name: a.Name })),
      ...(back.ok && missing.length ? { notPersisted: missing.map((n) => `${n} did not appear as a new attachment`) } : {}),
      ...(!back.ok ? { warning: `Could not read attachments back: ${back.message}` } : {}),
    })
  },
})

// ---------------------------------------------------------------- undelete

export const adminUndelete = defineTool({
  name: 'admin_undelete',
  description:
    'Restore deleted entities by id and entity type (e.g. UserStory, Project, User). Administrator token required. Comments, milestones and ' +
    'programs cannot be undeleted.',
  input: {
    items: z.array(z.object({ id, entityType: z.string().regex(/^[A-Za-z]+$/) })).min(1).max(500),
  },
  handler: async (args, ctx) => {
    const refused = args.items.filter((i) => ['comment', 'milestone', 'program'].includes(i.entityType.toLowerCase()))
    if (refused.length) return invalid(`${refused.map((i) => i.entityType).join(', ')} cannot be undeleted`)
    const payload = args.items.map((i) => ({ Id: i.id, EntityType: i.entityType }))
    const r = payload.length === 1 ? await ctx.v1.postPath('undelete', payload[0]) : await ctx.v1.postPath('undelete/bulk', payload)
    if (!r.ok) return failure(r.status === 403 ? 'Undelete refused: an administrator token is required' : 'Undelete failed', r)
    return success({ restored: payload, result: r.data })
  },
})

export const extraReadTools = [readV2Query, readHistory, readContext, readConversions, readDeleted, readStorage]
export const extraWriteTools = [writeStorage, writeAttachment]
export const extraDeleteTools = [deleteStorage]
export const extraAdminTools = [adminUndelete]
