import { afterAll, beforeAll, describe, expect, it } from 'vitest'
import { LIVE, liveClient } from './helpers.js'

type Live = Awaited<ReturnType<typeof liveClient>>

// Read-only checks against the configured instance. No writes.
describe.runIf(LIVE)('live: layer 1 reads', () => {
  let c: Live
  beforeAll(async () => {
    c = await liveClient()
  })
  afterAll(async () => {
    await c.close()
  })

  it('loads the live catalog', async () => {
    const r = await c.call('read_meta')
    expect(r.isError, r.text).toBe(false)
    expect(r.json.source).toBe('live')
    expect(r.json.count).toBeGreaterThan(100)
  })

  it('queries with include, orderBy desc and a limit', async () => {
    const r = await c.call('read_query', { resource: 'UserStory', include: 'Id,Name,CreateDate', orderBy: 'CreateDate desc', limit: 3 })
    expect(r.isError, r.text).toBe(false)
    expect(r.json.count).toBe(3)
    expect(r.json.truncated).toBe(true)
    expect(r.json.items[0].CreateDate).toMatch(/^\d{4}-\d{2}-\d{2}T/)
  })

  it('reads one entity and an inner collection', async () => {
    const q = await c.call('read_query', { resource: 'UserStory', include: 'Id', orderBy: 'CreateDate desc', limit: 1 })
    const id = q.json.items[0].Id as number
    const one = await c.call('read_get', { resource: 'UserStory', id, include: 'Id,Name,Assignments[Id,Role,GeneralUser]' })
    expect(one.isError, one.text).toBe(false)
    expect(one.json.Id).toBe(id)
    const tasks = await c.call('read_collection', { resource: 'UserStory', id, collection: 'Tasks', include: 'Id,Name' })
    expect(tasks.isError, tasks.text).toBe(false)
  })

  it('surfaces the Targetprocess error for a bad filter', async () => {
    const r = await c.call('read_query', { resource: 'UserStory', where: 'Nonsense eq', limit: 1 })
    expect(r.isError).toBe(true)
    expect(r.text).toMatch(/status: 400/)
  })
})
