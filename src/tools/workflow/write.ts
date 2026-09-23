import { z } from 'zod'
import { describeAssignment, listAssignments, addAssignment, removeAssignment, type AssignmentRow } from '../../domain/assignments.js'
import { readCardAs, readCardRaw, shapeCard, type CardRead } from '../../domain/cards.js'
import { isCardResource, parentLink } from '../../domain/create.js'
import { prepareCustomFields, unpersistedCustomFields, type CustomFieldValue } from '../../domain/custom-fields.js'
import { applyRoleEfforts, roleEffortRows, type EffortRequest } from '../../domain/efforts.js'
import { parentChange, parentSnapshot, snapshotAgain, snapshotOf } from '../../domain/parent.js'
import { htmlToText, textToHtml } from '../../format/html.js'
import { items, num, ref, tags as parseTags } from '../../format/shape.js'
import type { Err, Result } from '../../http/result.js'
import { cardInfo } from '../../resolve/card.js'
import { fullName, type TpRole, type TpUser } from '../../resolve/directory.js'
import { match, type Resolved } from '../../resolve/match.js'
import type { ToolContext } from '../context.js'
import { prepareBody, unpersisted } from '../generic/validate.js'
import { isAdminResource } from '../generic/policy.js'
import { failure, invalid, success } from '../respond.js'
import { id } from '../schema.js'
import { defineTool, type ToolOutput } from '../types.js'
import { unresolved } from './common.js'

type Raw = Record<string, unknown>

const nameOrId = z.union([z.string().min(1), z.number().int().positive()])
const person = nameOrId.describe('Person name, login, email, id, or "me"')
const role = nameOrId.describe('Role name or id, e.g. "Developer", "Product Owner"')
const effortValue = z.number().min(0).max(100_000)

type Loaded = { ok: true; card: CardRead } | { ok: false; output: ToolOutput }

async function load(ctx: ToolContext, cardId: number): Promise<Loaded> {
  const r = await readCardRaw(ctx, cardId)
  if (!r.ok) return { ok: false, output: r.error ? failure(r.message, r.error) : invalid(r.message) }
  return { ok: true, card: r.value }
}

function reload(ctx: ToolContext, card: CardRead): Promise<Result<Raw>> {
  return readCardAs(ctx, card.info)
}

/** The write was sent but could not be read back: say so instead of guessing a verdict. */
function unverified(r: Err): { warning: string } {
  return { warning: `The write was sent but the card could not be read back to verify it (${r.message})` }
}

function resolvePerson(ctx: ToolContext, input: string | number, options: { allowInactive?: boolean } = {}): Promise<Resolved<TpUser>> {
  return String(input).trim().toLowerCase() === 'me' ? ctx.directory.me() : ctx.directory.user(input, options)
}

function cardLabel(card: CardRead) {
  return { id: card.info.id, type: card.info.entityType, name: card.info.name }
}

/** A write failed after earlier writes succeeded: say exactly what was done. */
function partial(summary: string, error: Err, done: unknown[]): ToolOutput {
  return failure(summary, error, done.length ? { alreadyDone: done } : undefined)
}

// ---------------------------------------------------------------- state

export const writeSetState = defineTool({
  name: 'write_set_state',
  description:
    "Move a card to a state by name or id, resolved against the card's own project workflow. With team, sets that team's state on the card " +
    '(its team sub-workflow, or the project workflow when the team has none). The state is read back, and the parent card\'s state before/after is ' +
    'reported: moving a task out of its initial state can advance its user story.',
  input: { id, state: nameOrId.describe('State name or id, e.g. "In Progress", "Coded"'), team: nameOrId.optional().describe('Team whose team state to set') },
  handler: async (args, ctx) => {
    const loaded = await load(ctx, args.id)
    if (!loaded.ok) return loaded.output
    const card = loaded.card
    const { info, raw } = card
    if (info.processId === undefined || !info.project) return invalid(`${info.entityType} ${info.id} has no project workflow`)
    const parentBefore = await parentSnapshot(ctx, raw)

    if (args.team !== undefined) {
      // A team already on the card may since have been deactivated; its state can still be set.
      const team = await ctx.directory.team(args.team, { allowInactive: true })
      if (!team.ok) return unresolved(team)
      const assignment = items(raw.AssignedTeams).find((t) => ref(t.Team)?.id === team.value.Id)
      if (!assignment) {
        const teams = items(raw.AssignedTeams).map((t) => ref(t.Team)?.name).filter(Boolean)
        return invalid(`${team.value.Name} is not assigned to ${info.entityType} ${info.id}; assigned teams: ${teams.join(', ') || 'none'}`)
      }
      const states = await ctx.directory.teamStates(info.processId, info.entityType, team.value.Id, info.project.id)
      if (!states.ok) return failure(`Could not load ${team.value.Name} states`, states)
      const target = match(args.state, { kind: `${team.value.Name} ${info.entityType} state`, items: states.data, id: (s) => s.Id, name: (s) => s.Name, keys: (s) => [s.Name] })
      if (!target.ok) return unresolved(target)
      const before = ref(assignment.EntityState)
      const taId = num(assignment.Id) as number
      if (before?.id !== target.value.Id) {
        const r = await ctx.v1.update('TeamAssignments', taId, { EntityState: { Id: target.value.Id } })
        if (!r.ok) return failure(`Could not set ${team.value.Name} state of ${info.entityType} ${info.id} to ${target.value.Name}`, r)
      }
      const after = await reload(ctx, card)
      const parentAfter = await snapshotAgain(ctx, parentBefore)
      if (!after.ok) {
        return success({
          card: cardLabel(card),
          team: { id: team.value.Id, name: team.value.Name },
          teamState: { before: before?.name, requested: target.value.Name },
          parent: parentChange(parentBefore, parentAfter),
          ...unverified(after),
        })
      }
      const afterTeam = items(after.data.AssignedTeams).find((t) => num(t.Id) === taId)
      const got = ref(afterTeam?.EntityState)
      return success({
        card: cardLabel(card),
        team: { id: team.value.Id, name: team.value.Name },
        teamState: { before: before?.name, requested: target.value.Name, after: got?.name, changed: before?.id !== got?.id },
        cardState: ref(after.data.EntityState)?.name,
        parent: parentChange(parentBefore, parentAfter),
        ...(got?.id !== target.value.Id ? { notPersisted: [`team state: requested ${target.value.Name}, got ${got?.name ?? 'no team assignment'}`] } : {}),
      })
    }

    const target = await ctx.directory.state(info.processId, info.entityType, args.state)
    if (!target.ok) return unresolved(target)
    const before = ref(raw.EntityState)
    if (before?.id === target.value.Id) {
      return success({ card: cardLabel(card), state: { before: before.name, requested: target.value.Name, after: before.name, changed: false } })
    }
    const r = await ctx.v1.update(info.resource.path, info.id, { EntityState: { Id: target.value.Id } })
    if (!r.ok) return failure(`Could not move ${info.entityType} ${info.id} to ${target.value.Name}`, r)
    const after = await reload(ctx, card)
    const parentAfter = await snapshotAgain(ctx, parentBefore)
    if (!after.ok) {
      return success({ card: cardLabel(card), state: { before: before?.name, requested: target.value.Name }, parent: parentChange(parentBefore, parentAfter), ...unverified(after) })
    }
    const got = ref(after.data.EntityState)
    return success({
      card: cardLabel(card),
      state: { before: before?.name, requested: target.value.Name, after: got?.name, changed: before?.id !== got?.id },
      teamStates: items(after.data.AssignedTeams).map((t) => ({ team: ref(t.Team)?.name, state: ref(t.EntityState)?.name })),
      parent: parentChange(parentBefore, parentAfter),
      ...(got?.id !== target.value.Id ? { notPersisted: [`state: requested ${target.value.Name}, got ${got?.name ?? 'no state'}`] } : {}),
    })
  },
})

// ---------------------------------------------------------------- assignments

export const writeAssign = defineTool({
  name: 'write_assign',
  description:
    'Assign a person to a card in a role (names or ids; "me" for yourself). Adds to existing assignments; with exclusive: true, also removes ' +
    'everyone else in that role (only when explicitly asked). Reports what was removed and reads the assignments back.',
  input: { id, user: person, role, exclusive: z.boolean().optional() },
  handler: async (args, ctx) => {
    const info = await cardInfo(ctx, args.id)
    if (!info.ok) return unresolved(info)
    if (!(await ctx.catalog()).member(info.value.resource, 'Assignments')) return invalid(`A ${info.value.entityType} cannot have assignments`)
    const user = await resolvePerson(ctx, args.user)
    if (!user.ok) return unresolved(user)
    const r = await ctx.directory.role(args.role)
    if (!r.ok) return unresolved(r)
    const current = await listAssignments(ctx, args.id)
    if (!current.ok) return failure(`Could not read assignments of ${args.id}`, current)

    // Add first, so a failure never leaves the role empty.
    const done: unknown[] = []
    const already = current.data.some((a) => a.user.id === user.value.Id && a.role.id === r.value.Id)
    if (!already) {
      const add = await addAssignment(ctx, args.id, user.value.Id, r.value.Id)
      if (!add.ok) return failure(`Could not assign ${fullName(user.value)} as ${r.value.Name} on ${args.id}; nobody was removed`, add)
      done.push({ assigned: `${fullName(user.value)} as ${r.value.Name}` })
    }
    const removed: AssignmentRow[] = []
    if (args.exclusive) {
      for (const a of current.data.filter((a) => a.role.id === r.value.Id && a.user.id !== user.value.Id)) {
        const del = await removeAssignment(ctx, a.id)
        if (!del.ok) return partial(`Could not remove ${a.user.name} as ${a.role.name}`, del, done)
        removed.push(a)
        done.push({ removed: describeAssignment(a) })
      }
    }
    const after = await listAssignments(ctx, args.id)
    const assignments = after.ok ? after.data.map(describeAssignment) : undefined
    const present = after.ok && after.data.some((a) => a.user.id === user.value.Id && a.role.id === r.value.Id)
    const leftover = after.ok ? removed.filter((x) => after.data.some((a) => a.id === x.id)) : []
    return success({
      card: { id: args.id, type: info.value.entityType, name: info.value.name },
      assigned: { user: `${fullName(user.value)} (${user.value.Id})`, role: r.value.Name },
      alreadyAssigned: already,
      removed: removed.map(describeAssignment),
      assignments,
      ...(after.ok && (!present || leftover.length)
        ? {
            notPersisted: [
              ...(present ? [] : ['the assignment is not on the card after writing']),
              ...leftover.map((x) => `${x.user.name} as ${x.role.name} is still on the card after removing`),
            ],
          }
        : {}),
      ...(!after.ok ? { warning: `Could not read assignments back: ${after.message}` } : {}),
    })
  },
})

export const writeUnassign = defineTool({
  name: 'write_unassign',
  description: 'Remove one exact assignment (person + role) from a card. If the person holds several roles on the card, the role is required.',
  input: { id, user: person, role: role.optional() },
  handler: async (args, ctx) => {
    const user = await resolvePerson(ctx, args.user, { allowInactive: true })
    if (!user.ok) return unresolved(user)
    let roleId: number | undefined
    if (args.role !== undefined) {
      const r = await ctx.directory.role(args.role)
      if (!r.ok) return unresolved(r)
      roleId = r.value.Id
    }
    const current = await listAssignments(ctx, args.id)
    if (!current.ok) return failure(`Could not read assignments of ${args.id}`, current)
    const hits = current.data.filter((a) => a.user.id === user.value.Id && (roleId === undefined || a.role.id === roleId))
    if (hits.length === 0) {
      return invalid(`${fullName(user.value)} holds no such assignment on ${args.id}; assignments: ${current.data.map((a) => `${a.user.name} as ${a.role.name}`).join(', ') || 'none'}`)
    }
    if (hits.length > 1) return invalid(`${fullName(user.value)} holds ${hits.length} roles on ${args.id} (${hits.map((a) => a.role.name).join(', ')}); pass the role`)
    const target = hits[0] as AssignmentRow
    const del = await removeAssignment(ctx, target.id)
    if (!del.ok) return failure(`Could not remove ${target.user.name} as ${target.role.name} from ${args.id}`, del)
    const after = await listAssignments(ctx, args.id)
    const stillThere = after.ok && after.data.some((a) => a.id === target.id)
    return success({
      card: args.id,
      removed: describeAssignment(target),
      assignments: after.ok ? after.data.map(describeAssignment) : undefined,
      ...(stillThere ? { notPersisted: ['the assignment is still on the card'] } : {}),
      ...(!after.ok ? { warning: `Could not read assignments back: ${after.message}` } : {}),
    })
  },
})

// ---------------------------------------------------------------- efforts

/** Live: card and parent effort totals are recomputed a few seconds after a role-effort write. */
const TOTALS_NOTE = 'Targetprocess recomputes effort totals a few seconds after a write; totals read right after it may still change'

const effortsInput = z
  .array(z.object({ role, effort: effortValue.describe('Hours (or points) for this role') }))
  .min(1)
  .describe('Effort per role, e.g. [{"role":"Developer","effort":8}]')

async function resolveEfforts(ctx: ToolContext, list: { role: string | number; effort: number }[]): Promise<{ ok: true; value: EffortRequest[] } | { ok: false; output: ToolOutput }> {
  const out: EffortRequest[] = []
  for (const e of list) {
    const r = await ctx.directory.role(e.role)
    if (!r.ok) return { ok: false, output: unresolved(r) }
    const roleValue: TpRole = r.value
    if (roleValue.HasEffort === false) return { ok: false, output: invalid(`Role ${roleValue.Name} does not carry effort`) }
    if (out.some((x) => x.roleId === roleValue.Id)) return { ok: false, output: invalid(`Role ${roleValue.Name} is listed twice`) }
    out.push({ roleId: roleValue.Id, roleName: roleValue.Name, effort: e.effort })
  }
  return { ok: true, value: out }
}

export const writeSetRoleEffort = defineTool({
  name: 'write_set_role_effort',
  description:
    'Set effort for one or more roles on a card. Effort always belongs to a role: this writes the RoleEffort rows, never the card total, which ' +
    "Targetprocess computes from them. A role is required. Task role efforts ROLL UP into the parent user story, overwriting the story's value " +
    'for that role; the parent\'s before/after values are reported rather than assuming the story estimate was kept.',
  input: { id, efforts: effortsInput },
  handler: async (args, ctx) => {
    const loaded = await load(ctx, args.id)
    if (!loaded.ok) return loaded.output
    const card = loaded.card
    if (!(await ctx.catalog()).member(card.info.resource, 'RoleEfforts')) return invalid(`A ${card.info.entityType} has no role efforts`)
    const requests = await resolveEfforts(ctx, args.efforts)
    if (!requests.ok) return requests.output
    const parentBefore = await parentSnapshot(ctx, card.raw)
    const outcome = await applyRoleEfforts(ctx, card.info.id, roleEffortRows(card.raw), requests.value)
    if (outcome.error) return partial(outcome.error.message, outcome.error.error, outcome.applied)
    const after = await reload(ctx, card)
    const parentAfter = await snapshotAgain(ctx, parentBefore)
    if (!after.ok) {
      return success({ card: cardLabel(card), efforts: outcome.applied, parent: parentChange(parentBefore, parentAfter), ...unverified(after) })
    }
    const rows = roleEffortRows(after.data)
    const notPersisted = requests.value.flatMap((req) => {
      const got = rows.find((r) => r.role.id === req.roleId)?.effort
      return got === req.effort ? [] : [`${req.roleName}: requested ${req.effort}, got ${got ?? 'no row'}`]
    })
    return success({
      card: cardLabel(card),
      efforts: outcome.applied,
      total: { before: num(card.raw.Effort), after: num(after.data.Effort) },
      totalsNote: TOTALS_NOTE,
      parent: parentChange(parentBefore, parentAfter),
      ...(notPersisted.length ? { notPersisted } : {}),
    })
  },
})

// ---------------------------------------------------------------- custom fields

export const writeSetCustomFields = defineTool({
  name: 'write_set_custom_fields',
  description:
    "Set custom fields on a card by name, e.g. {\"BackEnd\":\"Done\",\"FrontEnd\":\"To Do\"}. Dropdown values are validated against the field's " +
    'options (see read_custom_field_options) before anything is sent; null clears a field. Values are read back.',
  input: { id, fields: z.record(z.string(), z.unknown()).refine((v) => Object.keys(v).length > 0, 'give at least one field') },
  handler: async (args, ctx) => {
    const loaded = await load(ctx, args.id)
    if (!loaded.ok) return loaded.output
    const card = loaded.card
    if (card.info.processId === undefined) return invalid(`${card.info.entityType} ${card.info.id} has no project/process`)
    const values = await prepareCustomFields(ctx, card.info.processId, card.info.entityType, args.fields)
    if (!values.ok) return unresolved(values)
    const r = await ctx.v1.update(card.info.resource.path, card.info.id, { CustomFields: values.value })
    if (!r.ok) return failure(`Could not set custom fields on ${card.info.entityType} ${card.info.id}`, r)
    const after = await reload(ctx, card)
    if (!after.ok) return success({ card: cardLabel(card), set: values.value, ...unverified(after) })
    const notPersisted = unpersistedCustomFields(values.value, after.data.CustomFields)
    return success({ card: cardLabel(card), set: values.value, ...(notPersisted.length ? { notPersisted } : {}) })
  },
})

// ---------------------------------------------------------------- teams

export const writeTeam = defineTool({
  name: 'write_team',
  description: 'Add teams to or remove teams from a card (names or ids). Adding appends; removing deletes only the named team assignment.',
  input: { id, add: z.array(nameOrId).optional(), remove: z.array(nameOrId).optional() },
  handler: async (args, ctx) => {
    if (!args.add?.length && !args.remove?.length) return invalid('pass add and/or remove')
    const loaded = await load(ctx, args.id)
    if (!loaded.ok) return loaded.output
    const card = loaded.card
    if (!(await ctx.catalog()).member(card.info.resource, 'AssignedTeams')) return invalid(`A ${card.info.entityType} cannot have teams`)
    const current = items(card.raw.AssignedTeams)
    const toAdd: { Id: number; Name: string }[] = []
    const toRemove: { taId: number; name: string; teamId: number }[] = []
    for (const t of args.add ?? []) {
      const team = await ctx.directory.team(t)
      if (!team.ok) return unresolved(team)
      if (!current.some((c) => ref(c.Team)?.id === team.value.Id)) toAdd.push({ Id: team.value.Id, Name: team.value.Name })
    }
    for (const t of args.remove ?? []) {
      const team = await ctx.directory.team(t, { allowInactive: true })
      if (!team.ok) return unresolved(team)
      const ta = current.find((c) => ref(c.Team)?.id === team.value.Id)
      if (!ta) return invalid(`${team.value.Name} is not assigned to ${card.info.entityType} ${card.info.id}`)
      toRemove.push({ taId: num(ta.Id) as number, name: team.value.Name, teamId: team.value.Id })
    }
    const done: unknown[] = []
    if (toAdd.length) {
      const r = await ctx.v1.update(card.info.resource.path, card.info.id, { AssignedTeams: { Items: toAdd.map((t) => ({ Team: { Id: t.Id } })) } })
      if (!r.ok) return failure(`Could not add teams to ${card.info.entityType} ${card.info.id}`, r)
      done.push({ added: toAdd.map((t) => t.Name) })
    }
    for (const t of toRemove) {
      const r = await ctx.v1.delete('TeamAssignments', t.taId)
      if (!r.ok) return partial(`Could not remove ${t.name} from ${card.info.entityType} ${card.info.id}`, r, done)
      done.push({ removed: t.name })
    }
    const after = await reload(ctx, card)
    if (!after.ok) return success({ card: cardLabel(card), added: toAdd.map((t) => t.Name), removed: toRemove.map((t) => t.name), ...unverified(after) })
    const teamIds = items(after.data.AssignedTeams).map((c) => ref(c.Team)?.id)
    const notPersisted = [
      ...toAdd.filter((t) => !teamIds.includes(t.Id)).map((t) => `${t.Name} is not on the card after adding`),
      ...toRemove.filter((t) => teamIds.includes(t.teamId)).map((t) => `${t.name} is still on the card after removing`),
    ]
    return success({
      card: cardLabel(card),
      added: toAdd.map((t) => t.Name),
      removed: toRemove.map((t) => t.name),
      teams: items(after.data.AssignedTeams).map((c) => ({ team: ref(c.Team)?.name, state: ref(c.EntityState)?.name })),
      ...(notPersisted.length ? { notPersisted } : {}),
    })
  },
})

// ---------------------------------------------------------------- shared verification

interface Verdict {
  notPersisted: string[]
  /** Sent, but not present in any read-back, so neither confirmed nor refuted. */
  notVerified: string[]
}

/**
 * Checks what create and update both write: name, description, tags,
 * release/iteration/team iteration, the parent link, and every extra field.
 * Fields the card read does not include are read explicitly; anything still
 * missing is listed as not verified rather than silently skipped.
 */
async function verifyWritten(
  ctx: ToolContext,
  info: CardRead['info'],
  body: Record<string, unknown>,
  after: Raw,
  opts: { parentField?: string | undefined; parentId?: number | undefined; extraFields?: Record<string, unknown> },
): Promise<Verdict> {
  const v: Verdict = { notPersisted: [], notVerified: [] }
  if (body.Name !== undefined && after.Name !== body.Name) v.notPersisted.push(`name: requested ${JSON.stringify(body.Name)}, got ${JSON.stringify(after.Name)}`)
  if (body.Description !== undefined && htmlToText(String(after.Description ?? '')) !== htmlToText(String(body.Description))) {
    v.notPersisted.push('description differs from what was sent')
  }
  if (body.Tags !== undefined) {
    const got = (parseTags(after.Tags) ?? []).map((t) => t.toLowerCase()).sort().join(',')
    const want = String(body.Tags).split(',').filter(Boolean).map((t) => t.toLowerCase()).sort().join(',')
    if (got !== want) v.notPersisted.push(`tags: requested [${want}], got [${got}]`)
  }
  for (const key of ['Release', 'Iteration', 'TeamIteration']) {
    if (!(key in body)) continue
    if (!(key in after)) {
      v.notVerified.push(key)
      continue
    }
    const want = (body[key] as { Id: number } | null)?.Id ?? null
    const got = ref(after[key])?.id ?? null
    if (want !== got) v.notPersisted.push(`${key}: requested ${want}, got ${got}`)
  }
  if (opts.parentField === 'TestPlans') {
    const plans = await ctx.v1.get<Raw>(info.resource.path, info.id, { include: '[Id,TestPlans[Id]]', innerTake: 1000 })
    if (!plans.ok) v.notVerified.push(`test plan link (${plans.message})`)
    else if (!items(plans.data.TestPlans).some((p) => num(p.Id) === opts.parentId)) v.notPersisted.push(`not in test plan ${opts.parentId}`)
  } else if (opts.parentField) {
    if (!(opts.parentField in after)) v.notVerified.push(opts.parentField)
    else if (ref(after[opts.parentField])?.id !== opts.parentId) {
      v.notPersisted.push(`${opts.parentField}: requested ${opts.parentId}, got ${ref(after[opts.parentField])?.id ?? null}`)
    }
  }
  const extra = opts.extraFields ?? {}
  const missing = Object.keys(extra).filter((k) => !(k in after))
  let extraBack: Raw = after
  if (missing.length) {
    const r = await ctx.v1.get<Raw>(info.resource.path, info.id, { include: `[Id,${missing.join(',')}]` })
    if (r.ok) extraBack = { ...after, ...r.data }
  }
  v.notPersisted.push(...unpersisted(extra, extraBack))
  v.notVerified.push(...Object.keys(extra).filter((k) => !(k in extraBack)))
  return v
}

function verdict(v: Verdict) {
  return { ...(v.notPersisted.length ? { notPersisted: v.notPersisted } : {}), ...(v.notVerified.length ? { notVerified: v.notVerified } : {}) }
}

// ---------------------------------------------------------------- create

const refOrNull = z.union([id, z.null()])
const tag = z.string().trim().min(1).refine((t) => !t.includes(','), 'a tag cannot contain a comma (Targetprocess separates tags with commas)')

/** Fields that have their own argument or tool, because a domain rule applies to them. */
const RESERVED: Record<string, string> = {
  Name: 'name',
  Description: 'description',
  Project: 'project (or parent, to inherit it)',
  EntityState: 'state, or write_set_state',
  Effort: 'write_set_role_effort: the card total is computed from role efforts',
  RoleEfforts: 'roleEfforts, or write_set_role_effort',
  Assignments: 'assignees, or write_assign',
  AssignedTeams: 'teams, or write_team',
  CustomFields: 'customFields, or write_set_custom_fields',
  Tags: 'tags',
  TagObjects: 'tags',
  Release: 'release',
  Iteration: 'iteration',
  TeamIteration: 'teamIteration',
  UserStory: 'parent',
  Feature: 'parent',
  Epic: 'parent',
  PortfolioEpic: 'parent',
  LinkedGeneral: 'parent',
  TestPlans: 'parent',
  Tasks: 'write_create_card with parent (children get the creation rules)',
  Bugs: 'write_create_card with parent (children get the creation rules)',
  UserStories: 'write_create_card with parent (children get the creation rules)',
  Features: 'write_create_card with parent (children get the creation rules)',
  Epics: 'write_create_card with parent (children get the creation rules)',
  TestCases: 'write_test_cases',
}

function reservedFields(fields: Record<string, unknown>): string | undefined {
  const hits = Object.keys(fields).flatMap((k) => {
    const key = Object.keys(RESERVED).find((r) => r.toLowerCase() === k.toLowerCase())
    return key ? [`${key} → use ${RESERVED[key]}`] : []
  })
  return hits.length ? `these fields have their own argument or tool:\n- ${hits.join('\n- ')}` : undefined
}

/** Today as YYYY-MM-DD in the server's local time zone. */
function localDate(now = new Date()): string {
  const pad = (n: number) => String(n).padStart(2, '0')
  return `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())}`
}

export const writeCreateCard = defineTool({
  name: 'write_create_card',
  description:
    'Create any card type (UserStory, Task, Bug, Feature, Epic, TestPlan, Request, ...) with parent, name, description, state, team, assignees, ' +
    'role efforts, tags and custom fields, applying the team rules: the project is inherited from the parent (task ← story, story ← feature, ' +
    'feature ← epic, bug ← its card, test case ← test plan) and the call fails before posting if none resolves; assignments Targetprocess adds by ' +
    'default are removed and reported, then exactly the requested people are assigned; effort is written per role; the parent\'s state and ' +
    'role efforts before/after are reported. Description: HTML is sent as-is, plain text lines become paragraphs.',
  input: {
    type: z.string().min(1).describe('Entity type, e.g. UserStory, Task, Bug'),
    name: z.string().min(1),
    parent: id.optional().describe('Parent card id (story for a task, feature for a story, test plan for a test case, ...)'),
    project: nameOrId.optional().describe('Project; defaults to the parent\'s project'),
    description: z.string().optional(),
    state: nameOrId.optional(),
    teams: z.array(nameOrId).optional().describe('Teams to assign; TP_DEFAULT_TEAM_ID applies when omitted'),
    assignees: z.array(z.object({ user: person, role })).optional(),
    roleEfforts: effortsInput.optional(),
    tags: z.array(tag).optional(),
    customFields: z.record(z.string(), z.unknown()).optional(),
    release: refOrNull.optional(),
    iteration: refOrNull.optional(),
    teamIteration: refOrNull.optional(),
    fields: z.record(z.string(), z.unknown()).optional().describe('Any other settable fields as Targetprocess JSON, validated against the catalog'),
  },
  handler: async (args, ctx) => {
    const catalog = await ctx.catalog()
    const lookup = catalog.resource(args.type)
    if (!lookup.ok) return invalid(lookup.message)
    const resource = lookup.value
    if (!isCardResource(resource) || isAdminResource(resource)) return invalid(`${resource.name} is not a card type pounce creates here; use write_create`)

    // ---- resolve everything before posting
    const body: Record<string, unknown> = { Name: args.name }
    let parentRef: { id: number; resourceType: string } | undefined
    let parentField: string | undefined
    let parentProject: { id: number; name: string } | undefined
    if (args.parent !== undefined) {
      const p = await cardInfo(ctx, args.parent)
      if (!p.ok) return unresolved(p)
      const link = parentLink(resource, p.value)
      if (!link.ok) return invalid(link.message)
      Object.assign(body, link.body)
      parentProject = p.value.project
      parentRef = { id: p.value.id, resourceType: p.value.entityType }
      parentField = link.field
    }

    let projectId: number | undefined
    let processId: number | undefined
    if (args.project !== undefined) {
      const project = await ctx.directory.project(args.project)
      if (!project.ok) return unresolved(project)
      projectId = project.value.Id
      processId = project.value.Process?.Id
    } else {
      projectId = parentProject?.id ?? ctx.config.defaultProjectId
    }
    if (projectId === undefined) {
      return invalid('no project could be resolved: pass project, or a parent card to inherit it from (TP_DEFAULT_PROJECT_ID is not set)')
    }
    if (processId === undefined) {
      const project = await ctx.directory.project(projectId)
      if (!project.ok) return unresolved(project)
      processId = project.value.Process?.Id
    }
    body.Project = { Id: projectId }

    if (args.description !== undefined) body.Description = textToHtml(args.description)
    if (args.state !== undefined) {
      if (processId === undefined) return invalid('the project has no process, so states cannot be resolved')
      const state = await ctx.directory.state(processId, resource.name, args.state)
      if (!state.ok) return unresolved(state)
      body.EntityState = { Id: state.value.Id }
    }

    const teamIds: { Id: number; Name?: string }[] = []
    for (const t of args.teams ?? []) {
      const team = await ctx.directory.team(t)
      if (!team.ok) return unresolved(team)
      teamIds.push({ Id: team.value.Id, Name: team.value.Name })
    }
    if (!args.teams && ctx.config.defaultTeamId !== undefined && catalog.member(resource, 'AssignedTeams')) teamIds.push({ Id: ctx.config.defaultTeamId })
    if (teamIds.length) {
      if (!catalog.member(resource, 'AssignedTeams')) return invalid(`A ${resource.name} cannot have teams`)
      body.AssignedTeams = { Items: teamIds.map((t) => ({ Team: { Id: t.Id } })) }
    }

    const assignees: { user: TpUser; role: TpRole }[] = []
    for (const a of args.assignees ?? []) {
      const user = await resolvePerson(ctx, a.user)
      if (!user.ok) return unresolved(user)
      const r = await ctx.directory.role(a.role)
      if (!r.ok) return unresolved(r)
      if (!assignees.some((x) => x.user.Id === user.value.Id && x.role.Id === r.value.Id)) assignees.push({ user: user.value, role: r.value })
    }
    if (assignees.length && !catalog.member(resource, 'Assignments')) return invalid(`A ${resource.name} cannot have assignments`)

    let efforts: EffortRequest[] = []
    if (args.roleEfforts?.length) {
      if (!catalog.member(resource, 'RoleEfforts')) return invalid(`A ${resource.name} has no role efforts`)
      const r = await resolveEfforts(ctx, args.roleEfforts)
      if (!r.ok) return r.output
      efforts = r.value
    }

    let customFields: CustomFieldValue[] = []
    if (args.customFields && Object.keys(args.customFields).length) {
      if (processId === undefined) return invalid('the project has no process, so custom fields cannot be resolved')
      const cf = await prepareCustomFields(ctx, processId, resource.name, args.customFields)
      if (!cf.ok) return unresolved(cf)
      customFields = cf.value
      body.CustomFields = customFields
    }
    if (args.tags?.length) body.Tags = [...new Set(args.tags.map((t) => t.trim()))].join(',')
    for (const [key, value] of [['Release', args.release], ['Iteration', args.iteration], ['TeamIteration', args.teamIteration]] as const) {
      if (value !== undefined) body[key] = value === null ? null : { Id: value }
    }
    let extraFields: Record<string, unknown> = {}
    if (args.fields && Object.keys(args.fields).length) {
      const reserved = reservedFields(args.fields)
      if (reserved) return invalid(reserved)
      const extra = prepareBody(catalog, resource, args.fields, 'create')
      if (!extra.ok) return invalid(extra.message)
      extraFields = extra.value
      Object.assign(body, extra.value)
    }

    // ---- write, sequentially
    const parentBefore = parentRef ? await snapshotOf(ctx, parentRef) : undefined
    const created = await ctx.v1.create<Raw>(resource.path, body, { resultInclude: '[Id,Name]' })
    if (!created.ok) return failure(`Could not create the ${resource.name}`, created)
    const newId = num(created.data?.Id)
    if (newId === undefined) return failure(`Created a ${resource.name} but the response carried no Id`, undefined, { response: created.data })
    const done: unknown[] = [{ created: { id: newId, type: resource.name } }]

    const removedDefaults: (ReturnType<typeof describeAssignment> & { late?: boolean })[] = []
    if (catalog.member(resource, 'Assignments')) {
      const defaults = await listAssignments(ctx, newId)
      if (!defaults.ok) return partial(`Created ${resource.name} ${newId} but could not read its default assignments`, defaults, done)
      for (const a of defaults.data) {
        if (assignees.some((x) => x.user.Id === a.user.id && x.role.Id === a.role.id)) continue
        const del = await removeAssignment(ctx, a.id)
        if (!del.ok) return partial(`Created ${resource.name} ${newId} but could not remove default assignment ${a.user.name} as ${a.role.name}`, del, done)
        removedDefaults.push(describeAssignment(a))
        done.push({ removedDefault: describeAssignment(a) })
      }
      const kept = defaults.data.filter((a) => assignees.some((x) => x.user.Id === a.user.id && x.role.Id === a.role.id))
      for (const a of assignees) {
        if (kept.some((k) => k.user.id === a.user.Id && k.role.id === a.role.Id)) continue
        const add = await addAssignment(ctx, newId, a.user.Id, a.role.Id)
        if (!add.ok) return partial(`Created ${resource.name} ${newId} but could not assign ${fullName(a.user)} as ${a.role.Name}`, add, done)
        done.push({ assigned: `${fullName(a.user)} as ${a.role.Name}` })
      }
    }

    const info = { id: newId, name: args.name, entityType: resource.name, resource, project: { id: projectId, name: '' }, ...(processId !== undefined ? { processId } : {}) }
    if (efforts.length) {
      const fresh = await readCardAs(ctx, info)
      if (!fresh.ok) return partial(`Created ${resource.name} ${newId} but could not read it to set role efforts`, fresh, done)
      const outcome = await applyRoleEfforts(ctx, newId, roleEffortRows(fresh.data), efforts)
      if (outcome.error) return partial(`Created ${resource.name} ${newId}; ${outcome.error.message}`, outcome.error.error, [...done, ...outcome.applied])
      done.push(...outcome.applied.map((e) => ({ effort: e })))
    }

    // Targetprocess can add default assignments after the first check: look again and remove them.
    if (catalog.member(resource, 'Assignments')) {
      for (const wait of ctx.settleDelaysMs) {
        await new Promise((resolve) => setTimeout(resolve, wait))
        const now = await listAssignments(ctx, newId)
        if (!now.ok) break
        for (const a of now.data) {
          if (assignees.some((x) => x.user.Id === a.user.id && x.role.Id === a.role.id)) continue
          const del = await removeAssignment(ctx, a.id)
          if (!del.ok) return partial(`Created ${resource.name} ${newId} but could not remove late default assignment ${a.user.name} as ${a.role.name}`, del, done)
          removedDefaults.push({ ...describeAssignment(a), late: true })
          done.push({ removedDefault: describeAssignment(a), late: true })
        }
      }
    }

    // ---- read back and verify
    const final = await readCardAs(ctx, info)
    if (!final.ok) {
      // Every write succeeded; only the check failed. Not an error: a retry would create a duplicate.
      const parentAfter = await snapshotAgain(ctx, parentBefore)
      return success({
        created: { id: newId, type: resource.name, name: args.name },
        done,
        removedDefaultAssignments: removedDefaults,
        ...(parentBefore ? { parent: parentChange(parentBefore, parentAfter) } : {}),
        ...unverified(final),
      })
    }
    const raw = final.data
    const notPersisted: string[] = []
    if (ref(raw.Project)?.id !== projectId) notPersisted.push(`project: requested ${projectId}, got ${ref(raw.Project)?.id}`)
    const wantedState = (body.EntityState as { Id: number } | undefined)?.Id
    if (wantedState !== undefined && ref(raw.EntityState)?.id !== wantedState) notPersisted.push(`state: got ${ref(raw.EntityState)?.name}`)
    if (catalog.member(resource, 'Assignments')) {
      const got = items(raw.Assignments).map((a) => `${ref(a.GeneralUser)?.id}:${ref(a.Role)?.id}`).sort()
      const want = assignees.map((a) => `${a.user.Id}:${a.role.Id}`).sort()
      if (got.join() !== want.join()) {
        notPersisted.push(`assignments: expected exactly ${want.length}, card has ${items(raw.Assignments).map((a) => `${ref(a.GeneralUser)?.id} as ${ref(a.Role)?.name}`).join(', ') || 'none'}`)
      }
    }
    const rows = roleEffortRows(raw)
    for (const e of efforts) {
      const got = rows.find((r) => r.role.id === e.roleId)?.effort
      if (got !== e.effort) notPersisted.push(`${e.roleName} effort: requested ${e.effort}, got ${got ?? 'no row'}`)
    }
    const teamIdsAfter = items(raw.AssignedTeams).map((t) => ref(t.Team)?.id)
    for (const t of teamIds) if (!teamIdsAfter.includes(t.Id)) notPersisted.push(`team ${t.Name ?? t.Id} is not on the card`)
    notPersisted.push(...unpersistedCustomFields(customFields, raw.CustomFields))
    const common = await verifyWritten(ctx, info, body, raw, { parentField, parentId: parentRef?.id, extraFields })
    notPersisted.push(...common.notPersisted)

    const parentAfter = await snapshotAgain(ctx, parentBefore)
    const shapedInfo = { ...info, project: ref(raw.Project) ? { id: ref(raw.Project)!.id, name: ref(raw.Project)!.name ?? '' } : info.project }
    return success({
      created: { id: newId, type: resource.name, name: args.name },
      card: shapeCard(shapedInfo, raw),
      removedDefaultAssignments: removedDefaults,
      ...(efforts.length ? { totalsNote: TOTALS_NOTE } : {}),
      ...(parentAfter || parentBefore ? { parent: parentChange(parentBefore, parentAfter) } : {}),
      ...verdict({ notPersisted, notVerified: common.notVerified }),
    })
  },
})

// ---------------------------------------------------------------- update

export const writeUpdateCard = defineTool({
  name: 'write_update_card',
  description:
    'Update a card: name, description, tags (tags replaces the whole list; addTags/removeTags edit it), release, iteration, team iteration ' +
    '(null clears), parent, or other settable fields. Changes are read back; removed tags are reported. For a test case, parent ADDS it to ' +
    'that test plan (existing plans are kept). For state, people, effort, teams and custom fields use the dedicated write_* tools.',
  input: {
    id,
    name: z.string().min(1).optional(),
    description: z.string().optional(),
    tags: z.array(tag).optional(),
    addTags: z.array(tag).optional(),
    removeTags: z.array(z.string().min(1)).optional(),
    release: refOrNull.optional(),
    iteration: refOrNull.optional(),
    teamIteration: refOrNull.optional(),
    parent: id.optional(),
    fields: z.record(z.string(), z.unknown()).optional(),
  },
  handler: async (args, ctx) => {
    const loaded = await load(ctx, args.id)
    if (!loaded.ok) return loaded.output
    const card = loaded.card
    const catalog = await ctx.catalog()
    const body: Record<string, unknown> = {}
    if (args.name !== undefined) body.Name = args.name
    if (args.description !== undefined) body.Description = textToHtml(args.description)

    const currentTags = parseTags(card.raw.Tags) ?? []
    let removedTags: string[] = []
    if (args.tags !== undefined || args.addTags?.length || args.removeTags?.length) {
      if (args.tags !== undefined && (args.addTags?.length || args.removeTags?.length)) return invalid('use either tags or addTags/removeTags')
      const lower = (s: string) => s.trim().toLowerCase()
      let next = args.tags !== undefined ? args.tags.map((t) => t.trim()).filter(Boolean) : [...currentTags]
      for (const t of args.addTags ?? []) if (!next.some((n) => lower(n) === lower(t))) next.push(t.trim())
      if (args.removeTags?.length) next = next.filter((n) => !args.removeTags!.some((r) => lower(r) === lower(n)))
      next = [...new Map(next.map((t) => [lower(t), t])).values()]
      removedTags = currentTags.filter((t) => !next.some((n) => lower(n) === lower(t)))
      body.Tags = next.join(',')
    }
    for (const [key, value] of [['Release', args.release], ['Iteration', args.iteration], ['TeamIteration', args.teamIteration]] as const) {
      if (value !== undefined) {
        if (!catalog.member(card.info.resource, key)) return invalid(`A ${card.info.entityType} has no ${key}`)
        body[key] = value === null ? null : { Id: value }
      }
    }
    let parentField: string | undefined
    if (args.parent !== undefined) {
      const p = await cardInfo(ctx, args.parent)
      if (!p.ok) return unresolved(p)
      const link = parentLink(card.info.resource, p.value)
      if (!link.ok) return invalid(link.message)
      Object.assign(body, link.body)
      parentField = link.field
    }
    let extraFields: Record<string, unknown> = {}
    if (args.fields && Object.keys(args.fields).length) {
      const reserved = reservedFields(args.fields)
      if (reserved) return invalid(`${reserved}\n(write_update is the escape hatch for an explicit override)`)
      const extra = prepareBody(catalog, card.info.resource, args.fields, 'update')
      if (!extra.ok) return invalid(extra.message)
      extraFields = extra.value
      Object.assign(body, extra.value)
    }
    if (Object.keys(body).length === 0) return invalid('nothing to update')

    const r = await ctx.v1.update(card.info.resource.path, card.info.id, body)
    if (!r.ok) return failure(`Could not update ${card.info.entityType} ${card.info.id}`, r)
    const reread = await reload(ctx, card)
    if (!reread.ok) return success({ card: cardLabel(card), updated: Object.keys(body), ...(removedTags.length ? { removedTags } : {}), ...unverified(reread) })
    const after = reread.data
    const checked = await verifyWritten(ctx, card.info, body, after, { parentField, parentId: args.parent, extraFields })
    return success({
      card: cardLabel(card),
      updated: Object.keys(body),
      ...(removedTags.length ? { removedTags } : {}),
      now: shapeCard(card.info, after),
      ...verdict(checked),
    })
  },
})

// ---------------------------------------------------------------- comments, time, relations, follow

export const writeComment = defineTool({
  name: 'write_comment',
  description: 'Add a comment to a card; plain text lines become paragraphs, HTML is sent as-is. replyTo answers another comment.',
  input: { id, text: z.string().min(1), replyTo: id.optional(), private: z.boolean().optional() },
  handler: async (args, ctx) => {
    const body: Record<string, unknown> = { General: { Id: args.id }, Description: textToHtml(args.text) }
    if (args.replyTo !== undefined) body.ParentId = args.replyTo
    if (args.private !== undefined) body.IsPrivate = args.private
    const r = await ctx.v1.create<Raw>('Comments', body, { resultInclude: '[Id,Description,General[Id,Name],ParentId,CreateDate]' })
    if (!r.ok) return failure(`Could not comment on ${args.id}`, r)
    const persisted = htmlToText(String(r.data?.Description ?? '')) === htmlToText(String(body.Description))
    return success({
      comment: num(r.data?.Id),
      card: ref(r.data?.General) ?? { id: args.id },
      ...(persisted ? {} : { notPersisted: ['the stored comment text differs from what was sent'] }),
    })
  },
})

export const writeLogTime = defineTool({
  name: 'write_log_time',
  description:
    'Log time spent on a card for a person (default: you). The role defaults to the person\'s only role on the card; if they have none or ' +
    'several, pass role. date is YYYY-MM-DD (default: today in the server\'s time zone).',
  input: {
    id,
    spent: z.number().positive().max(1000),
    remain: z.number().min(0).max(100_000).optional().describe('Remaining time to record on the card'),
    date: z.string().regex(/^\d{4}-\d{2}-\d{2}$/).optional(),
    description: z.string().optional(),
    role: role.optional(),
    user: person.optional(),
  },
  handler: async (args, ctx) => {
    const loaded = await load(ctx, args.id)
    if (!loaded.ok) return loaded.output
    const card = loaded.card
    if (!card.info.project) return invalid(`${card.info.entityType} ${card.info.id} has no project`)
    const user = await resolvePerson(ctx, args.user ?? 'me')
    if (!user.ok) return unresolved(user)
    let roleId: number
    let roleName: string | undefined
    if (args.role !== undefined) {
      const r = await ctx.directory.role(args.role)
      if (!r.ok) return unresolved(r)
      roleId = r.value.Id
      roleName = r.value.Name
    } else {
      const mine = items(card.raw.Assignments).filter((a) => ref(a.GeneralUser)?.id === user.value.Id)
      if (mine.length !== 1) {
        return invalid(
          `${fullName(user.value)} has ${mine.length ? `${mine.length} roles (${mine.map((a) => ref(a.Role)?.name).join(', ')})` : 'no role'} on ${card.info.entityType} ${card.info.id}; pass role`,
        )
      }
      const r = ref(mine[0]!.Role)!
      roleId = r.id
      roleName = r.name
    }
    const body: Record<string, unknown> = {
      Assignable: { Id: card.info.id },
      Project: { Id: card.info.project.id },
      User: { Id: user.value.Id },
      Role: { Id: roleId },
      Spent: args.spent,
      Date: args.date ?? localDate(),
    }
    if (args.remain !== undefined) body.Remain = args.remain
    if (args.description) body.Description = args.description
    const r = await ctx.v1.create<Raw>('Times', body, { resultInclude: '[Id,Spent,Remain,Date]' })
    if (!r.ok) return failure(`Could not log time on ${card.info.entityType} ${card.info.id}`, r)
    const notPersisted = num(r.data?.Spent) === args.spent ? [] : [`spent: requested ${args.spent}, got ${String(r.data?.Spent)}`]
    const after = await reload(ctx, card)
    return success({
      time: num(r.data?.Id),
      card: cardLabel(card),
      user: `${fullName(user.value)} (${user.value.Id})`,
      role: roleName,
      spent: num(r.data?.Spent),
      date: r.data?.Date,
      cardTime: after.ok ? { spent: num(after.data.TimeSpent), remaining: num(after.data.TimeRemain) } : undefined,
      ...(notPersisted.length ? { notPersisted } : {}),
    })
  },
})

export const writeRelate = defineTool({
  name: 'write_relate',
  description:
    'Relate two cards: relation Dependency, Blocker, Relation (default), Link or Duplicate (or any type on the instance). outbound (default): ' +
    'id → to (id is the master); inbound: to → id. An existing identical relation is left alone.',
  input: {
    id,
    to: id,
    relation: nameOrId.optional(),
    direction: z.enum(['outbound', 'inbound']).optional(),
  },
  handler: async (args, ctx) => {
    if (args.id === args.to) return invalid('a card cannot relate to itself')
    const types = await ctx.v1.list<Raw>('RelationTypes', { include: '[Id,Name]' })
    if (!types.ok) return failure('Could not load relation types', types)
    const type = match(args.relation ?? 'Relation', {
      kind: 'relation type',
      items: types.data.items,
      id: (t) => num(t.Id) as number,
      name: (t) => String(t.Name),
      keys: (t) => [String(t.Name)],
    })
    if (!type.ok) return unresolved(type)
    const [master, slave] = args.direction === 'inbound' ? [args.to, args.id] : [args.id, args.to]
    const existing = await ctx.v1.list<Raw>('Relations', { where: `(Master.Id eq ${master}) and (Slave.Id eq ${slave})`, include: '[Id,RelationType[Id,Name]]' })
    if (!existing.ok) return failure('Could not read existing relations', existing)
    const same = existing.data.items.find((r) => ref(r.RelationType)?.id === num(type.value.Id))
    if (same) return success({ relation: num(same.Id), master, slave, type: type.value.Name, alreadyRelated: true })
    const r = await ctx.v1.create<Raw>('Relations', { Master: { Id: master }, Slave: { Id: slave }, RelationType: { Id: num(type.value.Id) } }, { resultInclude: '[Id,Master[Id,Name],Slave[Id,Name],RelationType[Id,Name]]' })
    if (!r.ok) return failure(`Could not relate ${master} → ${slave}`, r)
    return success({ relation: num(r.data?.Id), master: ref(r.data?.Master) ?? master, slave: ref(r.data?.Slave) ?? slave, type: ref(r.data?.RelationType)?.name ?? type.value.Name })
  },
})

export const writeFollow = defineTool({
  name: 'write_follow',
  description: 'Follow (or with unfollow: true, stop following) a card, for yourself or another person.',
  input: { id, user: person.optional(), unfollow: z.boolean().optional() },
  handler: async (args, ctx) => {
    const user = await resolvePerson(ctx, args.user ?? 'me')
    if (!user.ok) return unresolved(user)
    const existing = await ctx.v1.list<Raw>('GeneralFollowers', { where: `(General.Id eq ${args.id}) and (User.Id eq ${user.value.Id})`, include: '[Id]' })
    if (!existing.ok) return failure(`Could not read followers of ${args.id}`, existing)
    const rows = existing.data.items
    const who = `${fullName(user.value)} (${user.value.Id})`
    const want = !args.unfollow
    if (want === rows.length > 0) return success({ card: args.id, user: who, following: want, changed: false })
    if (want) {
      const r = await ctx.v1.create<Raw>('GeneralFollowers', { General: { Id: args.id }, User: { Id: user.value.Id } })
      if (!r.ok) return failure(`Could not follow ${args.id}`, r)
    } else {
      for (const row of rows) {
        const r = await ctx.v1.delete('GeneralFollowers', num(row.Id) as number)
        if (!r.ok) return failure(`Could not unfollow ${args.id}`, r)
      }
    }
    const after = await ctx.v1.list<Raw>('GeneralFollowers', { where: `(General.Id eq ${args.id}) and (User.Id eq ${user.value.Id})`, include: '[Id]' })
    if (!after.ok) return success({ card: args.id, user: who, following: want, changed: true, warning: `Could not read followers back (${after.message})` })
    const following = after.data.items.length > 0
    return success({ card: args.id, user: who, following, changed: true, ...(following !== want ? { notPersisted: [`following: requested ${want}, got ${following}`] } : {}) })
  },
})

// ---------------------------------------------------------------- testing

export const writeTestCases = defineTool({
  name: 'write_test_cases',
  description: 'Create test cases with ordered steps under a test plan. The project is inherited from the test plan. Each case is read back.',
  input: {
    testPlan: id,
    cases: z
      .array(
        z.object({
          name: z.string().min(1),
          description: z.string().optional(),
          steps: z.array(z.object({ step: z.string().min(1), expected: z.string().optional() })).optional(),
        }),
      )
      .min(1)
      .max(100),
  },
  handler: async (args, ctx) => {
    const plan = await cardInfo(ctx, args.testPlan)
    if (!plan.ok) return unresolved(plan)
    if (plan.value.entityType !== 'TestPlan') return invalid(`${args.testPlan} is a ${plan.value.entityType}, not a TestPlan`)
    if (!plan.value.project) return invalid(`Test plan ${args.testPlan} has no project`)
    const created: unknown[] = []
    for (const c of args.cases) {
      const body: Record<string, unknown> = {
        Name: c.name,
        Project: { Id: plan.value.project.id },
        TestPlans: { Items: [{ Id: args.testPlan }] },
      }
      if (c.description) body.Description = textToHtml(c.description)
      if (c.steps?.length) {
        body.TestSteps = {
          Items: c.steps.map((s, i) => ({ Description: textToHtml(s.step), ...(s.expected ? { Result: textToHtml(s.expected) } : {}), RunOrder: i + 1 })),
        }
      }
      const r = await ctx.v1.create<Raw>('TestCases', body, { resultInclude: '[Id,Name]' })
      if (!r.ok) return partial(`Could not create test case "${c.name}"`, r, created)
      const caseId = num(r.data?.Id) as number
      const back = await ctx.v1.get<Raw>('TestCases', caseId, { include: '[Id,Name,TestPlans[Id],TestSteps[Id]]', innerTake: 1000 })
      const notPersisted: string[] = []
      if (back.ok) {
        if (!items(back.data.TestPlans).some((p) => num(p.Id) === args.testPlan)) notPersisted.push('not linked to the test plan')
        if (items(back.data.TestSteps).length !== (c.steps?.length ?? 0)) notPersisted.push(`steps: sent ${c.steps?.length ?? 0}, found ${items(back.data.TestSteps).length}`)
      }
      created.push({
        id: caseId,
        name: c.name,
        steps: c.steps?.length ?? 0,
        ...(notPersisted.length ? { notPersisted } : {}),
        ...(!back.ok ? { warning: `created, but could not be read back to verify (${back.message})` } : {}),
      })
    }
    return success({ testPlan: { id: args.testPlan, name: plan.value.name }, created })
  },
})

const RUN_STATUSES = ['Passed', 'Failed', 'Blocked', 'OnHold', 'NotRun'] as const

export const writeTestRun = defineTool({
  name: 'write_test_run',
  description:
    'Record a test run of a test plan: creates a test plan run (Targetprocess adds a "Not run" test case run per case), then sets each given ' +
    `test case's result (${RUN_STATUSES.join(', ')}) with an optional comment.`,
  input: {
    testPlan: id,
    name: z.string().optional(),
    results: z.array(z.object({ testCase: id, status: z.enum(RUN_STATUSES), comment: z.string().optional() })).optional(),
  },
  handler: async (args, ctx) => {
    const plan = await cardInfo(ctx, args.testPlan)
    if (!plan.ok) return unresolved(plan)
    if (plan.value.entityType !== 'TestPlan') return invalid(`${args.testPlan} is a ${plan.value.entityType}, not a TestPlan`)
    if (args.results?.length) {
      const cases = await ctx.v1.list<Raw>(`TestPlans/${args.testPlan}/TestCases`, { include: '[Id]', limit: 20_000 })
      if (!cases.ok) return failure(`Could not read the test cases of test plan ${args.testPlan}`, cases)
      const inPlan = new Set(cases.data.items.map((c) => num(c.Id)))
      const foreign = args.results.filter((r) => !inPlan.has(r.testCase)).map((r) => r.testCase)
      if (foreign.length) return invalid(`test cases ${foreign.join(', ')} are not in test plan ${args.testPlan}; no run was started`)
    }
    const body: Record<string, unknown> = { TestPlan: { Id: args.testPlan } }
    if (args.name) body.Name = args.name
    if (plan.value.project) body.Project = { Id: plan.value.project.id }
    const run = await ctx.v1.create<Raw>('TestPlanRuns', body, { resultInclude: '[Id,Name]' })
    if (!run.ok) return failure(`Could not start a run of test plan ${args.testPlan}`, run)
    const runId = num(run.data?.Id) as number
    const done: unknown[] = [{ testPlanRun: runId }]
    // Read the generated case runs fully paged: an inner collection in the response would stop at 25.
    const list = await ctx.v1.list<Raw>('TestCaseRuns', { where: `(TestPlanRun.Id eq ${runId})`, include: '[Id,TestCase[Id,Name]]', limit: 20_000 })
    if (!list.ok) return partial(`Started run ${runId} but could not read its test case runs`, list, done)
    const caseRuns = list.data.items
    const recorded: unknown[] = []
    const missing: number[] = []
    for (const res of args.results ?? []) {
      const cr = caseRuns.find((c) => ref(c.TestCase)?.id === res.testCase)
      if (!cr) {
        missing.push(res.testCase)
        continue
      }
      const update: Record<string, unknown> = { Status: res.status }
      if (res.comment) update.Comment = res.comment
      const r = await ctx.v1.update<Raw>('TestCaseRuns', num(cr.Id) as number, update, { resultInclude: '[Id,Status,Comment]' })
      if (!r.ok) return partial(`Could not record ${res.status} for test case ${res.testCase}`, r, [...done, ...recorded])
      recorded.push({ testCase: res.testCase, status: r.data?.Status ?? res.status, ...(r.data?.Status && r.data.Status !== res.status ? { notPersisted: `status: got ${String(r.data.Status)}` } : {}) })
    }
    return success({
      testPlanRun: { id: runId, name: run.data?.Name },
      caseRuns: caseRuns.length,
      recorded,
      ...(missing.length ? { notInRun: missing } : {}),
    })
  },
})

export const workflowWriteTools = [
  writeCreateCard,
  writeUpdateCard,
  writeSetState,
  writeAssign,
  writeUnassign,
  writeSetRoleEffort,
  writeSetCustomFields,
  writeTeam,
  writeComment,
  writeLogTime,
  writeRelate,
  writeFollow,
  writeTestCases,
  writeTestRun,
]
