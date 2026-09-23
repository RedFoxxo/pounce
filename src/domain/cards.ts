import type { Catalog } from '../catalog/catalog.js'
import type { CatalogResource } from '../catalog/types.js'
import {
  assignment,
  compact,
  customFields,
  items,
  num,
  person,
  ref,
  roleEffort,
  tags,
  teamAssignment,
  text,
} from '../format/shape.js'
import type { Err, Result } from '../http/result.js'
import { cardInfo, type CardInfo } from '../resolve/card.js'
import type { ToolContext } from '../tools/context.js'

type Raw = Record<string, unknown>

/** Parent references, nearest first. The first one set on a card is its parent (LinkedGeneral: the card a test plan covers). */
export const PARENT_REFS = ['UserStory', 'Feature', 'Epic', 'PortfolioEpic', 'LinkedGeneral'] as const

/** What a card read asks for; members the resource lacks are skipped. */
const CARD_INCLUDE: [member: string, expr: string][] = [
  ['Name', 'Name'],
  ['Description', 'Description'],
  ['EntityType', 'EntityType[Id,Name]'],
  ['EntityState', 'EntityState[Id,Name,IsInitial,IsFinal]'],
  ['Project', 'Project[Id,Name,Process[Id,Name]]'],
  ['Owner', 'Owner[Id,FirstName,LastName,Login]'],
  ['CreateDate', 'CreateDate'],
  ['ModifyDate', 'ModifyDate'],
  ['Tags', 'Tags'],
  ['Effort', 'Effort'],
  ['EffortCompleted', 'EffortCompleted'],
  ['EffortToDo', 'EffortToDo'],
  ['TimeSpent', 'TimeSpent'],
  ['TimeRemain', 'TimeRemain'],
  ['Priority', 'Priority[Id,Name]'],
  ['Severity', 'Severity[Id,Name]'],
  ['Release', 'Release[Id,Name]'],
  ['Iteration', 'Iteration[Id,Name]'],
  ['TeamIteration', 'TeamIteration[Id,Name]'],
  ['Team', 'Team[Id,Name]'],
  ['CustomFields', 'CustomFields'],
  ['Assignments', 'Assignments[Id,Role[Id,Name],GeneralUser[Id,FirstName,LastName,Login]]'],
  ['RoleEfforts', 'RoleEfforts[Id,Role[Id,Name],Effort,EffortCompleted,EffortToDo]'],
  ['AssignedTeams', 'AssignedTeams[Id,Team[Id,Name],EntityState[Id,Name]]'],
  ...PARENT_REFS.map((p): [string, string] => [p, `${p}[Id,Name]`]),
]

const COUNTED = ['Tasks', 'Bugs', 'UserStories', 'Features', 'Epics', 'TestCases', 'Comments', 'Attachments', 'Times']

function has(catalog: Catalog, resource: CatalogResource, member: string): boolean {
  return catalog.member(resource, member) !== undefined
}

export function cardInclude(catalog: Catalog, resource: CatalogResource): string {
  return `[Id,${CARD_INCLUDE.filter(([m]) => has(catalog, resource, m)).map(([, e]) => e).join(',')}]`
}

export function cardAppend(resource: CatalogResource): string | undefined {
  const counts = COUNTED.filter((c) => resource.collections.some((x) => x.name === c)).map((c) => `${c}-Count`)
  return counts.length ? `[${counts.join(',')}]` : undefined
}

/** The nearest parent reference set on a raw card, with its resource name. */
export function parentOf(raw: Raw): { member: string; id: number; name?: string; resourceType?: string } | undefined {
  for (const member of PARENT_REFS) {
    const value = raw[member]
    if (value && typeof value === 'object' && typeof (value as Raw).Id === 'number') {
      const v = value as Raw
      return {
        member,
        id: v.Id as number,
        ...(typeof v.Name === 'string' ? { name: v.Name } : {}),
        ...(typeof v.ResourceType === 'string' ? { resourceType: v.ResourceType } : {}),
      }
    }
  }
  return undefined
}

export interface ParentLink {
  id: number
  type: string
  name?: string
  state?: string
}

/** Walks up the parent chain (task → story → feature → epic → portfolio epic), nearest first. */
export async function parentChain(ctx: ToolContext, raw: Raw, maxDepth = 5): Promise<ParentLink[]> {
  const catalog = await ctx.catalog()
  const chain: ParentLink[] = []
  const seen = new Set<number>()
  let current: Raw | undefined = raw
  while (current && chain.length < maxDepth) {
    const parent = parentOf(current)
    if (!parent || seen.has(parent.id)) break
    seen.add(parent.id)
    let resource = parent.resourceType ? catalog.find(parent.resourceType) : undefined
    if (!resource || resource.name === 'General' || resource.name === 'Assignable') {
      const info = await cardInfo(ctx, parent.id)
      if (!info.ok) {
        chain.push(compact({ id: parent.id, type: parent.resourceType, name: parent.name }) as ParentLink)
        break
      }
      resource = info.value.resource
    }
    const fields = ['Id', 'Name', 'EntityState[Id,Name]', ...PARENT_REFS.filter((p) => has(catalog, resource, p)).map((p) => `${p}[Id,Name]`)]
    const r = await ctx.v1.get<Raw>(resource.path, parent.id, { include: `[${fields.join(',')}]` })
    if (!r.ok || !r.data) {
      chain.push(compact({ id: parent.id, type: resource.name, name: parent.name }) as ParentLink)
      break
    }
    chain.push(
      compact({
        id: parent.id,
        type: resource.name,
        name: typeof r.data.Name === 'string' ? r.data.Name : parent.name,
        state: ref(r.data.EntityState)?.name,
      }) as ParentLink,
    )
    current = r.data
  }
  return chain
}

export interface CardRead {
  info: CardInfo
  raw: Raw
}

/** Reads a card by id alone: resolves its type, then reads everything read_card reports. */
export async function readCardRaw(ctx: ToolContext, id: number): Promise<{ ok: true; value: CardRead } | { ok: false; message: string; error?: Err }> {
  const info = await cardInfo(ctx, id)
  if (!info.ok) return info.error ? { ok: false, message: info.message, error: info.error } : { ok: false, message: info.message }
  const r = await readCardAs(ctx, info.value)
  if (!r.ok) return { ok: false, message: `Could not read ${info.value.entityType} ${id}`, error: r }
  return { ok: true, value: { info: info.value, raw: r.data } }
}

export async function readCardAs(ctx: ToolContext, info: CardInfo): Promise<Result<Raw>> {
  const catalog = await ctx.catalog()
  const append = cardAppend(info.resource)
  return ctx.v1.get<Raw>(info.resource.path, info.id, {
    include: cardInclude(catalog, info.resource),
    ...(append ? { append } : {}),
    innerTake: 1000,
  })
}

export function shapeCard(info: CardInfo, raw: Raw, parents?: ParentLink[]) {
  const counts: Record<string, number> = {}
  for (const [key, value] of Object.entries(raw)) {
    if (key.endsWith('-Count') && typeof value === 'number') counts[key.slice(0, -'-Count'.length).replace(/^./, (c) => c.toLowerCase())] = value
  }
  const state = raw.EntityState as Raw | undefined
  const effort = compact({
    total: num(raw.Effort),
    completed: num(raw.EffortCompleted),
    toDo: num(raw.EffortToDo),
    timeSpent: num(raw.TimeSpent),
    timeRemaining: num(raw.TimeRemain),
  })
  return compact({
    id: info.id,
    type: info.entityType,
    name: raw.Name,
    state: state ? compact({ id: num(state.Id), name: state.Name, initial: state.IsInitial || undefined, final: state.IsFinal || undefined }) : undefined,
    project: ref(raw.Project),
    parents: parents?.length ? parents : undefined,
    team: ref(raw.Team),
    teams: 'AssignedTeams' in raw ? items(raw.AssignedTeams).map(teamAssignment) : undefined,
    assignments: 'Assignments' in raw ? items(raw.Assignments).map(assignment) : undefined,
    roleEfforts: 'RoleEfforts' in raw ? items(raw.RoleEfforts).map(roleEffort) : undefined,
    effort: Object.keys(effort).length ? effort : undefined,
    release: ref(raw.Release),
    iteration: ref(raw.Iteration),
    teamIteration: ref(raw.TeamIteration),
    priority: ref(raw.Priority),
    severity: ref(raw.Severity),
    owner: person(raw.Owner),
    tags: tags(raw.Tags),
    customFields: customFields(raw.CustomFields),
    counts: Object.keys(counts).length ? counts : undefined,
    created: raw.CreateDate,
    modified: raw.ModifyDate,
    description: text(raw.Description),
  })
}
