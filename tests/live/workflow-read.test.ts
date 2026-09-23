import { afterAll, beforeAll, describe, expect, it } from 'vitest'
import { LIVE, liveClient } from './helpers.js'

type Live = Awaited<ReturnType<typeof liveClient>>

// Read-only workflow tool checks against the configured instance. No writes.
describe.runIf(LIVE)('live: workflow reads', () => {
  let c: Live
  let storyId: number
  let taskId: number | undefined

  beforeAll(async () => {
    c = await liveClient()
    const mine = await c.call('read_my_work', { type: 'UserStory', limit: 5 })
    expect(mine.isError, mine.text).toBe(false)
    const any = mine.json.cards[0] ?? (await c.call('read_search', { type: 'UserStory', limit: 1, includeClosed: true })).json.cards[0]
    storyId = any.id
    const tasks = await c.call('read_collection', { resource: 'UserStory', id: storyId, collection: 'Tasks', include: 'Id', limit: 1 })
    taskId = tasks.json.items[0]?.Id
  })
  afterAll(async () => {
    await c.close()
  })

  it('read_card returns assignments, role efforts and the parent chain', async () => {
    const r = await c.call('read_card', { id: taskId ?? storyId })
    expect(r.isError, r.text).toBe(false)
    expect(Array.isArray(r.json.assignments)).toBe(true)
    expect(Array.isArray(r.json.roleEfforts)).toBe(true)
    if (taskId) expect(r.json.parents[0].type).toBe('UserStory')
  })

  it('read_states lists the card workflow', async () => {
    const r = await c.call('read_states', { id: storyId })
    expect(r.isError, r.text).toBe(false)
    expect(r.json.states.length).toBeGreaterThan(1)
    expect(r.json.current.name).toBeTruthy()
  })

  it('reference data tools answer', async () => {
    for (const [name, args] of [
      ['read_people', { query: 'a', limit: 3 }],
      ['read_teams', {}],
      ['read_roles', {}],
      ['read_projects', {}],
      ['read_releases', { limit: 2 }],
      ['read_iterations', {}],
      ['read_custom_field_options', { id: storyId }],
      ['read_comments', { id: storyId }],
      ['read_relations', { id: storyId }],
      ['read_times', { user: 'me', limit: 3 }],
      ['read_attachments', { id: storyId }],
    ] as const) {
      const r = await c.call(name, args)
      expect(r.isError, `${name}: ${r.text}`).toBe(false)
    }
  })

  it('read_test_plan reads a plan with cases and steps', async () => {
    const plans = await c.call('read_query', { resource: 'TestPlan', include: 'Id', orderBy: 'Id desc', limit: 1 })
    const planId = plans.json.items[0]?.Id
    if (!planId) return
    const r = await c.call('read_test_plan', { id: planId })
    expect(r.isError, r.text).toBe(false)
    expect(Array.isArray(r.json.testCases)).toBe(true)
  })
})
