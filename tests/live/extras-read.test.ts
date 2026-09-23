import { afterAll, beforeAll, describe, expect, it } from 'vitest'
import { LIVE, liveClient } from './helpers.js'

type Live = Awaited<ReturnType<typeof liveClient>>

// Read-only checks of v2, history, context, conversions, deleted items and storage. No writes.
describe.runIf(LIVE)('live: v2, history, context, conversions, storage', () => {
  let c: Live
  let storyId: number
  beforeAll(async () => {
    c = await liveClient()
    const q = await c.call('read_query', { resource: 'UserStory', include: 'Id', orderBy: 'CreateDate desc', limit: 1 })
    storyId = q.json.items[0].Id
  })
  afterAll(async () => {
    await c.close()
  })

  it('v2 query and aggregation', async () => {
    const q = await c.call('read_v2_query', { entity: 'userStory', select: '{id,name,state:entityState.name}', where: `(id==${storyId})` })
    expect(q.isError, q.text).toBe(false)
    expect(q.json.items[0].id).toBe(storyId)
    const agg = await c.call('read_v2_query', { entity: 'userStory', result: '{count:count}', where: '(entityState.isFinal==false)' })
    expect(agg.isError, agg.text).toBe(false)
    expect(typeof agg.json.result.count).toBe('number')
  })

  it('simple and full history of a card', async () => {
    const simple = await c.call('read_history', { id: storyId })
    expect(simple.isError, simple.text).toBe(false)
    expect(simple.json.kind).toBe('simple')
    const full = await c.call('read_history', { id: storyId, full: true, limit: 5 })
    expect(full.isError, full.text).toBe(false)
    expect(full.json.entries[0].date).toMatch(/^\d{4}-/)
  })

  it('context, conversions and deleted projects', async () => {
    const ctx = await c.call('read_context')
    expect(ctx.isError, ctx.text).toBe(false)
    expect(ctx.json.Version).toBeTruthy()
    const conv = await c.call('read_conversions', { id: 6040 })
    expect(conv.isError, conv.text).toBe(false)
    const deleted = await c.call('read_deleted', { kind: 'projects', limit: 2 })
    expect(deleted.isError, deleted.text).toBe(false)
    expect(deleted.json.items[0].deleteDate).toMatch(/^\d{4}-/)
  })

  it('storage groups, a group and one entry', async () => {
    const groups = await c.call('read_storage')
    expect(groups.isError, groups.text).toBe(false)
    expect(groups.json.items).toContain('boards')
    const boards = await c.call('read_storage', { group: 'boards', select: '{key,publicData.name}', limit: 2 })
    expect(boards.isError, boards.text).toBe(false)
    const key = boards.json.items[0].key
    const one = await c.call('read_storage', { group: 'boards', key })
    expect(one.isError, one.text).toBe(false)
    expect(one.json.key).toBe(key)
  })
})
