import { z } from 'zod'
import { parentChain, readCardRaw, shapeCard } from '../../domain/cards.js'
import { cardSummary, compact, items, num, person, ref, text } from '../../format/shape.js'
import { v1String } from '../../http/v1.js'
import { customFieldOptions, fullName, isActiveUser, isTeamWorkflowState } from '../../resolve/directory.js'
import { match } from '../../resolve/match.js'
import type { ToolContext } from '../context.js'
import { failure, invalid, success } from '../respond.js'
import { id } from '../schema.js'
import { defineTool, type ToolOutput } from '../types.js'
import { typeScope, unresolved } from './common.js'

type Raw = Record<string, unknown>

const nameOrId = z.union([z.string().min(1), z.number().int().positive()])
const listLimit = (d: number, max: number) => z.number().int().min(1).max(max).optional().describe(`Maximum results (default ${d}, max ${max}); capped results say truncated: true`)

export const readCard = defineTool({
  name: 'read_card',
  description:
    'Read any card (user story, task, bug, feature, epic, test plan, ...) by id: type, name, state, project, parent chain, teams with team states, ' +
    'assignments (who is assigned in which role), role efforts, effort totals, release/iteration, custom fields, tags, counts, and the description as plain text.',
  input: { id },
  handler: async ({ id: cardId }, ctx) => {
    const card = await readCardRaw(ctx, cardId)
    if (!card.ok) return card.error ? failure(card.message, card.error) : invalid(card.message)
    const parents = await parentChain(ctx, card.value.raw)
    return success(shapeCard(card.value.info, card.value.raw, parents))
  },
})

interface SearchArgs {
  text?: string | undefined
  type?: string | undefined
  state?: string | undefined
  project?: string | number | undefined
  assignee?: string | number | undefined
  tag?: string | undefined
  includeClosed?: boolean | undefined
  limit?: number | undefined
}

async function searchCards(ctx: ToolContext, args: SearchArgs, extra: Record<string, unknown> = {}): Promise<ToolOutput> {
  const catalog = await ctx.catalog()
  const lookup = catalog.resource(args.type ?? 'Assignable')
  if (!lookup.ok) return invalid(lookup.message)
  const r = lookup.value
  const has = (m: string) => catalog.member(r, m) !== undefined
  const where: string[] = []

  const t = args.text?.trim()
  if (t) where.push(/^#?\d+$/.test(t) ? `(Id eq ${t.replace('#', '')})` : `(Name contains ${v1String(t)})`)
  if (args.state) {
    if (!has('EntityState')) return invalid(`${r.name} has no state`)
    where.push(`(EntityState.Name eq ${v1String(args.state)})`)
  }
  if (args.project !== undefined) {
    const project = await ctx.directory.project(args.project)
    if (!project.ok) return unresolved(project)
    where.push(`(Project.Id eq ${project.value.Id})`)
  }
  if (args.assignee !== undefined) {
    if (!has('AssignedUser')) return invalid(`${r.name} cannot be filtered by assignee`)
    const user = String(args.assignee).toLowerCase() === 'me' ? await ctx.directory.me() : await ctx.directory.user(args.assignee)
    if (!user.ok) return unresolved(user)
    where.push(`(AssignedUser.Id eq ${user.value.Id})`)
  }
  if (args.tag) {
    if (!has('TagObjects')) return invalid(`${r.name} has no tags`)
    where.push(`(TagObjects.Name eq ${v1String(args.tag)})`)
  }
  if (!args.includeClosed && has('EntityState') && !args.state) where.push(`(EntityState.IsFinal eq 'false')`)

  const include = ['Id', 'Name', 'EntityType[Name]', 'EntityState[Name]', 'Project[Name]', 'Tags', 'ModifyDate'].filter((f) => has(f.split('[')[0] as string))
  const result = await ctx.v1.list<Raw>(r.path, {
    ...(where.length ? { where: where.join(' and ') } : {}),
    include: `[${include.join(',')}]`,
    ...(has('ModifyDate') ? { orderByDesc: 'ModifyDate' } : {}),
    limit: args.limit ?? 50,
  })
  if (!result.ok) return failure('Search failed', result)
  const cards = result.data.items.map(cardSummary)
  const byState: Record<string, number> = {}
  for (const c of cards) if (typeof c.state === 'string') byState[c.state] = (byState[c.state] ?? 0) + 1
  return success({ ...extra, count: cards.length, truncated: result.data.truncated, byState, cards })
}

export const readSearch = defineTool({
  name: 'read_search',
  description:
    'Find cards by text in the name (or #id), type, state, project, assignee ("me" for yourself) and tag. Open cards only unless includeClosed. ' +
    'Names are resolved; ambiguous names are listed, never guessed. Newest-modified first.',
  input: {
    text: z.string().optional().describe('Text contained in the card name, or a #id'),
    type: z.string().optional().describe('Entity type, e.g. UserStory, Task, Bug, Feature (default: any assignable card)'),
    state: z.string().optional().describe('State name, e.g. "In Progress"'),
    project: nameOrId.optional().describe('Project name, abbreviation or id'),
    assignee: nameOrId.optional().describe('Person name, login, email, id, or "me"'),
    tag: z.string().optional(),
    includeClosed: z.boolean().optional(),
    limit: listLimit(50, 1000),
  },
  handler: (args, ctx) => searchCards(ctx, args),
})

export const readMyWork = defineTool({
  name: 'read_my_work',
  description: 'Cards assigned to the current user (the token owner), open ones by default, with counts per state.',
  input: {
    type: z.string().optional().describe('Entity type, e.g. Task or Bug (default: any assignable card)'),
    state: z.string().optional(),
    includeClosed: z.boolean().optional(),
    limit: listLimit(100, 1000),
  },
  handler: async (args, ctx) => {
    const me = await ctx.directory.me()
    if (!me.ok) return unresolved(me)
    return searchCards(ctx, { ...args, assignee: me.value.Id, limit: args.limit ?? 100 }, { user: { id: me.value.Id, name: fullName(me.value) } })
  },
})

export const readStates = defineTool({
  name: 'read_states',
  description:
    "States available to a card (from its own project's process), or to an entity type in a project. " +
    'Team sub-workflow states are flagged isTeamWorkflow; for a card, its current state and team states are included.',
  input: {
    id: id.optional().describe('Card id'),
    project: nameOrId.optional().describe('Project (when no card id)'),
    type: z.string().optional().describe('Entity type (when no card id), e.g. UserStory'),
  },
  handler: async (args, ctx) => {
    const scope = await typeScope(ctx, args)
    if (!scope.ok) return scope.output
    const s = scope.value
    const states = await ctx.directory.states(s.processId, s.entityType)
    if (!states.ok) return failure(`Could not load ${s.entityType} states`, states)
    let current: unknown
    let teamStates: unknown
    if (s.cardId !== undefined) {
      const card = await ctx.v1.get<Raw>(s.resource.path, s.cardId, {
        include: `[Id,EntityState[Id,Name]${catalogHas(await ctx.catalog(), s.resource.name, 'AssignedTeams') ? ',AssignedTeams[Id,Team[Id,Name],EntityState[Id,Name]]' : ''}]`,
      })
      if (card.ok) {
        current = ref(card.data.EntityState)
        const teams = items(card.data.AssignedTeams)
        if (teams.length) teamStates = teams.map((t) => ({ team: ref(t.Team), state: ref(t.EntityState) }))
      }
    }
    return success(
      compact({
        entityType: s.entityType,
        project: s.project,
        processId: s.processId,
        card: s.cardId,
        current,
        teamStates,
        states: states.data.map((st) =>
          compact({
            id: st.Id,
            name: st.Name,
            initial: st.IsInitial || undefined,
            final: st.IsFinal || undefined,
            planned: st.IsPlanned || undefined,
            role: st.Role?.Name,
            workflow: st.Workflow?.Name,
            isTeamWorkflow: isTeamWorkflowState(st) || undefined,
            parentState: st.ParentEntityState?.Name,
          }),
        ),
      }),
    )
  },
})

function catalogHas(catalog: Awaited<ReturnType<ToolContext['catalog']>>, resource: string, member: string): boolean {
  const r = catalog.find(resource)
  return Boolean(r && catalog.member(r, member))
}

export const readPeople = defineTool({
  name: 'read_people',
  description:
    'Find people by name, login or email (every word must match), fully paged over all users. Without a query, lists everyone active. ' +
    'Use the returned ids when a name is ambiguous.',
  input: {
    query: z.string().optional(),
    includeInactive: z.boolean().optional(),
    limit: listLimit(100, 5000),
  },
  handler: async (args, ctx) => {
    const users = await ctx.directory.users()
    if (!users.ok) return failure('Could not load users', users)
    let pool = args.includeInactive ? users.data : users.data.filter(isActiveUser)
    if (args.query?.trim()) {
      const words = args.query.trim().toLowerCase().split(/\s+/)
      pool = pool.filter((u) => {
        const hay = [fullName(u), u.Login, u.Email].filter(Boolean).join(' ').toLowerCase()
        return words.every((w) => hay.includes(w))
      })
    }
    const limit = args.limit ?? 100
    return success({
      count: Math.min(pool.length, limit),
      truncated: pool.length > limit,
      people: pool.slice(0, limit).map((u) =>
        compact({ id: u.Id, name: fullName(u), login: u.Login, email: u.Email, role: u.Role?.Name, inactive: isActiveUser(u) ? undefined : true }),
      ),
    })
  },
})

function filterByName<T extends { Name: string }>(list: T[], query?: string): T[] {
  const q = query?.trim().toLowerCase()
  return q ? list.filter((x) => x.Name.toLowerCase().includes(q)) : list
}

export const readTeams = defineTool({
  name: 'read_teams',
  description: 'List teams (active by default), optionally filtered by name.',
  input: { query: z.string().optional(), includeInactive: z.boolean().optional() },
  handler: async (args, ctx) => {
    const teams = await ctx.directory.teams()
    if (!teams.ok) return failure('Could not load teams', teams)
    const list = filterByName(args.includeInactive ? teams.data : teams.data.filter((t) => t.IsActive !== false), args.query)
    return success({ count: list.length, teams: list.map((t) => compact({ id: t.Id, name: t.Name, inactive: t.IsActive === false || undefined })) })
  },
})

export const readRoles = defineTool({
  name: 'read_roles',
  description: 'List roles (Developer, Product Owner, ...). hasEffort tells whether a role can carry effort.',
  input: {},
  handler: async (_args, ctx) => {
    const roles = await ctx.directory.roles()
    if (!roles.ok) return failure('Could not load roles', roles)
    return success({ count: roles.data.length, roles: roles.data.map((r) => ({ id: r.Id, name: r.Name, hasEffort: r.HasEffort !== false })) })
  },
})

export const readProjects = defineTool({
  name: 'read_projects',
  description: 'List projects (active by default) with abbreviation and process, optionally filtered by name.',
  input: { query: z.string().optional(), includeInactive: z.boolean().optional() },
  handler: async (args, ctx) => {
    const projects = await ctx.directory.projects()
    if (!projects.ok) return failure('Could not load projects', projects)
    const q = args.query?.trim().toLowerCase()
    const list = (args.includeInactive ? projects.data : projects.data.filter((p) => p.IsActive !== false)).filter(
      (p) => !q || p.Name.toLowerCase().includes(q) || (p.Abbreviation ?? '').toLowerCase() === q,
    )
    return success({
      count: list.length,
      projects: list.map((p) => compact({ id: p.Id, name: p.Name, abbreviation: p.Abbreviation, process: ref(p.Process), inactive: p.IsActive === false || undefined })),
    })
  },
})

function dated(raw: Raw) {
  return compact({
    id: num(raw.Id),
    name: raw.Name,
    start: raw.StartDate,
    end: raw.EndDate,
    current: raw.IsCurrent === true || undefined,
    project: ref(raw.Project),
    team: ref(raw.Team),
  })
}

export const readReleases = defineTool({
  name: 'read_releases',
  description: 'Releases, newest first, optionally for one project; currentOnly for releases in progress.',
  input: { project: nameOrId.optional(), currentOnly: z.boolean().optional(), limit: listLimit(50, 1000) },
  handler: async (args, ctx) => {
    const where: string[] = []
    if (args.project !== undefined) {
      const project = await ctx.directory.project(args.project)
      if (!project.ok) return unresolved(project)
      where.push(`(Project.Id eq ${project.value.Id})`)
    }
    if (args.currentOnly) where.push(`(IsCurrent eq 'true')`)
    const r = await ctx.v1.list<Raw>('Releases', {
      ...(where.length ? { where: where.join(' and ') } : {}),
      include: '[Id,Name,StartDate,EndDate,IsCurrent,Project[Id,Name]]',
      orderByDesc: 'StartDate',
      limit: args.limit ?? 50,
    })
    if (!r.ok) return failure('Could not load releases', r)
    return success({ count: r.data.items.length, truncated: r.data.truncated, releases: r.data.items.map(dated) })
  },
})

export const readIterations = defineTool({
  name: 'read_iterations',
  description:
    'Team iterations (sprints) for a team, or project iterations for a project; newest first. currentOnly for the running ones. ' +
    'Without team or project: current team iterations of every team.',
  input: { team: nameOrId.optional(), project: nameOrId.optional(), currentOnly: z.boolean().optional(), limit: listLimit(50, 1000) },
  handler: async (args, ctx) => {
    const where: string[] = []
    let path = 'TeamIterations'
    let include = '[Id,Name,StartDate,EndDate,IsCurrent,Team[Id,Name]]'
    if (args.project !== undefined && args.team === undefined) {
      const project = await ctx.directory.project(args.project)
      if (!project.ok) return unresolved(project)
      path = 'Iterations'
      include = '[Id,Name,StartDate,EndDate,IsCurrent,Project[Id,Name]]'
      where.push(`(Project.Id eq ${project.value.Id})`)
    } else if (args.team !== undefined) {
      const team = await ctx.directory.team(args.team)
      if (!team.ok) return unresolved(team)
      where.push(`(Team.Id eq ${team.value.Id})`)
    }
    if (args.currentOnly || (args.team === undefined && args.project === undefined)) where.push(`(IsCurrent eq 'true')`)
    const r = await ctx.v1.list<Raw>(path, { where: where.join(' and '), include, orderByDesc: 'StartDate', limit: args.limit ?? 50 })
    if (!r.ok) return failure(`Could not load ${path}`, r)
    return success({ kind: path, count: r.data.items.length, truncated: r.data.truncated, iterations: r.data.items.map(dated) })
  },
})

export const readCustomFieldOptions = defineTool({
  name: 'read_custom_field_options',
  description:
    "Custom fields of a card's type (or of an entity type in a project) with their type and, for dropdowns, the allowed values. " +
    'Pass field to see one field.',
  input: { id: id.optional(), project: nameOrId.optional(), type: z.string().optional(), field: z.string().optional() },
  handler: async (args, ctx) => {
    const scope = await typeScope(ctx, args)
    if (!scope.ok) return scope.output
    const s = scope.value
    const fields = await ctx.directory.customFields(s.processId, s.entityType)
    if (!fields.ok) return failure(`Could not load ${s.entityType} custom fields`, fields)
    let list = fields.data
    if (args.field) {
      const one = match(args.field, { kind: `${s.entityType} custom field`, items: list, id: (f) => f.Id, name: (f) => f.Name, keys: (f) => [f.Name] })
      if (!one.ok) return unresolved(one)
      list = [one.value]
    }
    return success({
      entityType: s.entityType,
      processId: s.processId,
      fields: list.map((f) => compact({ id: f.Id, name: f.Name, type: f.FieldType, required: f.Required || undefined, options: customFieldOptions(f) })),
    })
  },
})

export const readComments = defineTool({
  name: 'read_comments',
  description: 'Comments on a card, oldest first, as plain text, with author and reply parent.',
  input: { id, limit: listLimit(200, 5000) },
  handler: async (args, ctx) => {
    const r = await ctx.v1.list<Raw>('Comments', {
      where: `(General.Id eq ${args.id})`,
      include: '[Id,Description,CreateDate,Owner[Id,FirstName,LastName,Login],ParentId,IsPrivate,IsPinned]',
      orderBy: 'CreateDate',
      limit: args.limit ?? 200,
    })
    if (!r.ok) return failure(`Could not read comments of ${args.id}`, r)
    return success({
      card: args.id,
      count: r.data.items.length,
      truncated: r.data.truncated,
      comments: r.data.items.map((c) =>
        compact({ id: num(c.Id), author: person(c.Owner), date: c.CreateDate, replyTo: num(c.ParentId), private: c.IsPrivate === true || undefined, pinned: c.IsPinned === true || undefined, text: text(c.Description) ?? '' }),
      ),
    })
  },
})

function relationEnd(raw: unknown) {
  if (!raw || typeof raw !== 'object') return undefined
  const r = raw as Raw
  return compact({ id: num(r.Id), type: (r.EntityType as Raw | undefined)?.Name ?? r.ResourceType, name: r.Name })
}

export const readRelations = defineTool({
  name: 'read_relations',
  description: 'Relations of a card in both directions (dependency, blocker, relation, link, duplicate), with the related card.',
  input: { id },
  handler: async (args, ctx) => {
    const include = '[Id,RelationType[Id,Name],Master[Id,Name,EntityType[Name]],Slave[Id,Name,EntityType[Name]]]'
    const [out, inn] = await Promise.all([
      ctx.v1.list<Raw>('Relations', { where: `(Master.Id eq ${args.id})`, include }),
      ctx.v1.list<Raw>('Relations', { where: `(Slave.Id eq ${args.id})`, include }),
    ])
    if (!out.ok) return failure(`Could not read relations of ${args.id}`, out)
    if (!inn.ok) return failure(`Could not read relations of ${args.id}`, inn)
    const relations = [
      ...out.data.items.map((r) => ({ id: num(r.Id), relation: ref(r.RelationType)?.name, direction: 'outbound', card: relationEnd(r.Slave) })),
      ...inn.data.items.map((r) => ({ id: num(r.Id), relation: ref(r.RelationType)?.name, direction: 'inbound', card: relationEnd(r.Master) })),
    ]
    return success({ card: args.id, count: relations.length, truncated: out.data.truncated || inn.data.truncated, relations })
  },
})

export const readTimes = defineTool({
  name: 'read_times',
  description: 'Time logged on a card, or by a person ("me" for yourself), optionally within dates (YYYY-MM-DD). Returns entries and the total spent.',
  input: {
    id: id.optional().describe('Card id'),
    user: nameOrId.optional().describe('Person, or "me"'),
    from: z.string().regex(/^\d{4}-\d{2}-\d{2}$/).optional(),
    to: z.string().regex(/^\d{4}-\d{2}-\d{2}$/).optional(),
    limit: listLimit(500, 5000),
  },
  handler: async (args, ctx) => {
    const where: string[] = []
    if (args.id !== undefined) where.push(`(Assignable.Id eq ${args.id})`)
    if (args.user !== undefined) {
      const user = String(args.user).toLowerCase() === 'me' ? await ctx.directory.me() : await ctx.directory.user(args.user)
      if (!user.ok) return unresolved(user)
      where.push(`(User.Id eq ${user.value.Id})`)
    }
    if (!where.length) return invalid('pass a card id or a user')
    if (args.from) where.push(`(Date gte '${args.from}')`)
    if (args.to) where.push(`(Date lte '${args.to}')`)
    const r = await ctx.v1.list<Raw>('Times', {
      where: where.join(' and '),
      include: '[Id,Spent,Remain,Date,Description,User[Id,FirstName,LastName,Login],Role[Id,Name],Assignable[Id,Name]]',
      orderByDesc: 'Date',
      limit: args.limit ?? 500,
    })
    if (!r.ok) return failure('Could not read times', r)
    const entries = r.data.items.map((t) =>
      compact({ id: num(t.Id), date: t.Date, spent: num(t.Spent), remain: num(t.Remain), user: person(t.User), role: ref(t.Role), card: ref(t.Assignable), description: text(t.Description) }),
    )
    const total = entries.reduce((sum, e) => sum + (e.spent ?? 0), 0)
    return success({ count: entries.length, truncated: r.data.truncated, totalSpent: Math.round(total * 100) / 100, entries })
  },
})

export const readAttachments = defineTool({
  name: 'read_attachments',
  description:
    'Attachments of a card: name, size, type, owner, date and URL. Downloading needs Basic or cookie auth; with an access token Targetprocess returns an HTML error page instead of the file.',
  input: { id },
  handler: async (args, ctx) => {
    const r = await ctx.v1.list<Raw>('Attachments', {
      where: `(General.Id eq ${args.id})`,
      include: '[Id,Name,Description,Date,Owner[Id,FirstName,LastName,Login],MimeType,Size,Uri]',
    })
    if (!r.ok) return failure(`Could not read attachments of ${args.id}`, r)
    return success({
      card: args.id,
      count: r.data.items.length,
      truncated: r.data.truncated,
      attachments: r.data.items.map((a) =>
        compact({ id: num(a.Id), name: a.Name, description: a.Description, date: a.Date, owner: person(a.Owner), mimeType: a.MimeType ?? a.PersistedMimeType, size: num(a.Size) ?? num(a.PersistedSize), uri: a.Uri }),
      ),
    })
  },
})

export const readTestPlan = defineTool({
  name: 'read_test_plan',
  description: 'A test plan with its test cases and their steps (in run order), child test plans, and recent test plan runs.',
  input: { id, runs: z.number().int().min(0).max(100).optional().describe('How many recent runs to include (default 10)') },
  handler: async (args, ctx) => {
    const plan = await ctx.v1.get<Raw>('TestPlans', args.id, {
      include: '[Id,Name,Description,EntityState[Id,Name],Project[Id,Name],LinkedGeneral[Id,Name],ChildTestPlans[Id,Name]]',
      innerTake: 1000,
    })
    if (!plan.ok) return failure(`Could not read test plan ${args.id}`, plan)
    const cases = await ctx.v1.list<Raw>(`TestPlans/${args.id}/TestCases`, {
      include: '[Id,Name,Description,LastStatus,LastRunDate,LastFailureComment,TestSteps[Id,Description,Result,RunOrder]]',
      innerTake: 1000,
    })
    if (!cases.ok) return failure(`Could not read test cases of test plan ${args.id}`, cases)
    const runLimit = args.runs ?? 10
    let runs: unknown[] | undefined
    if (runLimit > 0) {
      const r = await ctx.v1.list<Raw>('TestPlanRuns', {
        where: `(TestPlan.Id eq ${args.id})`,
        include: '[Id,Name,EntityState[Id,Name],CreateDate,Build[Id,Name]]',
        orderByDesc: 'CreateDate',
        limit: runLimit,
      })
      if (!r.ok) return failure(`Could not read runs of test plan ${args.id}`, r)
      runs = r.data.items.map((x) => compact({ id: num(x.Id), name: x.Name, state: ref(x.EntityState)?.name, created: x.CreateDate, build: ref(x.Build) }))
    }
    return success(
      compact({
        id: args.id,
        name: plan.data.Name,
        state: ref(plan.data.EntityState),
        project: ref(plan.data.Project),
        linkedCard: ref(plan.data.LinkedGeneral),
        description: text(plan.data.Description),
        childPlans: items(plan.data.ChildTestPlans).map(ref),
        testCases: cases.data.items.map((c) =>
          compact({
            id: num(c.Id),
            name: c.Name,
            lastStatus: c.LastStatus,
            lastRun: c.LastRunDate,
            lastFailure: text(c.LastFailureComment),
            description: text(c.Description),
            steps: items(c.TestSteps)
              .sort((a, b) => (num(a.RunOrder) ?? 0) - (num(b.RunOrder) ?? 0))
              .map((s) => compact({ id: num(s.Id), order: num(s.RunOrder), step: text(s.Description) ?? '', expected: text(s.Result) })),
          }),
        ),
        truncated: cases.data.truncated || undefined,
        runs,
      }),
    )
  },
})

export const workflowReadTools = [
  readCard,
  readSearch,
  readMyWork,
  readStates,
  readPeople,
  readTeams,
  readRoles,
  readProjects,
  readReleases,
  readIterations,
  readCustomFieldOptions,
  readComments,
  readRelations,
  readTimes,
  readAttachments,
  readTestPlan,
]
