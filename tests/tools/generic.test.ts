import { afterEach, describe, expect, it } from 'vitest'
import { FetchStub } from '../helpers/fetch-stub.js'
import { harness, type Harness } from '../helpers/harness.js'

let h: Harness
afterEach(async () => {
  await h?.close()
})

// A real v1 story as returned by our instance (names shortened).
const story = {
  ResourceType: 'UserStory',
  Id: 36217,
  Name: 'Validate the complete desktop workflow',
  CreateDate: '/Date(1789125687000+0200)/',
  Project: { ResourceType: 'Project', Id: 26080, Name: 'SBP', Process: { ResourceType: 'Process', Id: 13 } },
  EntityState: { ResourceType: 'EntityState', Id: 683, Name: 'Estimated', NumericPriority: 3.90625 },
}

describe('read_meta', () => {
  it('lists every resource with its operations', async () => {
    h = await harness()
    const r = await h.call('read_meta')
    expect(r.isError).toBe(false)
    expect(r.json.source).toBe('snapshot')
    const us = r.json.resources.find((x: { name: string }) => x.name === 'UserStory')
    expect(us).toEqual({ name: 'UserStory', path: 'UserStories', operations: ['create', 'update', 'delete'] })
    expect(r.json.resources.find((x: { name: string }) => x.name === 'SickLeave').operations).toBe('unavailable')
    expect(r.json.resources.find((x: { name: string }) => x.name === 'Project').admin).toBe(true)
    expect(h.stub.calls).toHaveLength(0)
  })

  it('describes one resource', async () => {
    h = await harness()
    const r = await h.call('read_meta', { resource: 'userstories' })
    expect(r.json.name).toBe('UserStory')
    expect(r.json.values.find((f: { name: string }) => f.name === 'Name')).toMatchObject({ set: true, required: true })
    expect(r.json.collections.find((c: { name: string }) => c.name === 'Assignments')).toMatchObject({ add: true, remove: true })
  })

  it('rejects an unknown resource with suggestions', async () => {
    h = await harness()
    const r = await h.call('read_meta', { resource: 'UserStorie' })
    expect(r.isError).toBe(true)
    expect(r.text).toMatch(/^Not sent: Unknown resource "UserStorie". Did you mean: UserStory/)
  })
})

describe('read_get / read_query / read_collection', () => {
  it('read_get sends include in brackets and innerTake, and returns ISO dates', async () => {
    h = await harness({ stub: new FetchStub().get('/api/v1/UserStories/36217', story) })
    const r = await h.call('read_get', { resource: 'UserStory', id: 36217, include: 'Id,Name,Project' })
    expect(r.isError).toBe(false)
    expect(r.json.CreateDate).toBe('2026-09-11T13:21:27.000+02:00')
    const q = h.stub.calls[0]!.query
    expect(q.get('include')).toBe('[Id,Name,Project]')
    expect(q.get('innerTake')).toBe('1000')
  })

  it('read_get carries the TP error', async () => {
    h = await harness({
      stub: new FetchStub().get(
        '/api/v1/UserStories/1',
        { Status: 'NotFound', Message: 'UserStory with Id 1 not found or access is forbidden.' },
        { status: 404 },
      ),
    })
    const r = await h.call('read_get', { resource: 'UserStory', id: 1 })
    expect(r.isError).toBe(true)
    expect(r.text.split('\n')[0]).toBe('Could not read UserStory 1')
    expect(r.text).toContain('status: 404')
    expect(r.text).toContain('UserStory with Id 1 not found')
  })

  it('read_query pages, converts "desc" and reports truncated', async () => {
    h = await harness({
      stub: new FetchStub().get('/api/v1/Bugs', { Next: 'x', Items: [{ Id: 1 }, { Id: 2 }] }),
    })
    const r = await h.call('read_query', { resource: 'Bug', where: "(EntityState.Name eq 'Open')", orderBy: 'CreateDate desc', limit: 2 })
    expect(r.json).toEqual({ resource: 'Bug', count: 2, truncated: true, items: [{ Id: 1 }, { Id: 2 }] })
    const q = h.stub.calls[0]!.query
    expect(q.get('orderByDesc')).toBe('CreateDate')
    expect(q.get('orderBy')).toBeNull()
    expect(q.get('take')).toBe('2')
  })

  it('read_collection validates the collection name', async () => {
    h = await harness()
    const r = await h.call('read_collection', { resource: 'UserStory', id: 5, collection: 'Taks' })
    expect(r.isError).toBe(true)
    expect(r.text).toMatch(/Did you mean: Tasks/)
    expect(h.stub.calls).toHaveLength(0)
  })

  it('read_collection reads the inner collection path', async () => {
    h = await harness({ stub: new FetchStub().get('/api/v1/UserStories/5/Tasks', { Items: [{ Id: 9, Name: 't' }] }) })
    const r = await h.call('read_collection', { resource: 'UserStories', id: 5, collection: 'tasks' })
    expect(r.json).toMatchObject({ collection: 'Tasks', count: 1, truncated: false })
  })
})

describe('write_create / write_update payloads', () => {
  it('sends exactly the requested fields with canonical names', async () => {
    h = await harness({ stub: new FetchStub().post('/api/v1/UserStories', { ...story, Name: 'New' }, { status: 201 }) })
    const r = await h.call('write_create', {
      resource: 'UserStory',
      fields: { name: 'New', project: { Id: '26080' } },
    })
    expect(r.isError).toBe(false)
    expect(h.stub.writes).toHaveLength(1)
    expect(h.stub.writes[0]!.body).toEqual({ Name: 'New', Project: { Id: 26080 } })
    expect(r.json.id).toBe(36217)
  })

  it('never sends an empty reference', async () => {
    h = await harness()
    const r = await h.call('write_create', { resource: 'UserStory', fields: { Name: 'x', Project: { Id: '' } } })
    expect(r.isError).toBe(true)
    expect(r.text).toMatch(/Project must be a reference/)
    expect(h.stub.calls).toHaveLength(0)
  })

  it('rejects Id on create, unknown and read-only fields, listing valid options', async () => {
    h = await harness()
    const r = await h.call('write_create', {
      resource: 'UserStory',
      fields: { Id: 1, Nmae: 'x', CreateDate: '2026-01-01', MyCustom: 'v' },
    })
    expect(r.isError).toBe(true)
    expect(r.text).toContain('remove Id when creating')
    expect(r.text).toContain('Did you mean: Name')
    expect(r.text).toContain('CreateDate is read-only')
    expect(r.text).toContain('Custom fields go in "CustomFields"')
    expect(r.text).toMatch(/Settable on UserStory: .*Name/)
    expect(h.stub.calls).toHaveLength(0)
  })

  it('validates nested children against their own resource', async () => {
    h = await harness()
    const r = await h.call('write_create', {
      resource: 'UserStory',
      fields: { Name: 'x', Project: { Id: 2 }, Tasks: { Items: [{ Name: 't', Bogus: 1 }] } },
    })
    expect(r.isError).toBe(true)
    expect(r.text).toContain('Tasks[0].Bogus: Task has no field "Bogus"')
  })

  it('wraps nested collections in Items and custom fields as an array', async () => {
    h = await harness({ stub: new FetchStub().post('/api/v1/UserStories', story, { status: 201 }) })
    await h.call('write_create', {
      resource: 'UserStory',
      fields: {
        Name: 'x',
        Project: { Id: 2 },
        Tasks: [{ Name: 't' }],
        CustomFields: [{ Name: 'FrontEnd', Value: 'To Do' }],
      },
    })
    expect(h.stub.writes[0]!.body).toEqual({
      Name: 'x',
      Project: { Id: 2 },
      Tasks: { Items: [{ Name: 't' }] },
      CustomFields: [{ Name: 'FrontEnd', Value: 'To Do' }],
    })
  })

  it('update posts to the entity with its Id and reports values that did not persist', async () => {
    h = await harness({ stub: new FetchStub().post('/api/v1/UserStories/36217', { ...story, Name: 'Old name' }) })
    const r = await h.call('write_update', { resource: 'UserStory', id: 36217, fields: { Name: 'New name' } })
    expect(h.stub.writes[0]!.body).toEqual({ Name: 'New name', Id: 36217 })
    expect(r.json.notPersisted).toEqual(['Name: requested "New name", got "Old name"'])
  })

  it('refuses administration resources and points at admin_*', async () => {
    h = await harness()
    const r = await h.call('write_update', { resource: 'Project', id: 2, fields: { Name: 'x' } })
    expect(r.isError).toBe(true)
    expect(r.text).toMatch(/use admin_update instead/)
    const r2 = await h.call('admin_update', { resource: 'UserStory', id: 2, fields: { Name: 'x' } })
    expect(r2.text).toMatch(/use write_update instead/)
    expect(h.stub.calls).toHaveLength(0)
  })

  it('refuses operations the resource does not support', async () => {
    h = await harness()
    const r = await h.call('write_create', { resource: 'EntityType', fields: { Name: 'x' } })
    expect(r.text).toMatch(/EntityType does not support create/)
  })

  it('refuses Context writes (read-only in practice)', async () => {
    h = await harness()
    const r = await h.call('write_create', { resource: 'Context', fields: { Acid: 'x' } })
    expect(r.text).toMatch(/read-only in practice/)
  })
})

describe('write_bulk', () => {
  it('validates every item before sending anything', async () => {
    h = await harness()
    const r = await h.call('write_bulk', { resource: 'Task', items: [{ Name: 'ok', UserStory: { Id: 1 } }, { Id: 5, Nope: 1 }] })
    expect(r.isError).toBe(true)
    expect(r.text).toContain('items[1].Nope')
    expect(h.stub.calls).toHaveLength(0)
  })

  it('posts creates and updates to /bulk', async () => {
    h = await harness({ stub: new FetchStub().post('/api/v1/Tasks/bulk', '') })
    const r = await h.call('write_bulk', { resource: 'Task', items: [{ Name: 'a', UserStory: { Id: 1 } }, { Id: '5', Name: 'b' }] })
    expect(r.isError).toBe(false)
    expect(h.stub.writes[0]!.body).toEqual([{ Name: 'a', UserStory: { Id: 1 } }, { Id: 5, Name: 'b' }])
  })

  it('caps at 500 items', async () => {
    h = await harness()
    const r = await h.call('write_bulk', { resource: 'Task', items: Array.from({ length: 501 }, () => ({ Name: 'x' })) })
    expect(r.isError).toBe(true)
    expect(h.stub.calls).toHaveLength(0)
  })
})

describe('collections', () => {
  it('write_collection_add appends with {Items}', async () => {
    h = await harness({ stub: new FetchStub().post('/api/v1/Builds/7', { Id: 7 }) })
    const r = await h.call('write_collection_add', { resource: 'Build', id: 7, collection: 'bugs', items: [{ Id: 42 }] })
    expect(r.isError).toBe(false)
    expect(h.stub.writes[0]!.body).toEqual({ Bugs: { Items: [{ Id: 42 }] }, Id: 7 })
  })

  it('TagObjects are posted as a plain array', async () => {
    h = await harness({ stub: new FetchStub().post('/api/v1/Bugs/7', { Id: 7 }) })
    await h.call('write_collection_add', { resource: 'Bug', id: 7, collection: 'TagObjects', items: [{ Name: 'ui' }] })
    expect(h.stub.writes[0]!.body).toEqual({ TagObjects: [{ Name: 'ui' }], Id: 7 })
  })

  it('refuses read-only collections', async () => {
    h = await harness()
    const r = await h.call('write_collection_add', { resource: 'UserStory', id: 7, collection: 'Bugs', items: [{ Id: 1 }] })
    expect(r.text).toMatch(/cannot be added to UserStory.Bugs/)
  })

  it('delete_collection_remove deletes each child by path', async () => {
    h = await harness({ stub: new FetchStub().delete(/^\/api\/v1\/TestPlans\/234\/TestCases\/\d+$/, '') })
    const r = await h.call('delete_collection_remove', { resource: 'TestPlan', id: 234, collection: 'TestCases', childIds: [1234, 1235] })
    expect(r.isError).toBe(false)
    expect(h.stub.calls.map((c) => c.path)).toEqual(['/api/v1/TestPlans/234/TestCases/1234', '/api/v1/TestPlans/234/TestCases/1235'])
  })
})

describe('delete tier', () => {
  it('delete_entity reads first and reports what was deleted', async () => {
    h = await harness({
      stub: new FetchStub().get('/api/v1/Comments/5', { Id: 5, Description: 'x' }).delete('/api/v1/Comments/5', ''),
    })
    const r = await h.call('delete_entity', { resource: 'Comment', id: 5 })
    expect(r.json).toEqual({ deleted: 'Comment', id: 5 })
    expect(h.stub.writes.map((c) => c.method)).toEqual(['DELETE'])
  })

  it('delete_entity sends nothing when the entity cannot be read', async () => {
    h = await harness({ stub: new FetchStub().get('/api/v1/Comments/5', { Message: 'not found' }, { status: 404 }) })
    const r = await h.call('delete_entity', { resource: 'Comment', id: 5 })
    expect(r.isError).toBe(true)
    expect(r.text).toMatch(/nothing was deleted/)
    expect(h.stub.writes).toHaveLength(0)
  })

  it('delete_bulk sends ids', async () => {
    h = await harness({ stub: new FetchStub().delete('/api/v1/Tags/bulk', '') })
    await h.call('delete_bulk', { resource: 'Tag', ids: [14, 56] })
    expect(h.stub.writes[0]!.body).toEqual([{ Id: 14 }, { Id: 56 }])
  })
})
