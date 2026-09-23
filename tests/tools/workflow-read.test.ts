import { afterEach, describe, expect, it } from 'vitest'
import { FetchStub } from '../helpers/fetch-stub.js'
import { harness, type Harness } from '../helpers/harness.js'
import { general, withReferenceData } from '../fixtures/reference.js'

let h: Harness
afterEach(async () => {
  await h?.close()
})

// Payloads as our instance returns them (task 36406 and its parents, trimmed).
const task = {
  ResourceType: 'Task',
  Id: 36406,
  Name: 'Add delivery date to ODA',
  Description: '<div>Delivery date belongs on the ODA line&#44; not on PurchaseOrder.</div>',
  CreateDate: '/Date(1789479082000+0200)/',
  Tags: 'backend, oda',
  Effort: 8.0,
  EffortCompleted: 0.0,
  EffortToDo: 8.0,
  TimeSpent: 0.0,
  TimeRemain: 8.0,
  EntityType: { ResourceType: 'EntityType', Id: 5, Name: 'Task' },
  EntityState: { ResourceType: 'EntityState', Id: 691, Name: 'Open', IsInitial: true, IsFinal: false },
  Project: { ResourceType: 'Project', Id: 26080, Name: 'SBP', Process: { ResourceType: 'Process', Id: 13, Name: 'Mamami 2025' } },
  Team: { ResourceType: 'Team', Id: 447, Name: 'Core Team' },
  Release: null,
  Iteration: null,
  TeamIteration: null,
  Assignments: {
    Items: [
      {
        ResourceType: 'Assignment',
        Id: 49582,
        GeneralUser: { ResourceType: 'User', Id: 18, FirstName: 'Paolo', LastName: 'Turello', Login: 'pturello', FullName: 'Paolo Turello', Kind: 'User' },
        Role: { ResourceType: 'Role', Id: 13, Name: 'Developer' },
      },
    ],
  },
  RoleEfforts: {
    Items: [{ ResourceType: 'RoleEffort', Id: 72324, Effort: 8.0, EffortCompleted: 0.0, EffortToDo: 8.0, Role: { ResourceType: 'Role', Id: 13, Name: 'Developer' } }],
  },
  AssignedTeams: {
    Items: [
      {
        ResourceType: 'TeamAssignment',
        Id: 21420,
        Team: { ResourceType: 'Team', Id: 447, Name: 'Core Team' },
        EntityState: { ResourceType: 'EntityState', Id: 691, Name: 'Open' },
      },
    ],
  },
  UserStory: { ResourceType: 'UserStory', Id: 36216, Name: 'Review and manage ODAs' },
  CustomFields: [],
  'Comments-Count': 0,
  'Times-Count': 0,
}

const story = {
  ResourceType: 'UserStory',
  Id: 36216,
  Name: 'Review and manage ODAs',
  EntityState: { ResourceType: 'EntityState', Id: 684, Name: 'Designable' },
  Feature: { ResourceType: 'Feature', Id: 36193, Name: 'Review 2025 - Improvements' },
}
const feature = { ResourceType: 'Feature', Id: 36193, Name: 'Review 2025 - Improvements', EntityState: { Id: 1, Name: 'In progress' }, Epic: null }

describe('read_card', () => {
  it('returns assignments, role efforts, teams, parents and plain-text description', async () => {
    const stub = new FetchStub()
      .get('/api/v1/Generals/36406', general(36406, 'Task', 5))
      .get('/api/v1/Tasks/36406', task)
      .get('/api/v1/UserStories/36216', story)
      .get('/api/v1/Features/36193', feature)
    h = await harness({ stub })
    const r = await h.call('read_card', { id: 36406 })
    expect(r.isError, r.text).toBe(false)
    expect(r.json).toMatchObject({
      id: 36406,
      type: 'Task',
      state: { id: 691, name: 'Open', initial: true },
      project: { id: 26080, name: 'SBP' },
      parents: [
        { id: 36216, type: 'UserStory', name: 'Review and manage ODAs', state: 'Designable' },
        { id: 36193, type: 'Feature', state: 'In progress' },
      ],
      teams: [{ id: 21420, team: { id: 447, name: 'Core Team' }, state: { id: 691, name: 'Open' } }],
      assignments: [{ id: 49582, user: { id: 18, name: 'Paolo Turello', login: 'pturello' }, role: { id: 13, name: 'Developer' } }],
      roleEfforts: [{ id: 72324, role: { id: 13, name: 'Developer' }, effort: 8, completed: 0, toDo: 8 }],
      effort: { total: 8, toDo: 8 },
      tags: ['backend', 'oda'],
      created: '2026-09-15T13:31:22.000Z',
      description: 'Delivery date belongs on the ODA line, not on PurchaseOrder.',
      counts: { comments: 0, times: 0 },
    })
    const include = h.stub.find('GET', '/api/v1/Tasks/36406')[0]!.query.get('include')!
    expect(include).toContain('Assignments[Id,Role[Id,Name],GeneralUser[Id,FirstName,LastName,Login]]')
    expect(include).toContain('RoleEfforts[')
    expect(include).not.toContain('Severity') // Task has no Severity
    expect(h.stub.find('GET', '/api/v1/Tasks/36406')[0]!.query.get('innerTake')).toBe('1000')
  })

  it('includes empty assignments explicitly', async () => {
    const stub = new FetchStub()
      .get('/api/v1/Generals/9', general(9, 'UserStory'))
      .get('/api/v1/UserStories/9', { Id: 9, Name: 'Login', Assignments: { Items: [] }, RoleEfforts: { Items: [] }, Feature: null })
    h = await harness({ stub })
    const r = await h.call('read_card', { id: 9 })
    expect(r.json.assignments).toEqual([])
    expect(r.json.roleEfforts).toEqual([])
  })

  it('reports a missing card without guessing', async () => {
    h = await harness({ stub: new FetchStub().get('/api/v1/Generals/36410', { Status: 'NotFound', Message: 'General with Id 36410 not found or access is forbidden.' }, { status: 404 }) })
    const r = await h.call('read_card', { id: 36410 })
    expect(r.isError).toBe(true)
    expect(r.text.split('\n')[0]).toBe('No card with id 36410 (or no access to it).')
    expect(r.text).toContain('status: 404')
  })
})

describe('read_search / read_my_work', () => {
  const found = {
    Items: [
      {
        ResourceType: 'Assignable',
        Id: 15153,
        Name: 'Search it\'s fine',
        EntityType: { ResourceType: 'EntityType', Id: 4, Name: 'UserStory' },
        EntityState: { ResourceType: 'EntityState', Id: 682, Name: 'Refining' },
        Project: { ResourceType: 'Project', Id: 26080, Name: 'SBP' },
        Tags: '',
        ModifyDate: '/Date(0+0000)/',
      },
    ],
  }

  it('builds the v1 filter from resolved names, escaping quotes', async () => {
    h = await harness({ stub: withReferenceData(new FetchStub().get('/api/v1/Assignables', found)) })
    const r = await h.call('read_search', { text: "it's", project: 'SBP', assignee: 'Giorgio Marchetti', tag: 'ui' })
    expect(r.isError, r.text).toBe(false)
    const where = h.stub.find('GET', '/api/v1/Assignables')[0]!.query.get('where')
    expect(where).toBe(
      "(Name contains 'it\\'s') and (Project.Id eq 26080) and (AssignedUser.Id eq 16) and (TagObjects.Name eq 'ui') and (EntityState.IsFinal eq 'false')",
    )
    expect(r.json).toMatchObject({ count: 1, truncated: false, byState: { Refining: 1 }, cards: [{ id: 15153, type: 'UserStory', state: 'Refining', project: 'SBP' }] })
  })

  it('refuses an ambiguous assignee and sends no search', async () => {
    h = await harness({ stub: withReferenceData(new FetchStub()) })
    const r = await h.call('read_search', { assignee: 'giorg' })
    expect(r.isError).toBe(true)
    expect(r.text).toContain('Giorgio Marchetti (16)')
    expect(h.stub.find('GET', '/api/v1/Assignables')).toHaveLength(0)
  })

  it('read_my_work filters on the logged user', async () => {
    h = await harness({ stub: withReferenceData(new FetchStub().get('/api/v1/Tasks', found)) })
    const r = await h.call('read_my_work', { type: 'Task' })
    expect(r.json.user).toEqual({ id: 2286, name: 'Leszek Bielski' })
    expect(h.stub.find('GET', '/api/v1/Tasks')[0]!.query.get('where')).toBe("(AssignedUser.Id eq 2286) and (EntityState.IsFinal eq 'false')")
  })
})

describe('read_states', () => {
  it("lists the card's workflow, flags team states, shows current and team states", async () => {
    const stub = withReferenceData(
      new FetchStub()
        .get('/api/v1/Generals/36216', general(36216, 'UserStory'))
        .get('/api/v1/UserStories/36216', {
          Id: 36216,
          EntityState: { Id: 684, Name: 'Designable' },
          AssignedTeams: { Items: [{ Id: 1, Team: { Id: 447, Name: 'Core Team' }, EntityState: { Id: 684, Name: 'Designable' } }] },
        }),
    )
    h = await harness({ stub })
    const r = await h.call('read_states', { id: 36216 })
    expect(r.isError, r.text).toBe(false)
    expect(r.json.current).toEqual({ id: 684, name: 'Designable' })
    expect(r.json.teamStates).toEqual([{ team: { id: 447, name: 'Core Team' }, state: { id: 684, name: 'Designable' } }])
    expect(r.json.states[0]).toMatchObject({ id: 681, name: 'Open', initial: true, role: 'Product Owner' })
    expect(r.json.states.find((s: { id: number }) => s.id === 900)).toMatchObject({ isTeamWorkflow: true, parentState: 'In Progress' })
    const where = h.stub.find('GET', '/api/v1/EntityStates')[0]!.query.get('where')
    expect(where).toBe("(Process.Id eq 13) and (EntityType.Name eq 'UserStory')")
  })

  it('works from project + type, and requires one of them', async () => {
    h = await harness({ stub: withReferenceData(new FetchStub()) })
    const r = await h.call('read_states', { project: 'SBP', type: 'Task' })
    expect(r.json.states.map((s: { name: string }) => s.name)).toEqual(['Open', 'In Progress', 'Coded', 'Done'])
    const bad = await h.call('read_states', {})
    expect(bad.isError).toBe(true)
  })
})

describe('reference data', () => {
  it('read_people filters by every word and hides inactive people', async () => {
    h = await harness({ stub: withReferenceData(new FetchStub()) })
    const r = await h.call('read_people', { query: 'giorg' })
    expect(r.json.people.map((p: { id: number }) => p.id)).toEqual([16, 17])
    const all = await h.call('read_people', { includeInactive: true, limit: 2 })
    expect(all.json).toMatchObject({ count: 2, truncated: true })
  })

  it('read_roles, read_teams, read_projects', async () => {
    h = await harness({ stub: withReferenceData(new FetchStub()) })
    expect((await h.call('read_roles')).json.roles).toContainEqual({ id: 8, name: 'Top Manager', hasEffort: false })
    expect((await h.call('read_teams')).json.teams.map((t: { name: string }) => t.name)).toEqual(['Core Team', 'Pandolfo Team'])
    expect((await h.call('read_projects')).json.projects).toEqual([{ id: 26080, name: 'SBP', abbreviation: 'SBP', process: { id: 13, name: 'Mamami 2025' } }])
  })

  it('read_custom_field_options returns dropdown values', async () => {
    h = await harness({ stub: withReferenceData(new FetchStub().get('/api/v1/Generals/5', general(5, 'UserStory'))) })
    const r = await h.call('read_custom_field_options', { id: 5, field: 'backend' })
    expect(r.json.fields).toEqual([{ id: 150, name: 'BackEnd', type: 'DropDown', options: ['To Do', 'Doing', 'Review', 'Done'] }])
  })

  it('read_iterations defaults to current team iterations', async () => {
    h = await harness({ stub: new FetchStub().get('/api/v1/TeamIterations', { Items: [{ Id: 1, Name: 'W1', IsCurrent: true, StartDate: '/Date(0)/', Team: { Id: 447, Name: 'Core Team' } }] }) })
    const r = await h.call('read_iterations')
    expect(h.stub.calls[0]!.query.get('where')).toBe("(IsCurrent eq 'true')")
    expect(r.json.iterations[0]).toMatchObject({ id: 1, current: true, team: { id: 447 } })
  })

  it('read_releases filters by project', async () => {
    h = await harness({ stub: withReferenceData(new FetchStub().get('/api/v1/Releases', { Items: [] })) })
    await h.call('read_releases', { project: 'sbp', currentOnly: true })
    expect(h.stub.find('GET', '/api/v1/Releases')[0]!.query.get('where')).toBe("(Project.Id eq 26080) and (IsCurrent eq 'true')")
  })
})

describe('per-card reads', () => {
  it('read_comments returns plain text', async () => {
    h = await harness({
      stub: new FetchStub().get('/api/v1/Comments', {
        Items: [
          {
            Id: 33401,
            Description: '<div>Fatto&#33;</div>',
            ParentId: null,
            CreateDate: '/Date(1790162026000+0200)/',
            IsPrivate: false,
            Owner: { Id: 16, FirstName: 'Giorgio', LastName: 'Marchetti', Login: 'g', FullName: 'Giorgio Marchetti' },
          },
        ],
      }),
    })
    const r = await h.call('read_comments', { id: 36507 })
    expect(h.stub.calls[0]!.query.get('where')).toBe('(General.Id eq 36507)')
    expect(r.json.comments).toEqual([{ id: 33401, author: { id: 16, name: 'Giorgio Marchetti', login: 'g' }, date: '2026-09-23T11:13:46.000Z', text: 'Fatto!' }])
  })

  it('read_relations merges both directions', async () => {
    const rel = (id: number, other: unknown) => ({ Id: id, RelationType: { Id: 3, Name: 'Relation' }, Master: other, Slave: other })
    const other = { Id: 36487, Name: 'PM', EntityType: { Name: 'Request' } }
    h = await harness({
      stub: new FetchStub()
        .get('/api/v1/Relations', { Items: [rel(1, other)] }, { query: { where: '(Master.Id eq 36506)' } })
        .get('/api/v1/Relations', { Items: [rel(2, other)] }, { query: { where: '(Slave.Id eq 36506)' } }),
    })
    const r = await h.call('read_relations', { id: 36506 })
    expect(r.json.relations).toEqual([
      { id: 1, relation: 'Relation', direction: 'outbound', card: { id: 36487, type: 'Request', name: 'PM' } },
      { id: 2, relation: 'Relation', direction: 'inbound', card: { id: 36487, type: 'Request', name: 'PM' } },
    ])
  })

  it('read_times totals spent time and needs a card or user', async () => {
    h = await harness({
      stub: new FetchStub().get('/api/v1/Times', { Items: [{ Id: 1, Spent: 0.15, Remain: 0, Date: '/Date(0)/' }, { Id: 2, Spent: 0.25, Remain: 0, Date: '/Date(0)/' }] }),
    })
    const r = await h.call('read_times', { id: 36488, from: '2026-09-01' })
    expect(r.json.totalSpent).toBe(0.4)
    expect(h.stub.calls[0]!.query.get('where')).toBe("(Assignable.Id eq 36488) and (Date gte '2026-09-01')")
    expect((await h.call('read_times', {})).isError).toBe(true)
  })

  it('read_attachments explains the download limitation', async () => {
    h = await harness({ stub: new FetchStub().get('/api/v1/Attachments', { Items: [{ Id: 1, Name: 'a.docx', Uri: 'https://tp.example/Attachment.aspx?AttachmentID=1' }] }) })
    const r = await h.call('read_attachments', { id: 5 })
    expect(r.json.attachments[0]).toMatchObject({ id: 1, name: 'a.docx' })
    const tool = (await h.listTools()).find((t) => t.name === 'read_attachments')
    expect(tool?.description).toMatch(/HTML error page/)
  })

  it('read_test_plan returns cases with ordered steps', async () => {
    h = await harness({
      stub: new FetchStub()
        .get('/api/v1/TestPlans/17796', { Id: 17796, Name: 'Export', EntityState: { Id: 134, Name: 'Open' }, ChildTestPlans: { Items: [] } })
        .get('/api/v1/TestPlans/17796/TestCases', {
          Items: [
            {
              Id: 1,
              Name: 'Case',
              LastStatus: null,
              TestSteps: { Items: [{ Id: 2, Description: '<div>second</div>', Result: 'ok', RunOrder: 2 }, { Id: 1, Description: 'first', Result: null, RunOrder: 1 }] },
            },
          ],
        })
        .get('/api/v1/TestPlanRuns', { Items: [] }),
    })
    const r = await h.call('read_test_plan', { id: 17796 })
    expect(r.json.testCases[0].steps).toEqual([
      { id: 1, order: 1, step: 'first' },
      { id: 2, order: 2, step: 'second', expected: 'ok' },
    ])
    expect(r.json.runs).toEqual([])
  })
})
