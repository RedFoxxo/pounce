import { afterEach, describe, expect, it } from 'vitest'
import { FakeTp } from '../helpers/fake-tp.js'
import { StubReply } from '../helpers/fetch-stub.js'
import { harness, type Harness } from '../helpers/harness.js'

let h: Harness
afterEach(async () => {
  await h?.close()
})

// Roles: Developer 13, Product Owner 7, Designer 12. Users: Foxxo 2286 (me), Giorgio 16, Giorgia 17.

function world() {
  const tp = new FakeTp()
  tp.addCard({ Id: 36193, type: 'Feature', Name: 'Review 2025' })
  const story = tp.addCard({ Id: 36216, type: 'UserStory', Name: 'Review and manage orders', EntityState: { Id: 683, Name: 'Estimated' }, parent: { member: 'Feature', id: 36193 } }, {
    efforts: { 13: 20 },
    assign: [[2286, 13], [16, 7]],
    teams: [447],
  })
  const task = tp.addCard({ Id: 36406, type: 'Task', Name: 'Add delivery date', parent: { member: 'UserStory', id: 36216 } }, { assign: [[2286, 13]], teams: [447] })
  return { tp, story, task }
}

const writes = (h: Harness) => h.stub.writes.map((c) => `${c.method} ${c.path}`)

describe('write_set_role_effort', () => {
  it('writes the role row, never the card Effort, and reports the story rollup', async () => {
    const { tp } = world()
    h = await harness({ stub: tp.stub })
    const r = await h.call('write_set_role_effort', { id: 36406, efforts: [{ role: 'Developer', effort: 6 }] })
    expect(r.isError, r.text).toBe(false)
    const post = h.stub.writes[0]!
    expect(post.path).toMatch(/^\/api\/v1\/RoleEfforts\/\d+$/)
    expect(post.body).toEqual({ Effort: 6, Id: expect.any(Number) })
    expect(h.stub.writes.some((c) => c.path === '/api/v1/Tasks/36406' && JSON.stringify(c.body).includes('"Effort"'))).toBe(false)
    expect(r.json.efforts).toEqual([{ role: 'Developer', before: 0, after: 6, row: expect.any(Number), created: false }])
    // The story's Developer effort (20) was overwritten by the rollup (6): reported, not hidden.
    expect(r.json.parent).toMatchObject({ id: 36216, type: 'UserStory', roleEfforts: [{ role: 'Developer', before: 20, after: 6 }] })
    expect(r.json.notPersisted).toBeUndefined()
  })

  it('requires a role for every effort', async () => {
    const { tp } = world()
    h = await harness({ stub: tp.stub })
    const r = await h.call('write_set_role_effort', { id: 36406, efforts: [{ effort: 6 }] })
    expect(r.isError).toBe(true)
    expect(h.stub.writes).toHaveLength(0)
  })

  it('refuses roles that carry no effort and ambiguous roles, sending nothing', async () => {
    const { tp } = world()
    h = await harness({ stub: tp.stub })
    expect((await h.call('write_set_role_effort', { id: 36406, efforts: [{ role: 'Top Manager', effort: 1 }] })).text).toMatch(/does not carry effort/)
    expect((await h.call('write_set_role_effort', { id: 36406, efforts: [{ role: 'end developer', effort: 1 }] })).text).toMatch(/Backend Developer \(1\), Frontend Developer \(11\)/)
    expect(h.stub.writes).toHaveLength(0)
  })

  it('creates a row when the card has none for the role', async () => {
    const { tp } = world()
    tp.roleEfforts = tp.roleEfforts.filter((e) => !(e.card === 36406 && e.role === 13))
    h = await harness({ stub: tp.stub })
    const r = await h.call('write_set_role_effort', { id: 36406, efforts: [{ role: 13, effort: 1 }] })
    expect(h.stub.writes[0]!.body).toEqual({ Assignable: { Id: 36406 }, Role: { Id: 13 }, Effort: 1 })
    expect(r.json.efforts[0].created).toBe(true)
    expect(tp.effortOf(36406, 13)).toBe(1)
  })

  it('reports a Targetprocess failure with status and body', async () => {
    const { tp } = world()
    tp.failNext = { method: 'POST', path: /RoleEfforts/, status: 400, message: 'Effort cannot be negative' }
    h = await harness({ stub: tp.stub })
    const r = await h.call('write_set_role_effort', { id: 36406, efforts: [{ role: 'Developer', effort: 3 }] })
    expect(r.isError).toBe(true)
    expect(r.text).toContain('Could not set Developer effort to 3')
    expect(r.text).toContain('status: 400')
    expect(r.text).toContain('Effort cannot be negative')
  })
})

describe('write_set_state', () => {
  it('moves the card, reads it back and reports the parent story moving', async () => {
    const { tp } = world()
    h = await harness({ stub: tp.stub })
    const r = await h.call('write_set_state', { id: 36406, state: 'coded' })
    expect(r.isError, r.text).toBe(false)
    expect(h.stub.writes.map((c) => c.body)).toEqual([{ EntityState: { Id: 693 }, Id: 36406 }])
    expect(r.json.state).toEqual({ before: 'Open', requested: 'Coded', after: 'Coded', changed: true })
    expect(r.json.parent).toMatchObject({ id: 36216, state: { before: 'Estimated', after: 'In Progress', changed: true } })
  })

  it('resolves against the card workflow only and lists close states', async () => {
    const { tp } = world()
    h = await harness({ stub: tp.stub })
    const r = await h.call('write_set_state', { id: 36406, state: 'Codde' })
    expect(r.isError).toBe(true)
    expect(r.text).toMatch(/No Task state matches "Codde". Did you mean: Coded \(693\)/)
    expect(h.stub.writes).toHaveLength(0)
  })

  it('does nothing when the card is already there', async () => {
    const { tp } = world()
    h = await harness({ stub: tp.stub })
    const r = await h.call('write_set_state', { id: 36406, state: 'Open' })
    expect(r.json.state.changed).toBe(false)
    expect(h.stub.writes).toHaveLength(0)
  })

  it("sets a team's state through its TeamAssignment", async () => {
    const { tp } = world()
    h = await harness({ stub: tp.stub })
    const r = await h.call('write_set_state', { id: 36216, state: 'Ready', team: 'Core Team' })
    expect(r.isError, r.text).toBe(false)
    expect(writes(h)).toEqual([expect.stringMatching(/^POST \/api\/v1\/TeamAssignments\/\d+$/)])
    expect(r.json.teamState).toMatchObject({ requested: 'Ready', after: 'Ready', changed: true })
  })

  it('surfaces a refused transition', async () => {
    const { tp } = world()
    tp.failNext = { method: 'POST', path: /Tasks\/36406/, status: 400, message: 'Comment is required for this state' }
    h = await harness({ stub: tp.stub })
    const r = await h.call('write_set_state', { id: 36406, state: 'Done' })
    expect(r.isError).toBe(true)
    expect(r.text).toContain('Comment is required for this state')
  })
})

describe('write_assign / write_unassign', () => {
  it('adds without removing anyone by default', async () => {
    const { tp } = world()
    h = await harness({ stub: tp.stub })
    const r = await h.call('write_assign', { id: 36216, user: 'Giorgia Rossi', role: 'Developer' })
    expect(r.isError, r.text).toBe(false)
    expect(writes(h)).toEqual(['POST /api/v1/Assignments'])
    expect(h.stub.writes[0]!.body).toEqual({ Assignable: { Id: 36216 }, GeneralUser: { Id: 17 }, Role: { Id: 13 } })
    expect(r.json.removed).toEqual([])
    expect(r.json.assignments).toHaveLength(3)
  })

  it('exclusive removes only the others in that role and says so', async () => {
    const { tp } = world()
    h = await harness({ stub: tp.stub })
    const r = await h.call('write_assign', { id: 36216, user: 'giorgia@example.com', role: 'Developer', exclusive: true })
    expect(r.json.removed).toEqual([{ id: expect.any(Number), user: 'Foxxo Vulpes (2286)', role: 'Developer' }])
    expect(tp.assignments.filter((a) => a.card === 36216).map((a) => [a.user, a.role])).toEqual([
      [16, 7],
      [17, 13],
    ])
  })

  it('is a no-op for an existing assignment', async () => {
    const { tp } = world()
    h = await harness({ stub: tp.stub })
    const r = await h.call('write_assign', { id: 36216, user: 'me', role: 'Developer' })
    expect(r.json.alreadyAssigned).toBe(true)
    expect(h.stub.writes).toHaveLength(0)
  })

  it('never guesses an ambiguous person', async () => {
    const { tp } = world()
    h = await harness({ stub: tp.stub })
    const r = await h.call('write_assign', { id: 36216, user: 'giorg', role: 'Developer' })
    expect(r.isError).toBe(true)
    expect(r.text).toContain('Giorgio Verdi (16)')
    expect(h.stub.writes).toHaveLength(0)
  })

  it('unassign removes one exact assignment', async () => {
    const { tp } = world()
    h = await harness({ stub: tp.stub })
    const r = await h.call('write_unassign', { id: 36216, user: 'Giorgio Verdi' })
    expect(r.isError, r.text).toBe(false)
    expect(r.json.removed).toMatchObject({ user: 'Giorgio Verdi (16)', role: 'Product Owner' })
    expect(writes(h)).toEqual([expect.stringMatching(/^DELETE \/api\/v1\/Assignments\/\d+$/)])
  })

  it('unassign needs the role when the person holds several', async () => {
    const { tp } = world()
    tp.assignments.push({ Id: 1, card: 36216, user: 2286, role: 7 })
    h = await harness({ stub: tp.stub })
    const r = await h.call('write_unassign', { id: 36216, user: 'me' })
    expect(r.text).toMatch(/holds 2 roles .*pass the role/)
    expect(h.stub.writes).toHaveLength(0)
  })
})

describe('write_create_card', () => {
  it('inherits the project, clears default assignments, assigns exactly the requested people, sets efforts and reports the parent', async () => {
    const { tp } = world()
    h = await harness({ stub: tp.stub })
    const r = await h.call('write_create_card', {
      type: 'Task',
      name: 'Write the endpoint',
      parent: 36216,
      state: 'Coded',
      teams: ['Core Team'],
      assignees: [{ user: 'me', role: 'Developer' }],
      roleEfforts: [{ role: 'Developer', effort: 1 }],
    })
    expect(r.isError, r.text).toBe(false)
    const create = h.stub.writes[0]!
    expect(create.path).toBe('/api/v1/Tasks')
    expect(create.body).toEqual({
      Name: 'Write the endpoint',
      UserStory: { Id: 36216 },
      Project: { Id: 26080 },
      EntityState: { Id: 693 },
      AssignedTeams: { Items: [{ Team: { Id: 447 } }] },
    })
    expect(r.json.removedDefaultAssignments).toEqual([{ id: expect.any(Number), user: 'Giorgio Verdi (16)', role: 'Product Owner' }])
    expect(r.json.card.assignments).toEqual([{ id: expect.any(Number), user: { id: 2286, name: 'Foxxo Vulpes', login: 'fvulpes' }, role: { id: 13, name: 'Developer' } }])
    expect(r.json.card.roleEfforts.find((e: { role: { id: number } }) => e.role.id === 13).effort).toBe(1)
    expect(r.json.parent).toMatchObject({ id: 36216, roleEfforts: [{ role: 'Developer', before: 20, after: 1 }] })
    expect(r.json.notPersisted).toBeUndefined()
    // Sequential: create, remove default, assign, effort.
    expect(writes(h)).toEqual([
      'POST /api/v1/Tasks',
      expect.stringMatching(/^DELETE \/api\/v1\/Assignments\/\d+$/),
      'POST /api/v1/Assignments',
      expect.stringMatching(/^POST \/api\/v1\/RoleEfforts\/\d+$/),
    ])
  })

  it('keeps a default assignment that was also requested', async () => {
    const { tp } = world()
    h = await harness({ stub: tp.stub })
    const r = await h.call('write_create_card', { type: 'UserStory', name: 'S', parent: 36193, assignees: [{ user: 16, role: 'Product Owner' }] })
    expect(r.json.removedDefaultAssignments).toEqual([])
    expect(writes(h)).toEqual(['POST /api/v1/UserStories'])
  })

  it('fails before posting when no project resolves', async () => {
    const { tp } = world()
    h = await harness({ stub: tp.stub })
    const r = await h.call('write_create_card', { type: 'UserStory', name: 'Orphan' })
    expect(r.isError).toBe(true)
    expect(r.text).toMatch(/no project could be resolved/)
    expect(h.stub.writes).toHaveLength(0)
  })

  it('uses TP_DEFAULT_PROJECT_ID and TP_DEFAULT_TEAM_ID only as fallbacks', async () => {
    const { tp } = world()
    h = await harness({ stub: tp.stub, config: { defaultProjectId: 26080, defaultTeamId: 19523 } })
    await h.call('write_create_card', { type: 'UserStory', name: 'S' })
    expect(h.stub.writes[0]!.body).toEqual({ Name: 'S', Project: { Id: 26080 }, AssignedTeams: { Items: [{ Team: { Id: 19523 } }] } })
  })

  it('never sends an empty team or project reference when defaults are unset', async () => {
    const { tp } = world()
    h = await harness({ stub: tp.stub })
    await h.call('write_create_card', { type: 'Task', name: 'T', parent: 36216 })
    const body = h.stub.writes[0]!.body as Record<string, unknown>
    expect(body).toEqual({ Name: 'T', UserStory: { Id: 36216 }, Project: { Id: 26080 } })
  })

  it('validates dropdown custom fields before posting', async () => {
    const { tp } = world()
    h = await harness({ stub: tp.stub })
    const bad = await h.call('write_create_card', { type: 'UserStory', name: 'S', parent: 36193, customFields: { BackEnd: 'Finished' } })
    expect(bad.isError).toBe(true)
    expect(bad.text).toContain('"Finished" is not an option of BackEnd; options: To Do, Doing, Review, Done')
    expect(h.stub.writes).toHaveLength(0)
    const ok = await h.call('write_create_card', { type: 'UserStory', name: 'S', parent: 36193, customFields: { backend: 'done' } })
    expect(h.stub.writes[0]!.body).toMatchObject({ CustomFields: [{ Name: 'BackEnd', Value: 'Done' }] })
    expect(ok.json.card.customFields.BackEnd).toBe('Done')
  })

  it('refuses a parent of the wrong type', async () => {
    const { tp } = world()
    h = await harness({ stub: tp.stub })
    const r = await h.call('write_create_card', { type: 'Task', name: 'T', parent: 36193 })
    expect(r.text).toMatch(/A Task cannot be created under a Feature. Valid parent types: UserStory/)
  })

  it('reports what was already done when a later step fails', async () => {
    const { tp } = world()
    tp.failNext = { method: 'POST', path: /^\/api\/v1\/Assignments$/, status: 403, message: 'Access denied' }
    h = await harness({ stub: tp.stub })
    const r = await h.call('write_create_card', { type: 'Task', name: 'T', parent: 36216, assignees: [{ user: 'me', role: 'Developer' }] })
    expect(r.isError).toBe(true)
    expect(r.text).toMatch(/^Created Task \d+ but could not assign Foxxo Vulpes as Developer/)
    expect(r.text).toContain('"created":{"id"')
    expect(r.text).toContain('removedDefault')
  })

  it('turns plain-text descriptions into paragraphs and keeps HTML as-is', async () => {
    const { tp } = world()
    h = await harness({ stub: tp.stub })
    await h.call('write_create_card', { type: 'Task', name: 'T', parent: 36216, description: 'a < b\nnext' })
    expect((h.stub.writes[0]!.body as { Description: string }).Description).toBe('<div>a &lt; b</div><div>next</div>')
    await h.call('write_create_card', { type: 'Task', name: 'T', parent: 36216, description: '<p>kept</p>' })
    const second = h.stub.writes.filter((c) => c.path === '/api/v1/Tasks')[1]!
    expect((second.body as { Description: string }).Description).toBe('<p>kept</p>')
  })
})

describe('write_update_card / custom fields / teams', () => {
  it('addTags merges into the list and removeTags reports what went', async () => {
    const { tp, story } = world()
    story.Tags = 'backend, ui'
    h = await harness({ stub: tp.stub })
    const r = await h.call('write_update_card', { id: 36216, addTags: ['API'], removeTags: ['UI'] })
    expect(h.stub.writes[0]!.body).toEqual({ Tags: 'backend,API', Id: 36216 })
    expect(r.json.removedTags).toEqual(['ui'])
  })

  it('refuses to write the card Effort total', async () => {
    const { tp } = world()
    h = await harness({ stub: tp.stub })
    const r = await h.call('write_update_card', { id: 36216, fields: { Effort: 5 } })
    expect(r.text).toMatch(/use write_set_role_effort/)
    expect(h.stub.writes).toHaveLength(0)
  })

  it('write_set_custom_fields clears BackEnd and sets FrontEnd, verified', async () => {
    const { tp } = world()
    h = await harness({ stub: tp.stub })
    const r = await h.call('write_set_custom_fields', { id: 36216, fields: { BackEnd: null, FrontEnd: 'doing' } })
    expect(r.isError, r.text).toBe(false)
    expect(h.stub.writes[0]!.body).toEqual({ CustomFields: [{ Name: 'BackEnd', Value: null }, { Name: 'FrontEnd', Value: 'Doing' }], Id: 36216 })
    expect(r.json.notPersisted).toBeUndefined()
  })

  it('write_team adds and removes exactly the named teams', async () => {
    const { tp } = world()
    h = await harness({ stub: tp.stub })
    const r = await h.call('write_team', { id: 36216, add: ['Pandolfo Team'], remove: ['Core Team'] })
    expect(r.isError, r.text).toBe(false)
    expect(writes(h)).toEqual(['POST /api/v1/UserStories/36216', expect.stringMatching(/^DELETE \/api\/v1\/TeamAssignments\/\d+$/)])
    expect(r.json.teams).toEqual([{ team: 'Pandolfo Team', state: 'Estimated' }])
  })
})

describe('comments, time, relations, follow', () => {
  it('write_comment posts HTML against the card', async () => {
    const { tp } = world()
    tp.stub.post('/api/v1/Comments', { Id: 1, Description: '<div>Done!</div>', General: { Id: 36216, Name: 'x' } })
    h = await harness({ stub: tp.stub })
    const r = await h.call('write_comment', { id: 36216, text: 'Done!' })
    expect(h.stub.writes[0]!.body).toEqual({ General: { Id: 36216 }, Description: '<div>Done!</div>' })
    expect(r.json).toEqual({ comment: 1, card: { id: 36216, name: 'x' } })
  })

  it("write_log_time takes the person's only role on the card", async () => {
    const { tp } = world()
    tp.stub.post('/api/v1/Times', { Id: 9, Spent: 1.5, Remain: 0, Date: '2026-09-23T00:00:00.000Z' })
    h = await harness({ stub: tp.stub })
    const r = await h.call('write_log_time', { id: 36406, spent: 1.5, date: '2026-09-23' })
    expect(r.isError, r.text).toBe(false)
    expect(h.stub.writes[0]!.body).toEqual({ Assignable: { Id: 36406 }, Project: { Id: 26080 }, User: { Id: 2286 }, Role: { Id: 13 }, Spent: 1.5, Date: '2026-09-23' })
  })

  it('write_log_time asks for a role when it is not obvious', async () => {
    const { tp } = world()
    h = await harness({ stub: tp.stub })
    const r = await h.call('write_log_time', { id: 36406, spent: 1, user: 'Giorgio Verdi' })
    expect(r.text).toMatch(/has no role on Task 36406; pass role/)
    expect(h.stub.writes).toHaveLength(0)
  })

  it('write_relate resolves the relation type and skips duplicates', async () => {
    const { tp } = world()
    tp.stub
      .get('/api/v1/RelationTypes', { Items: [{ Id: 1, Name: 'Dependency' }, { Id: 2, Name: 'Blocker' }, { Id: 3, Name: 'Relation' }] })
      .get('/api/v1/Relations', { Items: [] }, { times: 1 })
      .post('/api/v1/Relations', { Id: 8017, Master: { Id: 36216 }, Slave: { Id: 36406 }, RelationType: { Id: 2, Name: 'Blocker' } })
      .get('/api/v1/Relations', { Items: [{ Id: 8017, RelationType: { Id: 2, Name: 'Blocker' } }] })
    h = await harness({ stub: tp.stub })
    const r = await h.call('write_relate', { id: 36216, to: 36406, relation: 'blocker' })
    expect(h.stub.writes[0]!.body).toEqual({ Master: { Id: 36216 }, Slave: { Id: 36406 }, RelationType: { Id: 2 } })
    expect(r.json.type).toBe('Blocker')
    const again = await h.call('write_relate', { id: 36216, to: 36406, relation: 'Blocker' })
    expect(again.json.alreadyRelated).toBe(true)
    expect(h.stub.writes).toHaveLength(1)
  })

  it('write_follow is idempotent', async () => {
    const { tp } = world()
    tp.stub.get('/api/v1/GeneralFollowers', { Items: [{ Id: 5 }] })
    h = await harness({ stub: tp.stub })
    const r = await h.call('write_follow', { id: 36216 })
    expect(r.json).toMatchObject({ following: true, changed: false })
    expect(h.stub.writes).toHaveLength(0)
  })
})

describe('testing', () => {
  it('write_test_cases creates cases with ordered steps under the plan, in its project', async () => {
    const { tp } = world()
    tp.addCard({ Id: 17796, type: 'TestPlan', Name: 'Export' })
    tp.stub.first({ method: 'GET', path: /^\/api\/v1\/TestCases\/\d+$/, body: { Id: 1, TestPlans: { Items: [{ Id: 17796 }] }, TestSteps: { Items: [{ Id: 1 }, { Id: 2 }] } } })
    h = await harness({ stub: tp.stub })
    const r = await h.call('write_test_cases', {
      testPlan: 17796,
      cases: [{ name: 'Exports CSV', steps: [{ step: 'Click export', expected: 'A file downloads' }, { step: 'Open it' }] }],
    })
    expect(r.isError, r.text).toBe(false)
    expect(h.stub.writes[0]!.body).toEqual({
      Name: 'Exports CSV',
      Project: { Id: 26080 },
      TestPlans: { Items: [{ Id: 17796 }] },
      TestSteps: {
        Items: [
          { Description: '<div>Click export</div>', Result: '<div>A file downloads</div>', RunOrder: 1 },
          { Description: '<div>Open it</div>', RunOrder: 2 },
        ],
      },
    })
    expect(r.json.created[0]).toMatchObject({ name: 'Exports CSV', steps: 2 })
    expect(r.json.created[0].notPersisted).toBeUndefined()
  })

  it('write_test_run records results against every generated case run, past the 25-item inner cap', async () => {
    const { tp } = world()
    tp.addCard({ Id: 17796, type: 'TestPlan', Name: 'Export' })
    const caseIds = Array.from({ length: 30 }, (_, i) => i + 1)
    tp.stub
      .get('/api/v1/TestPlans/17796/TestCases', { Items: caseIds.map((Id) => ({ Id })) })
      .post('/api/v1/TestPlanRuns', { Id: 900, Name: 'Export' })
      .get('/api/v1/TestCaseRuns', { Items: caseIds.map((c) => ({ Id: 900 + c, TestCase: { Id: c } })) })
      .post('/api/v1/TestCaseRuns/930', { Id: 930, Status: 'Failed', Comment: 'broken' })
    h = await harness({ stub: tp.stub })
    const r = await h.call('write_test_run', { testPlan: 17796, results: [{ testCase: 30, status: 'Failed', comment: 'broken' }] })
    expect(r.isError, r.text).toBe(false)
    expect(h.stub.writes.map((c) => c.body)).toEqual([{ TestPlan: { Id: 17796 }, Project: { Id: 26080 } }, { Status: 'Failed', Comment: 'broken', Id: 930 }])
    expect(h.stub.find('GET', '/api/v1/TestCaseRuns')[0]!.query.get('where')).toBe('(TestPlanRun.Id eq 900)')
    expect(r.json).toMatchObject({ testPlanRun: { id: 900 }, caseRuns: 30, recorded: [{ testCase: 30, status: 'Failed' }] })
  })

  it('write_test_run refuses test cases outside the plan before starting a run', async () => {
    const { tp } = world()
    tp.addCard({ Id: 17796, type: 'TestPlan', Name: 'Export' })
    tp.stub.get('/api/v1/TestPlans/17796/TestCases', { Items: [{ Id: 1 }] })
    h = await harness({ stub: tp.stub })
    const r = await h.call('write_test_run', { testPlan: 17796, results: [{ testCase: 3, status: 'Passed' }] })
    expect(r.text).toMatch(/test cases 3 are not in test plan 17796; no run was started/)
    expect(h.stub.writes).toHaveLength(0)
  })
})

describe('review regressions', () => {
  it('a failed parent read is reported as unknown, never as a change', async () => {
    const { tp } = world()
    h = await harness({ stub: tp.stub })
    // The story can be read before the write, but not after it.
    let storyReads = 0
    tp.stub.first({
      method: 'GET',
      path: '/api/v1/UserStories/36216',
      body: () => (++storyReads > 1 ? new StubReply(503, { Message: 'Service Unavailable' }) : tp.render(tp.cards.get(36216)!)),
    })
    const r = await h.call('write_set_state', { id: 36406, state: 'Coded' })
    expect(r.isError, r.text).toBe(false)
    expect(r.json.parent.state).toBeUndefined()
    expect(r.json.parent.unknown).toMatch(/could not be read after the write .*Service Unavailable/)
  })

  it('a failed read-back gives a warning and no verdict', async () => {
    const { tp } = world()
    h = await harness({ stub: tp.stub })
    let taskReads = 0
    tp.stub.first({
      method: 'GET',
      path: '/api/v1/Tasks/36406',
      body: () => (++taskReads > 1 ? new StubReply(500, { Message: 'boom' }) : tp.render(tp.cards.get(36406)!)),
    })
    const r = await h.call('write_set_role_effort', { id: 36406, efforts: [{ role: 'Developer', effort: 2 }] })
    expect(r.json.notPersisted).toBeUndefined()
    expect(r.json.warning).toMatch(/could not be read back to verify it .*boom/)
  })

  it('write_create_card refuses rule-bound fields inside fields', async () => {
    const { tp } = world()
    h = await harness({ stub: tp.stub })
    const r = await h.call('write_create_card', { type: 'Task', name: 'T', parent: 36216, fields: { Effort: 5, assignments: [{ Id: 1 }] } })
    expect(r.text).toContain('Effort → use write_set_role_effort')
    expect(r.text).toContain('Assignments → use assignees, or write_assign')
    expect(h.stub.writes).toHaveLength(0)
  })

  it('TP_DEFAULT_TEAM_ID does not block card types without teams', async () => {
    const { tp } = world()
    tp.addCard({ Id: 17796, type: 'TestPlan', Name: 'Export' })
    h = await harness({ stub: tp.stub, config: { defaultTeamId: 447 } })
    const r = await h.call('write_create_card', { type: 'TestCase', name: 'Case', parent: 17796 })
    expect(r.isError, r.text).toBe(false)
    expect(h.stub.writes[0]!.body).toEqual({ Name: 'Case', TestPlans: { Items: [{ Id: 17796 }] }, Project: { Id: 26080 } })
  })

  it('exclusive assign adds before it removes', async () => {
    const { tp } = world()
    h = await harness({ stub: tp.stub })
    await h.call('write_assign', { id: 36216, user: 'Giorgia Rossi', role: 'Developer', exclusive: true })
    expect(writes(h)).toEqual(['POST /api/v1/Assignments', expect.stringMatching(/^DELETE \/api\/v1\/Assignments\/\d+$/)])
  })

  it('exclusive assign removes nobody when the add fails', async () => {
    const { tp } = world()
    tp.failNext = { method: 'POST', path: /^\/api\/v1\/Assignments$/, status: 403, message: 'Access denied' }
    h = await harness({ stub: tp.stub })
    const r = await h.call('write_assign', { id: 36216, user: 'Giorgia Rossi', role: 'Developer', exclusive: true })
    expect(r.text).toMatch(/nobody was removed/)
    expect(h.stub.writes.filter((c) => c.method === 'DELETE')).toHaveLength(0)
  })

  it('tags cannot contain commas', async () => {
    const { tp } = world()
    h = await harness({ stub: tp.stub })
    const r = await h.call('write_update_card', { id: 36216, addTags: ['a,b'] })
    expect(r.isError).toBe(true)
    expect(h.stub.writes).toHaveLength(0)
  })

  it('write_follow reads the follow back', async () => {
    const { tp } = world()
    tp.stub
      .get('/api/v1/GeneralFollowers', { Items: [] }, { times: 1 })
      .post('/api/v1/GeneralFollowers', { Id: 5 })
      .get('/api/v1/GeneralFollowers', { Items: [] })
    h = await harness({ stub: tp.stub })
    const r = await h.call('write_follow', { id: 36216 })
    expect(r.json.notPersisted).toEqual(['following: requested true, got false'])
  })

  it('a write with no response says its outcome is unknown', async () => {
    const { tp } = world()
    tp.stub.first({ method: 'POST', path: '/api/v1/Tasks', networkError: 'The operation was aborted due to timeout' })
    h = await harness({ stub: tp.stub })
    const r = await h.call('write_create_card', { type: 'Task', name: 'T', parent: 36216 })
    expect(r.isError).toBe(true)
    expect(r.text).toMatch(/the POST may or may not have been applied, so check before retrying/)
  })
})

describe('review regressions (write, round 2)', () => {
  it('extra fields outside the card read are read explicitly, or listed as not verified', async () => {
    const { tp } = world()
    tp.stub.first({ method: 'GET', path: '/api/v1/UserStories/36216', query: { include: '[Id,PlannedStartDate]' }, body: { Id: 36216 } })
    h = await harness({ stub: tp.stub })
    const r = await h.call('write_update_card', { id: 36216, fields: { PlannedStartDate: '2026-10-01' } })
    expect(r.isError, r.text).toBe(false)
    expect(r.json.notVerified).toEqual(['PlannedStartDate'])
  })

  it('child card collections in fields are refused', async () => {
    const { tp } = world()
    h = await harness({ stub: tp.stub })
    const r = await h.call('write_create_card', { type: 'UserStory', name: 'S', parent: 36193, fields: { Tasks: [{ Name: 't', Effort: 5 }] } })
    expect(r.text).toContain('Tasks → use write_create_card with parent')
    expect(h.stub.writes).toHaveLength(0)
  })

  it('a create whose read-back fails is still a success, with a warning', async () => {
    const { tp } = world()
    tp.stub.first({ method: 'GET', path: /^\/api\/v1\/Tasks\/5\d{4}$/, body: new StubReply(500, { Message: 'boom' }) })
    h = await harness({ stub: tp.stub })
    const r = await h.call('write_create_card', { type: 'Task', name: 'T', parent: 36216 })
    expect(r.isError).toBe(false)
    expect(r.json.created.id).toEqual(expect.any(Number))
    expect(r.json.warning).toMatch(/could not be read back/)
  })

  it('an inactive person can still be unassigned', async () => {
    const { tp } = world()
    tp.assignments.push({ Id: 777, card: 36216, user: 2429, role: 13 })
    h = await harness({ stub: tp.stub })
    const r = await h.call('write_unassign', { id: 36216, user: 'Rocco Neri' })
    expect(r.isError, r.text).toBe(false)
    expect(r.json.removed.user).toBe('Rocco Neri (2429)')
  })

  it('an inactive person cannot be newly assigned', async () => {
    const { tp } = world()
    h = await harness({ stub: tp.stub })
    const r = await h.call('write_assign', { id: 36216, user: 'Rocco Neri', role: 'Developer' })
    expect(r.text).toMatch(/inactive or deleted/)
  })
})

describe('live-test regressions', () => {
  it('removes a default assignment Targetprocess adds after the first check', async () => {
    const { tp } = world()
    tp.lateDefaults = true
    h = await harness({ stub: tp.stub })
    const r = await h.call('write_create_card', { type: 'Bug', name: 'B', parent: 36216, assignees: [{ user: 'me', role: 'Developer' }] })
    expect(r.isError, r.text).toBe(false)
    expect(r.json.removedDefaultAssignments).toEqual([{ id: expect.any(Number), user: 'Giorgio Verdi (16)', role: 'Product Owner', late: true }])
    expect(r.json.card.assignments.map((a: { user: { id: number } }) => a.user.id)).toEqual([2286])
    expect(r.json.notPersisted).toBeUndefined()
  })

  it('refuses system custom fields before sending', async () => {
    const { tp } = world()
    tp.stub.first({
      method: 'GET',
      path: '/api/v1/CustomFields',
      body: { Items: [{ Id: 159, Name: 'Total Hours', FieldType: 'Number', IsSystem: true, Config: { CalculationModel: '' } }] },
    })
    h = await harness({ stub: tp.stub })
    const r = await h.call('write_set_custom_fields', { id: 36216, fields: { 'Total Hours': 5 } })
    expect(r.text).toMatch(/Total Hours is a system field; only Targetprocess sets it/)
    expect(h.stub.writes).toHaveLength(0)
  })

  it('an exact project name beats the same abbreviation on another project', async () => {
    const { tp } = world()
    tp.stub.first({
      method: 'GET',
      path: '/api/v1/Projects',
      body: { Items: [{ Id: 26080, Name: 'SBP', Abbreviation: 'SBP', IsActive: true, Process: { Id: 13 } }, { Id: 9655, Name: 'SBP - RDA Platform', Abbreviation: 'SBP', IsActive: true, Process: { Id: 13 } }] },
    })
    tp.stub.get('/api/v1/UserStories', { Items: [] })
    h = await harness({ stub: tp.stub })
    const r = await h.call('read_search', { type: 'UserStory', project: 'SBP' })
    expect(r.isError, r.text).toBe(false)
    expect(h.stub.find('GET', '/api/v1/UserStories')[0]!.query.get('where')).toContain('(Project.Id eq 26080)')
  })
})

describe('time records (instances without the Time Tracking practice)', () => {
  function recordWorld() {
    const w = world()
    const records: Record<string, unknown>[] = []
    w.tp.stub
      .first({ method: 'GET', path: /^\/api\/v1\/Processes\/\d+$/, body: { Id: 13, Practices: { Items: [{ Name: 'Bug Tracking' }] } } })
      .first({
        method: 'GET',
        path: '/api/v1/CustomFields',
        query: (q) => (q.get('where') ?? '').includes("'TimeRecord'"),
        body: { Items: [{ Id: 1, Name: 'Hours', FieldType: 'Number', Required: true }, { Id: 2, Name: 'Date', FieldType: 'Date' }] },
      })
      .post('/api/v1/TimeRecords', (call: { body: unknown }) => {
        const b = call.body as Record<string, any>
        const rec = { Id: 900 + records.length, ...b, Name: `${b.Task ? 'Add delivery date' : 'Review and manage orders'} / Foxxo Vulpes / ${b.CustomFields[0].Value}h`, DayPeriod: { Id: 1, Name: '2026-09-23 / Wednesday' } }
        records.push(rec)
        return { Id: rec.Id }
      })
      .get(/^\/api\/v1\/TimeRecords\/\d+$/, (call: { path: string }) => {
        const rec = records.find((r) => r.Id === Number(call.path.split('/').pop()))!
        const cf = (rec.CustomFields as { Name: string; Value: unknown }[]).map((f) => ({ ...f, Value: f.Name === 'Date' ? '/Date(1790114400000+0200)/' : f.Value }))
        return { ...rec, CustomFields: cf }
      })
      .get('/api/v1/TimeRecords', { Items: [{ Id: 7, Name: 'x / Foxxo Vulpes / 0.5h', ConnectedUser: { Id: 2286, FirstName: 'Foxxo', LastName: 'Vulpes' }, UserStory: { Id: 36216, Name: 'Review and manage orders' }, Task: { Id: 36406, Name: 'Add delivery date' }, CustomFields: [{ Name: 'Hours', Value: 0.5 }, { Name: 'Date', Value: '/Date(1790114400000+0200)/' }] }] })
    return { ...w, records }
  }

  it('write_log_time creates a TimeRecord linked to the person, the task and its story', async () => {
    const { tp, records } = recordWorld()
    h = await harness({ stub: tp.stub })
    const r = await h.call('write_log_time', { id: 36406, spent: 0.75, date: '2026-09-23' })
    expect(r.isError, r.text).toBe(false)
    expect(records[0]).toMatchObject({
      Task: { Id: 36406 },
      UserStory: { Id: 36216 },
      ConnectedUser: { Id: 2286 },
      CustomFields: [{ Name: 'Hours', Value: 0.75 }, { Name: 'Date', Value: '2026-09-23' }],
    })
    expect(h.stub.find('POST', '/api/v1/Times')).toHaveLength(0)
    expect(r.json).toMatchObject({ kind: 'TimeRecord', spent: 0.75, date: '2026-09-23', period: '2026-09-23 / Wednesday', name: 'Add delivery date / Foxxo Vulpes / 0.75h' })
    expect(r.json.notPersisted).toBeUndefined()
  })

  it('refuses remaining time, which records do not have', async () => {
    const { tp } = recordWorld()
    h = await harness({ stub: tp.stub })
    const r = await h.call('write_log_time', { id: 36406, spent: 1, remain: 2 })
    expect(r.text).toMatch(/time records have no remaining time/)
    expect(h.stub.writes).toHaveLength(0)
  })

  it('read_times reads the records of a card, filtering dates on the Date field', async () => {
    const { tp } = recordWorld()
    h = await harness({ stub: tp.stub })
    const r = await h.call('read_times', { id: 36406, from: '2026-09-22' })
    expect(r.isError, r.text).toBe(false)
    expect(h.stub.find('GET', '/api/v1/TimeRecords')[0]!.query.get('where')).toBe("(Task.Id eq 36406) and (CustomFields.Date gte '2026-09-22')")
    expect(r.json).toMatchObject({ count: 1, totalSpent: 0.5, entries: [{ kind: 'TimeRecord', spent: 0.5, date: '2026-09-23T00:00:00.000+02:00', card: { id: 36406 } }] })
    expect(h.stub.find('GET', '/api/v1/Times')).toHaveLength(0)
  })

  it('a process without time tracking and no TimeRecord type gives a clear error', async () => {
    const { tp } = world()
    tp.stub.first({ method: 'GET', path: /^\/api\/v1\/Processes\/\d+$/, body: { Id: 13, Practices: { Items: [] } } })
    h = await harness({ stub: tp.stub })
    const r = await h.call('write_log_time', { id: 36406, spent: 1 })
    expect(r.text).toMatch(/Time tracking is off in this card's process .* no TimeRecord type with an Hours field/)
    expect(h.stub.writes).toHaveLength(0)
  })
})

describe('1.0.1: Markdown and rename label', () => {
  it('write_create_card and write_update_card store Markdown with the marker and verify it', async () => {
    const { tp } = world()
    h = await harness({ stub: tp.stub })
    const created = await h.call('write_create_card', { type: 'Task', name: 'T', parent: 36216, description: '## Plan\n\n1. one\n2. two', format: 'markdown' })
    expect(created.isError, created.text).toBe(false)
    expect((h.stub.writes[0]!.body as { Description: string }).Description).toBe('<!--markdown-->## Plan\n\n1. one\n2. two')
    expect(created.json.notPersisted).toBeUndefined()
    expect(created.json.card.description).toBe('## Plan\n\n1. one\n2. two')
    const updated = await h.call('write_update_card', { id: 36216, description: '**bold**', format: 'markdown' })
    expect(updated.json.notPersisted).toBeUndefined()
    expect(h.stub.writes.at(-1)!.body).toMatchObject({ Description: '<!--markdown-->**bold**' })
  })

  it('write_comment accepts format markdown', async () => {
    const { tp } = world()
    tp.stub.post('/api/v1/Comments', (call: { body: unknown }) => ({ Id: 1, ...(call.body as object), General: { Id: 36216, Name: 'x' } }))
    h = await harness({ stub: tp.stub })
    const r = await h.call('write_comment', { id: 36216, text: '- a\n- b', format: 'markdown' })
    expect(h.stub.writes[0]!.body).toEqual({ General: { Id: 36216 }, Description: '<!--markdown-->- a\n- b' })
    expect(r.json.notPersisted).toBeUndefined()
  })

  it('after a rename the result shows the new name and the old one', async () => {
    const { tp } = world()
    h = await harness({ stub: tp.stub })
    const r = await h.call('write_update_card', { id: 36216, name: 'Renamed' })
    expect(r.json.card).toMatchObject({ id: 36216, name: 'Renamed' })
    expect(r.json.renamedFrom).toBe('Review and manage orders')
    expect(r.json.now.name).toBe('Renamed')
  })
})
