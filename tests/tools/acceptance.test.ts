import { afterAll, beforeAll, describe, expect, it } from 'vitest'
import { FakeTp } from '../helpers/fake-tp.js'
import { harness, type Harness } from '../helpers/harness.js'

/**
 * The #36410 batch in one pass, against the stateful fake: the story goes to
 * Ready with two Developers and a Product Owner, role efforts, BackEnd cleared
 * and FrontEnd set; a task in Coded with Core Team, 1h Developer effort and
 * one assignee; a bug assigned to one person; default assignments removed;
 * the story's state change reported. The live twin is tests/live/acceptance.test.ts.
 */
describe('acceptance: the #36410 batch', () => {
  let tp: FakeTp
  let h: Harness
  const story = 36216

  beforeAll(async () => {
    tp = new FakeTp()
    tp.addCard({ Id: 36193, type: 'Feature', Name: 'Review 2025' })
    tp.addCard({ Id: story, type: 'UserStory', Name: 'Acceptance story', EntityState: { Id: 683, Name: 'Estimated' }, parent: { member: 'Feature', id: 36193 } }, { teams: [447] })
    h = await harness({ stub: tp.stub })
  })
  afterAll(async () => {
    await h.close()
  })

  it('story: state Ready', async () => {
    const r = await h.call('write_set_state', { id: story, state: 'Ready' })
    expect(r.json.state).toMatchObject({ after: 'Ready', changed: true })
  })

  it('story: two Developers and a Product Owner', async () => {
    for (const [user, role] of [['me', 'Developer'], ['Giorgia Rossi', 'Developer'], ['Giorgio Marchetti', 'Product Owner']]) {
      const r = await h.call('write_assign', { id: story, user, role })
      expect(r.isError, r.text).toBe(false)
      expect(r.json.removed).toEqual([])
    }
    const card = await h.call('read_card', { id: story })
    expect(card.json.assignments.map((a: { user: { id: number }; role: { name: string } }) => `${a.user.id}:${a.role.name}`).sort()).toEqual([
      '16:Product Owner',
      '17:Developer',
      '2286:Developer',
    ])
  })

  it('story: role efforts', async () => {
    const r = await h.call('write_set_role_effort', { id: story, efforts: [{ role: 'Developer', effort: 16 }, { role: 'Product Owner', effort: 2 }] })
    expect(r.isError, r.text).toBe(false)
    expect(r.json.notPersisted).toBeUndefined()
    expect(r.json.total.after).toBe(18)
  })

  it('story: clear BackEnd, set FrontEnd', async () => {
    const r = await h.call('write_set_custom_fields', { id: story, fields: { BackEnd: null, FrontEnd: 'Doing' } })
    expect(r.isError, r.text).toBe(false)
    const card = await h.call('read_card', { id: story })
    expect(card.json.customFields).toEqual({ BackEnd: null, FrontEnd: 'Doing' })
  })

  it('task in Coded with Core Team, 1h Developer effort and one assignee; story state change reported', async () => {
    const r = await h.call('write_create_card', {
      type: 'Task',
      parent: story,
      name: 'Acceptance task',
      state: 'Coded',
      teams: ['Core Team'],
      roleEfforts: [{ role: 'Developer', effort: 1 }],
      assignees: [{ user: 'me', role: 'Developer' }],
    })
    expect(r.isError, r.text).toBe(false)
    expect(r.json.notPersisted).toBeUndefined()
    expect(r.json.removedDefaultAssignments).toHaveLength(1)
    expect(r.json.card).toMatchObject({ state: { name: 'Coded' }, teams: [{ team: { name: 'Core Team' } }] })
    expect(r.json.card.assignments).toHaveLength(1)
    // The rollup overwrote the story's Developer effort (16 → 1): reported, not hidden.
    expect(r.json.parent.roleEfforts).toEqual([{ role: 'Developer', before: 16, after: 1 }])
  })

  it('bug assigned to one person, defaults removed', async () => {
    const r = await h.call('write_create_card', { type: 'Bug', parent: story, name: 'Acceptance bug', assignees: [{ user: 'Giorgia Rossi', role: 'Developer' }] })
    expect(r.isError, r.text).toBe(false)
    expect(r.json.removedDefaultAssignments).toEqual([{ id: expect.any(Number), user: 'Giorgio Marchetti (16)', role: 'Product Owner' }])
    expect(r.json.card.assignments.map((a: { user: { id: number } }) => a.user.id)).toEqual([17])
    expect(r.json.notPersisted).toBeUndefined()
  })

  it('moving the task out of its initial state reports the story advancing', async () => {
    const task = await h.call('write_create_card', { type: 'Task', parent: story, name: 'Second task' })
    const r = await h.call('write_set_state', { id: task.json.created.id, state: 'In Progress' })
    expect(r.json.parent).toMatchObject({ id: story, state: { before: 'Ready', after: 'In Progress', changed: true } })
  })
})
