import { afterEach, describe, expect, it } from 'vitest'
import { FakeTp } from '../helpers/fake-tp.js'
import { harness, type Harness } from '../helpers/harness.js'

let h: Harness
afterEach(async () => {
  await h?.close()
})

describe('delete_card', () => {
  it('resolves the type, deletes, confirms it is gone and reports the parent', async () => {
    const tp = new FakeTp()
    tp.addCard({ Id: 36216, type: 'UserStory', Name: 'Story', EntityState: { Id: 686, Name: 'In Progress' } }, { efforts: { 13: 8 } })
    tp.addCard({ Id: 36406, type: 'Task', Name: 'Task', parent: { member: 'UserStory', id: 36216 } }, { efforts: { 13: 8 } })
    h = await harness({ stub: tp.stub })
    const r = await h.call('delete_card', { id: 36406 })
    expect(r.isError, r.text).toBe(false)
    expect(h.stub.writes.map((c) => `${c.method} ${c.path}`)).toEqual(['DELETE /api/v1/Tasks/36406'])
    expect(r.json.deleted).toMatchObject({ id: 36406, type: 'Task', name: 'Task', state: 'Open' })
    expect(r.json.parent).toMatchObject({ id: 36216, state: { before: 'In Progress', after: 'In Progress', changed: false } })
    expect(r.json.notPersisted).toBeUndefined()
  })

  it('refuses administration resources', async () => {
    const tp = new FakeTp()
    tp.stub.first({ method: 'GET', path: '/api/v1/Generals/2', body: { Id: 2, Name: 'P', EntityType: { Id: 1, Name: 'Project' }, Project: null } })
    tp.stub.first({ method: 'GET', path: '/api/v1/Projects/2', body: { Id: 2, Name: 'P' } })
    h = await harness({ stub: tp.stub })
    const r = await h.call('delete_card', { id: 2 })
    expect(r.text).toMatch(/use admin_delete instead/)
    expect(h.stub.writes).toHaveLength(0)
  })
})

describe('delete_relation', () => {
  const rel = (Id: number, type: string) => ({ Id, RelationType: { Id: 1, Name: type }, Master: { Id: 1, Name: 'a' }, Slave: { Id: 2, Name: 'b' } })

  it('finds the relation in either direction and deletes it', async () => {
    const tp = new FakeTp()
    tp.stub
      .get('/api/v1/Relations', { Items: [] }, { query: { where: '(Master.Id eq 2) and (Slave.Id eq 1)' } })
      .get('/api/v1/Relations', { Items: [rel(7, 'Blocker')] })
      .delete('/api/v1/Relations/7', '')
    h = await harness({ stub: tp.stub })
    const r = await h.call('delete_relation', { id: 2, to: 1 })
    expect(r.json.deleted).toMatchObject({ id: 7, type: 'Blocker' })
  })

  it('asks for the type when there are several', async () => {
    const tp = new FakeTp()
    tp.stub.get('/api/v1/Relations', { Items: [rel(7, 'Blocker'), rel(8, 'Link')] })
    h = await harness({ stub: tp.stub })
    const r = await h.call('delete_relation', { id: 1, to: 2 })
    expect(r.text).toMatch(/relations between 1 and 2 .*pass relation/)
    expect(h.stub.writes).toHaveLength(0)
  })
})
