import { err, ok, type Result } from '../http/result.js'
import { v1String, type V1Client, type V1Ref } from '../http/v1.js'
import { match, matchActive, type Resolved } from './match.js'

export interface TpUser {
  Id: number
  FirstName?: string | null
  LastName?: string | null
  Login?: string | null
  Email?: string | null
  IsActive?: boolean
  DeleteDate?: string | null
  Kind?: string
  Role?: V1Ref | null
}

export interface TpRole {
  Id: number
  Name: string
  HasEffort?: boolean
}

export interface TpTeam {
  Id: number
  Name: string
  IsActive?: boolean
}

export interface TpProject {
  Id: number
  Name: string
  Abbreviation?: string | null
  IsActive?: boolean
  Process?: V1Ref | null
}

export interface TpState {
  Id: number
  Name: string
  IsInitial?: boolean
  IsFinal?: boolean
  IsPlanned?: boolean
  NumericPriority?: number
  Workflow?: (V1Ref & { ParentWorkflow?: V1Ref | null }) | null
  ParentEntityState?: V1Ref | null
  Role?: V1Ref | null
  EntityType?: V1Ref | null
}

export interface TpCustomField {
  Id: number
  Name: string
  FieldType?: string
  /** Newline-separated options for DropDown / MultipleSelectionList. */
  Value?: string | null
  Required?: boolean
  EntityType?: V1Ref | null
  Process?: V1Ref | null
}

export function fullName(u: TpUser): string {
  return [u.FirstName, u.LastName].filter(Boolean).join(' ') || u.Login || `User ${u.Id}`
}

export function isActiveUser(u: TpUser): boolean {
  return u.IsActive !== false && !u.DeleteDate
}

/** Allowed values of a dropdown or multi-select custom field; `undefined` for free-form fields. */
export function customFieldOptions(field: TpCustomField): string[] | undefined {
  if (!field.FieldType || !/^(DropDown|MultipleSelectionList)$/i.test(field.FieldType)) return undefined
  return (field.Value ?? '')
    .split(/\r?\n/)
    .map((o) => o.trim())
    .filter(Boolean)
}

const TTL_MS = 5 * 60 * 1000
const DIRECTORY_LIMIT = 20_000

interface Entry<T> {
  at: number
  value: Promise<Result<T>>
}

/**
 * Reference data used to resolve names to ids, loaded fully paged on first use
 * and cached briefly. Failed loads are not cached.
 */
export class Directory {
  private readonly cache = new Map<string, Entry<unknown>>()

  constructor(
    private readonly v1: V1Client,
    private readonly now: () => number = Date.now,
  ) {}

  private cached<T>(key: string, load: () => Promise<Result<T>>): Promise<Result<T>> {
    const hit = this.cache.get(key) as Entry<T> | undefined
    if (hit && this.now() - hit.at < TTL_MS) return hit.value
    const value = load().then((r) => {
      if (!r.ok) this.cache.delete(key)
      return r
    })
    this.cache.set(key, { at: this.now(), value })
    return value
  }

  private all<T>(key: string, path: string, include: string, where?: string): Promise<Result<T[]>> {
    return this.cached(key, async () => {
      const r = await this.v1.list<T>(path, { include, ...(where ? { where } : {}), limit: DIRECTORY_LIMIT })
      if (!r.ok) return r
      if (r.data.truncated) {
        return err(r.status, `More than ${DIRECTORY_LIMIT} ${path}; names cannot be resolved reliably. Pass ids instead.`)
      }
      return ok(r.data.items, r.status)
    })
  }

  users(): Promise<Result<TpUser[]>> {
    return this.all('users', 'Users', '[Id,FirstName,LastName,Login,Email,IsActive,DeleteDate,Kind,Role[Id,Name]]')
  }

  roles(): Promise<Result<TpRole[]>> {
    return this.all('roles', 'Roles', '[Id,Name,HasEffort]')
  }

  teams(): Promise<Result<TpTeam[]>> {
    return this.all('teams', 'Teams', '[Id,Name,IsActive]')
  }

  projects(): Promise<Result<TpProject[]>> {
    return this.all('projects', 'Projects', '[Id,Name,Abbreviation,IsActive,Process[Id,Name]]')
  }

  /** Every state of an entity type in a process, across the project workflow and team sub-workflows. */
  states(processId: number, entityType: string): Promise<Result<TpState[]>> {
    return this.cached(`states:${processId}:${entityType}`, async () => {
      const r = await this.v1.list<TpState>('EntityStates', {
        where: `(Process.Id eq ${processId}) and (EntityType.Name eq ${v1String(entityType)})`,
        include: '[Id,Name,IsInitial,IsFinal,IsPlanned,NumericPriority,Workflow[Id,Name,ParentWorkflow],ParentEntityState[Id,Name],Role[Id,Name],EntityType[Id,Name]]',
        orderBy: 'NumericPriority',
      })
      if (!r.ok) return r
      return ok(r.data.items, r.status)
    })
  }

  customFields(processId: number, entityType: string): Promise<Result<TpCustomField[]>> {
    return this.cached(`cf:${processId}:${entityType}`, async () => {
      const r = await this.v1.list<TpCustomField>('CustomFields', {
        where: `(Process.Id eq ${processId}) and (EntityType.Name eq ${v1String(entityType)})`,
        include: '[Id,Name,FieldType,Value,Required,EntityType[Id,Name],Process[Id,Name]]',
      })
      if (!r.ok) return r
      return ok(r.data.items, r.status)
    })
  }

  loggedUser(): Promise<Result<TpUser>> {
    return this.cached('me', () => this.v1.getPath<TpUser>('Users/LoggedUser', { include: '[Id,FirstName,LastName,Login,Email,IsActive]' }))
  }

  // ---- resolvers ---------------------------------------------------------

  async user(input: string | number): Promise<Resolved<TpUser>> {
    const r = await this.users()
    if (!r.ok) return { ok: false, reason: 'error', message: 'Could not load users', error: r }
    const spec = {
      kind: 'user',
      id: (u: TpUser) => u.Id,
      name: fullName,
      keys: (u: TpUser) => [fullName(u), `${u.LastName ?? ''} ${u.FirstName ?? ''}`, u.Login, u.Email, u.FirstName, u.LastName],
      describe: (u: TpUser) => ({ login: u.Login, ...(isActiveUser(u) ? {} : { inactive: true }) }),
    }
    return matchActive(input, { ...spec, items: r.data }, isActiveUser)
  }

  async me(): Promise<Resolved<TpUser>> {
    const r = await this.loggedUser()
    return r.ok ? { ok: true, value: r.data } : { ok: false, reason: 'error', message: 'Could not identify the current user (Users/LoggedUser)', error: r }
  }

  async role(input: string | number): Promise<Resolved<TpRole>> {
    const r = await this.roles()
    if (!r.ok) return { ok: false, reason: 'error', message: 'Could not load roles', error: r }
    return match(input, { kind: 'role', items: r.data, id: (x) => x.Id, name: (x) => x.Name, keys: (x) => [x.Name] })
  }

  async team(input: string | number): Promise<Resolved<TpTeam>> {
    const r = await this.teams()
    if (!r.ok) return { ok: false, reason: 'error', message: 'Could not load teams', error: r }
    const spec = { kind: 'team', id: (x: TpTeam) => x.Id, name: (x: TpTeam) => x.Name, keys: (x: TpTeam) => [x.Name] }
    return matchActive(input, { ...spec, items: r.data }, (t) => t.IsActive !== false)
  }

  async project(input: string | number): Promise<Resolved<TpProject>> {
    const r = await this.projects()
    if (!r.ok) return { ok: false, reason: 'error', message: 'Could not load projects', error: r }
    return match(input, {
      kind: 'project',
      items: r.data,
      id: (x) => x.Id,
      name: (x) => x.Name,
      keys: (x) => [x.Name, x.Abbreviation],
      describe: (x) => ({ abbreviation: x.Abbreviation, ...(x.IsActive === false ? { inactive: true } : {}) }),
    })
  }

  /**
   * Resolves a state name against one workflow's states: the project workflow
   * by default, or a team sub-workflow when `teamWorkflow` is true.
   */
  async state(processId: number, entityType: string, input: string | number, teamWorkflow = false): Promise<Resolved<TpState>> {
    const r = await this.states(processId, entityType)
    if (!r.ok) return { ok: false, reason: 'error', message: `Could not load ${entityType} states`, error: r }
    const items = r.data.filter((s) => isTeamWorkflowState(s) === teamWorkflow)
    return match(input, {
      kind: `${entityType} state`,
      items,
      id: (x) => x.Id,
      name: (x) => x.Name,
      keys: (x) => [x.Name],
      describe: (x) => ({ workflow: x.Workflow?.Name }),
    })
  }

  /**
   * States a team's assignment on a card can take: the team's sub-workflow for
   * this entity type in this project (via TeamProjects), or the project
   * workflow when the team has none.
   */
  async teamStates(processId: number, entityType: string, teamId: number, projectId: number): Promise<Result<TpState[]>> {
    const states = await this.states(processId, entityType)
    if (!states.ok) return states
    const links = await this.cached(`teamproject:${teamId}:${projectId}`, async () => {
      const r = await this.v1.list<{ Workflows?: { Items?: { Id: number; EntityType?: V1Ref | null; ParentWorkflow?: V1Ref | null }[] } }>(
        'TeamProjects',
        { where: `(Team.Id eq ${teamId}) and (Project.Id eq ${projectId})`, include: '[Id,Workflows[Id,Name,EntityType[Name],ParentWorkflow]]', innerTake: 1000 },
      )
      return r.ok ? ok(r.data.items, r.status) : r
    })
    if (!links.ok) return links
    const workflow = links.data
      .flatMap((l) => l.Workflows?.Items ?? [])
      .find((w) => w.ParentWorkflow && w.EntityType?.Name === entityType)
    const list = workflow ? states.data.filter((s) => s.Workflow?.Id === workflow.Id) : states.data.filter((s) => !isTeamWorkflowState(s))
    return ok(list, states.status)
  }

  async customField(processId: number, entityType: string, input: string | number): Promise<Resolved<TpCustomField>> {
    const r = await this.customFields(processId, entityType)
    if (!r.ok) return { ok: false, reason: 'error', message: `Could not load ${entityType} custom fields`, error: r }
    return match(input, {
      kind: `${entityType} custom field`,
      items: r.data,
      id: (x) => x.Id,
      name: (x) => x.Name,
      keys: (x) => [x.Name],
      describe: (x) => ({ type: x.FieldType }),
    })
  }
}

export function isTeamWorkflowState(state: TpState): boolean {
  return Boolean(state.Workflow?.ParentWorkflow || state.ParentEntityState)
}
