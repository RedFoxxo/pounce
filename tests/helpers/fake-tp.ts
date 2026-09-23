import { FetchStub, StubReply, type StubCall } from './fetch-stub.js'
import { general, roles, storyStates, taskStates, users, withReferenceData } from '../fixtures/reference.js'

/**
 * A small stateful Targetprocess: cards with assignments, role efforts, team
 * assignments and states, reproducing the behaviours the domain rules exist
 * for (default assignments on creation, task effort rollup into the story,
 * parent story moving when a task leaves its initial state).
 */

type Ref = { Id: number; Name?: string }

interface Card {
  Id: number
  type: 'UserStory' | 'Task' | 'Bug' | 'Feature' | 'TestPlan' | 'TestCase'
  Name: string
  Description: string | null
  Tags: string
  Project: Ref
  EntityState: Ref
  parent?: { member: 'UserStory' | 'Feature'; id: number }
  CustomFields: { Name: string; Type: string; Value: unknown }[]
}

interface Assignment {
  Id: number
  card: number
  user: number
  role: number
}

interface RoleEffort {
  Id: number
  card: number
  role: number
  Effort: number
}

interface TeamAssignment {
  Id: number
  card: number
  team: number
  state: Ref
}

const PLURAL: Record<string, Card['type']> = {
  UserStories: 'UserStory',
  Tasks: 'Task',
  Bugs: 'Bug',
  Features: 'Feature',
  TestPlans: 'TestPlan',
  TestCases: 'TestCase',
}
const TYPE_ID: Record<Card['type'], number> = { UserStory: 4, Task: 5, Bug: 8, Feature: 9, TestPlan: 13, TestCase: 12 }
const teamNames: Record<number, string> = { 447: 'Core Team', 19523: 'Pandolfo Team' }

function statesOf(type: Card['type']) {
  return type === 'Task' ? taskStates : storyStates
}

function stateRef(type: Card['type'], name: string): Ref {
  const s = statesOf(type).find((x) => x.Name === name)!
  return { Id: s.Id, Name: s.Name }
}

function whereId(call: StubCall, field: string): number | undefined {
  const m = new RegExp(`${field.replace('.', '\\.')} eq (\\d+)`).exec(call.query.get('where') ?? '')
  return m ? Number(m[1]) : undefined
}

export class FakeTp {
  readonly stub = new FetchStub()
  readonly cards = new Map<number, Card>()
  assignments: Assignment[] = []
  roleEfforts: RoleEffort[] = []
  teams: TeamAssignment[] = []
  private nextId = 50_000
  /** Default assignment Targetprocess adds on creation: Giorgio Verdi (16) as Product Owner (7). */
  defaultAssignment: { user: number; role: number } | undefined = { user: 16, role: 7 }
  /** Rejects the next write matching this path with the given status. */
  failNext?: { method: string; path: RegExp; status: number; message: string }

  constructor() {
    this.routes()
    withReferenceData(this.stub)
  }

  id(): number {
    return this.nextId++
  }

  addCard(card: Partial<Card> & Pick<Card, 'Id' | 'type' | 'Name'>, extra: { efforts?: Record<number, number>; assign?: [number, number][]; teams?: number[] } = {}): Card {
    const full: Card = {
      Description: null,
      Tags: '',
      Project: { Id: 26080, Name: 'SBP' },
      EntityState: stateRef(card.type, 'Open'),
      CustomFields: [
        { Name: 'BackEnd', Type: 'DropDown', Value: 'To Do' },
        { Name: 'FrontEnd', Type: 'DropDown', Value: 'To Do' },
      ],
      ...card,
    }
    this.cards.set(full.Id, full)
    for (const role of [12, 13, 7]) this.roleEfforts.push({ Id: this.id(), card: full.Id, role, Effort: extra.efforts?.[role] ?? 0 })
    for (const [user, role] of extra.assign ?? []) this.assignments.push({ Id: this.id(), card: full.Id, user, role })
    for (const team of extra.teams ?? []) this.teams.push({ Id: this.id(), card: full.Id, team, state: full.EntityState })
    return full
  }

  effortOf(cardId: number, role: number): number | undefined {
    return this.roleEfforts.find((e) => e.card === cardId && e.role === role)?.Effort
  }

  private userRef(id: number) {
    const u = users.find((x) => x.Id === id)!
    return { ResourceType: 'User', Id: u.Id, FirstName: u.FirstName, LastName: u.LastName, Login: u.Login }
  }

  private roleRef(id: number) {
    const r = roles.find((x) => x.Id === id)!
    return { ResourceType: 'Role', Id: r.Id, Name: r.Name }
  }

  private assignmentJson(a: Assignment) {
    return { ResourceType: 'Assignment', Id: a.Id, GeneralUser: this.userRef(a.user), Role: this.roleRef(a.role) }
  }

  render(card: Card): Record<string, unknown> {
    const efforts = this.roleEfforts.filter((e) => e.card === card.Id)
    const out: Record<string, unknown> = {
      ResourceType: card.type,
      Id: card.Id,
      Name: card.Name,
      Description: card.Description,
      Tags: card.Tags,
      EntityType: { ResourceType: 'EntityType', Id: TYPE_ID[card.type], Name: card.type },
      EntityState: { ResourceType: 'EntityState', ...card.EntityState },
      Project: { ResourceType: 'Project', ...card.Project, Process: { ResourceType: 'Process', Id: 13 } },
      Effort: efforts.reduce((s, e) => s + e.Effort, 0),
      CustomFields: card.CustomFields,
      Assignments: { Items: this.assignments.filter((a) => a.card === card.Id).map((a) => this.assignmentJson(a)) },
      RoleEfforts: { Items: efforts.map((e) => ({ ResourceType: 'RoleEffort', Id: e.Id, Effort: e.Effort, Role: this.roleRef(e.role) })) },
      AssignedTeams: {
        Items: this.teams
          .filter((t) => t.card === card.Id)
          .map((t) => ({ ResourceType: 'TeamAssignment', Id: t.Id, Team: { Id: t.team, Name: teamNames[t.team] }, EntityState: t.state })),
      },
    }
    if (card.type === 'Task') out.UserStory = card.parent ? { ResourceType: 'UserStory', Id: card.parent.id, Name: this.cards.get(card.parent.id)?.Name } : null
    if (card.type === 'UserStory') out.Feature = card.parent ? { ResourceType: 'Feature', Id: card.parent.id, Name: this.cards.get(card.parent.id)?.Name } : null
    if (card.type === 'Bug') {
      out.UserStory = card.parent?.member === 'UserStory' ? { ResourceType: 'UserStory', Id: card.parent.id } : null
      out.Feature = card.parent?.member === 'Feature' ? { ResourceType: 'Feature', Id: card.parent.id } : null
    }
    return out
  }

  /** Targetprocess recomputes a story's role effort from its tasks, overwriting the story's own value. */
  private rollup(task: Card) {
    if (task.type !== 'Task' || !task.parent) return
    const siblings = [...this.cards.values()].filter((c) => c.type === 'Task' && c.parent?.id === task.parent!.id).map((c) => c.Id)
    for (const role of [12, 13, 7]) {
      const sum = this.roleEfforts.filter((e) => siblings.includes(e.card) && e.role === role).reduce((s, e) => s + e.Effort, 0)
      const row = this.roleEfforts.find((e) => e.card === task.parent!.id && e.role === role)
      if (row && sum > 0) row.Effort = sum
    }
  }

  private rollupAll() {
    for (const c of this.cards.values()) if (c.type === 'Task') this.rollup(c)
  }

  private shouldFail(call: StubCall): { status: number; message: string } | undefined {
    const f = this.failNext
    if (f && f.method === call.method && f.path.test(call.path)) {
      this.failNext = undefined
      return { status: f.status, message: f.message }
    }
    return undefined
  }

  private routes() {
    const s = this.stub
    const error = (status: number, message: string) => new StubReply(status, { Status: 'Error', Message: message })
    const reply = (value: unknown) => value

    s.get(/^\/api\/v1\/Generals\/\d+$/, (call: StubCall) => {
      const card = this.cards.get(Number(call.path.split('/').pop()))
      return card ? { ...general(card.Id, card.type, TYPE_ID[card.type]), Name: card.Name } : error(404, 'not found')
    })

    s.get('/api/v1/Assignments', (call: StubCall) => ({
      Items: this.assignments.filter((a) => a.card === whereId(call, 'Assignable.Id')).map((a) => this.assignmentJson(a)),
    }))
    s.on({
      method: 'POST',
      path: '/api/v1/Assignments',
      body: (call: StubCall) => {
        const fail = this.shouldFail(call)
        if (fail) return error(fail.status, fail.message)
        const b = call.body as { Assignable: Ref; GeneralUser: Ref; Role: Ref }
        const a = { Id: this.id(), card: b.Assignable.Id, user: b.GeneralUser.Id, role: b.Role.Id }
        this.assignments.push(a)
        return this.assignmentJson(a)
      },
    })
    s.on({
      method: 'DELETE',
      path: /^\/api\/v1\/Assignments\/\d+$/,
      body: (call: StubCall) => {
        const idv = Number(call.path.split('/').pop())
        this.assignments = this.assignments.filter((a) => a.Id !== idv)
        return ''
      },
    })

    s.on({
      method: 'POST',
      path: /^\/api\/v1\/RoleEfforts(\/\d+)?$/,
      body: (call: StubCall) => {
        const fail = this.shouldFail(call)
        if (fail) return error(fail.status, fail.message)
        const idPart = call.path.split('/')[4]
        const b = call.body as { Effort: number; Assignable?: Ref; Role?: Ref }
        let row: RoleEffort
        if (idPart) {
          row = this.roleEfforts.find((e) => e.Id === Number(idPart))!
          row.Effort = b.Effort
        } else {
          row = { Id: this.id(), card: b.Assignable!.Id, role: b.Role!.Id, Effort: b.Effort }
          this.roleEfforts.push(row)
        }
        this.rollup(this.cards.get(row.card)!)
        return { Id: row.Id, Effort: row.Effort }
      },
    })

    s.on({
      method: 'POST',
      path: /^\/api\/v1\/TeamAssignments\/\d+$/,
      body: (call: StubCall) => {
        const ta = this.teams.find((t) => t.Id === Number(call.path.split('/').pop()))!
        const want = (call.body as { EntityState: Ref }).EntityState.Id
        const st = [...storyStates, ...taskStates].find((x) => x.Id === want)!
        ta.state = { Id: st.Id, Name: st.Name }
        return { Id: ta.Id }
      },
    })
    s.on({
      method: 'DELETE',
      path: /^\/api\/v1\/TeamAssignments\/\d+$/,
      body: (call: StubCall) => {
        this.teams = this.teams.filter((t) => t.Id !== Number(call.path.split('/').pop()))
        return ''
      },
    })
    s.get('/api/v1/TeamProjects', { Items: [] })

    // cards: read
    s.get(/^\/api\/v1\/(UserStories|Tasks|Bugs|Features|TestPlans|TestCases)\/\d+$/, (call: StubCall) => {
      const card = this.cards.get(Number(call.path.split('/').pop()))
      return card ? this.render(card) : error(404, 'not found')
    })

    // cards: create
    s.on({
      method: 'POST',
      path: /^\/api\/v1\/(UserStories|Tasks|Bugs|Features|TestPlans|TestCases)$/,
      body: (call: StubCall) => {
        const fail = this.shouldFail(call)
        if (fail) return error(fail.status, fail.message)
        const type = PLURAL[call.path.split('/')[3]!]!
        const b = call.body as Record<string, any>
        if (!b.Project?.Id) return error(400, 'Project is required')
        const parentMember = (['UserStory', 'Feature'] as const).find((m) => b[m]?.Id)
        const card = this.addCard(
          {
            Id: this.id(),
            type,
            Name: b.Name,
            Description: b.Description ?? null,
            Tags: b.Tags ?? '',
            Project: { Id: b.Project.Id, Name: 'SBP' },
            ...(b.EntityState ? { EntityState: [...storyStates, ...taskStates].filter((x) => x.Id === b.EntityState.Id).map((x) => ({ Id: x.Id, Name: x.Name }))[0]! } : {}),
            ...(parentMember ? { parent: { member: parentMember, id: b[parentMember].Id } } : {}),
          },
          { teams: (b.AssignedTeams?.Items ?? []).map((t: any) => t.Team.Id) },
        )
        for (const cf of b.CustomFields ?? []) {
          const f = card.CustomFields.find((x) => x.Name === cf.Name)
          if (f) f.Value = cf.Value
        }
        if (this.defaultAssignment) this.assignments.push({ Id: this.id(), card: card.Id, ...this.defaultAssignment })
        return reply({ ResourceType: type, Id: card.Id, Name: card.Name })
      },
    })

    // cards: delete
    s.on({
      method: 'DELETE',
      path: /^\/api\/v1\/(UserStories|Tasks|Bugs|Features|TestPlans|TestCases)\/\d+$/,
      body: (call: StubCall) => {
        const fail = this.shouldFail(call)
        if (fail) return error(fail.status, fail.message)
        const idv = Number(call.path.split('/').pop())
        const card = this.cards.get(idv)
        if (!card) return error(404, 'not found')
        this.cards.delete(idv)
        this.assignments = this.assignments.filter((a) => a.card !== idv)
        this.roleEfforts = this.roleEfforts.filter((e) => e.card !== idv)
        this.rollupAll()
        return ''
      },
    })

    // cards: update
    s.on({
      method: 'POST',
      path: /^\/api\/v1\/(UserStories|Tasks|Bugs|Features)\/\d+$/,
      body: (call: StubCall) => {
        const fail = this.shouldFail(call)
        if (fail) return error(fail.status, fail.message)
        const card = this.cards.get(Number(call.path.split('/').pop()))
        if (!card) return error(404, 'not found')
        const b = call.body as Record<string, any>
        if (b.Name !== undefined) card.Name = b.Name
        if (b.Description !== undefined) card.Description = b.Description
        if (b.Tags !== undefined) card.Tags = b.Tags
        for (const cf of b.CustomFields ?? []) {
          const f = card.CustomFields.find((x) => x.Name === cf.Name)
          if (f) f.Value = cf.Value
        }
        for (const t of b.AssignedTeams?.Items ?? []) this.teams.push({ Id: this.id(), card: card.Id, team: t.Team.Id, state: card.EntityState })
        if (b.EntityState) {
          const from = card.EntityState
          const st = statesOf(card.type).find((x) => x.Id === b.EntityState.Id)
          if (!st) return error(400, 'State does not belong to the workflow')
          card.EntityState = { Id: st.Id, Name: st.Name }
          for (const t of this.teams.filter((x) => x.card === card.Id)) t.state = card.EntityState
          // A task leaving its initial state starts its story.
          const initial = statesOf(card.type).find((x) => x.IsInitial)!
          if (card.type === 'Task' && from.Id === initial.Id && card.parent) {
            const story = this.cards.get(card.parent.id)!
            if (['Open', 'Refining', 'Designable', 'Estimated', 'Ready'].includes(story.EntityState.Name ?? '')) story.EntityState = stateRef('UserStory', 'In Progress')
          }
        }
        return this.render(card)
      },
    })
  }
}
