import { afterEach, describe, expect, it } from 'vitest'
import { FetchStub } from '../helpers/fetch-stub.js'
import { harness, type Harness } from '../helpers/harness.js'
import { general } from '../fixtures/reference.js'

let h: Harness
afterEach(async () => {
  await h?.close()
})

describe('read_v2_query / read_deleted', () => {
  it('pages v2, asks for ISO dates and repeats includeDeleted on every page', async () => {
    const stub = new FetchStub()
      .get('/api/v2/projects', { next: 'https://tp.example/api/v2/projects?take=1000&skip=1000', items: Array.from({ length: 1000 }, (_, i) => ({ id: i })) }, { query: { skip: '0' } })
      .get('/api/v2/projects', { items: [{ id: 180, name: 'Activity Monitor', deleteDate: '/Date(1393949239000+0100)/' }] }, { query: { skip: '1000' } })
    h = await harness({ stub })
    const r = await h.call('read_deleted', { kind: 'projects', limit: 5000 })
    expect(r.isError, r.text).toBe(false)
    expect(r.json).toMatchObject({ count: 1001, truncated: false })
    expect(r.json.items[1000].deleteDate).toBe('2014-03-04T17:07:19.000+01:00')
    for (const call of h.stub.calls) {
      expect(call.query.get('includeDeleted')).toBe('true')
      expect(call.query.get('isoDate')).toBe('true')
      expect(call.query.get('where')).toBe('(deleteDate!=null)')
    }
  })

  it('returns aggregations as one object', async () => {
    h = await harness({ stub: new FetchStub().get('/api/v2/userStory', { count: 399 }) })
    const r = await h.call('read_v2_query', { entity: 'userStory', result: '{count:count}', where: '(entityState.isFinal==false)' })
    expect(r.json).toEqual({ entity: 'userStory', result: { count: 399 } })
    expect(h.stub.calls[0]!.query.get('result')).toBe('{count:count}')
  })

  it('carries v2 errors', async () => {
    h = await harness({ stub: new FetchStub().get('/api/v2/userStory', { Status: 'BadRequest', Message: "Property 'x' does not exist in 'UserStory'." }, { status: 400 }) })
    const r = await h.call('read_v2_query', { entity: 'userStory', where: '(x==1)' })
    expect(r.isError).toBe(true)
    expect(r.text).toContain("Property 'x' does not exist")
  })
})

describe('read_history', () => {
  it('simple history via the inner History collection', async () => {
    const stub = new FetchStub()
      .get('/api/v1/Generals/36216', general(36216, 'UserStory'))
      .get('/api/v1/UserStories/36216/History', {
        Items: [
          {
            ResourceType: 'UserStorySimpleHistory',
            Id: 230209,
            Date: '/Date(1789125681640+0200)/',
            Effort: 0,
            EntityState: { Id: 681, Name: 'Open' },
            Modifier: { Id: 2286, FirstName: 'Foxxo', LastName: 'Vulpes', Login: 'fvulpes' },
            Project: { Id: 26080, Name: 'SBP' },
            Release: null,
          },
        ],
      })
    h = await harness({ stub })
    const r = await h.call('read_history', { id: 36216 })
    expect(r.json.entries).toEqual([
      { id: 230209, date: '2026-09-11T13:21:21.640+02:00', by: { id: 2286, name: 'Foxxo Vulpes', login: 'fvulpes' }, state: 'Open', effort: 0, project: 'SBP' },
    ])
  })

  it('full history lists changed fields and their values', async () => {
    const stub = new FetchStub()
      .get('/api/v1/Generals/36216', general(36216, 'UserStory'))
      .get('/api/v1/UserStoryHistories', {
        Items: [
          { Id: 1, Date: '/Date(0)/', Modification: 'Update', Changes: 'EntityState,ModifyDate,Name', Name: 'New', EntityState: { Id: 686, Name: 'In Progress' }, IsChangedName: true },
        ],
      })
    h = await harness({ stub })
    const r = await h.call('read_history', { id: 36216, full: true })
    expect(h.stub.find('GET', '/api/v1/UserStoryHistories')[0]!.query.get('where')).toBe('(SourceEntityId eq 36216)')
    expect(r.json.entries[0]).toMatchObject({ modification: 'Update', changed: ['EntityState', 'Name'], values: { EntityState: 'In Progress', Name: 'New' } })
  })

  it('works for non-card resources when named', async () => {
    h = await harness({ stub: new FetchStub().get('/api/v1/CommentHistories', { Items: [] }) })
    const r = await h.call('read_history', { id: 5, resource: 'Comment', full: true })
    expect(r.isError, r.text).toBe(false)
    const simple = await h.call('read_history', { id: 5, resource: 'Comment' })
    expect(simple.text).toMatch(/Comment has no simple history; try full: true/)
  })
})

describe('read_context / read_conversions', () => {
  it('passes ids to Context', async () => {
    h = await harness({ stub: new FetchStub().get('/api/v1/Context', { Version: '2609.2.0.3492' }) })
    await h.call('read_context', { ids: [1, 2], teamIds: [447] })
    const q = h.stub.calls[0]!.query
    expect([q.get('ids'), q.get('teamIds'), q.get('projectIds')]).toEqual(['1,2', '447', null])
  })

  it('reads conversions in both directions', async () => {
    const conv = { Id: 1, FromGeneralID: 6039, FromGeneralType: { Id: 27, Name: 'Epic' }, ActualGeneral: { Id: 6040, Name: 'Blocco', EntityType: { Name: 'Feature' } } }
    h = await harness({
      stub: new FetchStub()
        .get('/api/v1/GeneralConversions', { Items: [] }, { query: { where: '(FromGeneralID eq 6040)' } })
        .get('/api/v1/GeneralConversions', { Items: [conv] }, { query: { where: '(ActualGeneral.Id eq 6040)' } }),
    })
    const r = await h.call('read_conversions', { id: 6040 })
    expect(r.json).toEqual({ id: 6040, convertedTo: [], convertedFrom: [{ from: { id: 6039, type: 'Epic' }, now: { id: 6040, name: 'Blocco', type: 'Feature' } }] })
  })
})

describe('storage', () => {
  it('reads groups, a group and an entry', async () => {
    h = await harness({
      stub: new FetchStub()
        .get('/storage/v1/', { items: ['boards', 'settings'] })
        .get('/storage/v1/boards', { items: [{ key: 'K1', publicData: { name: 'Kanban' } }] })
        .get('/storage/v1/boards/K1', { key: 'K1', scope: 'Public', publicData: { name: 'Kanban' } }),
    })
    expect((await h.call('read_storage')).json.items).toEqual(['boards', 'settings'])
    expect((await h.call('read_storage', { group: 'boards', select: '{key}' })).json).toMatchObject({ group: 'boards', count: 1 })
    expect((await h.call('read_storage', { group: 'boards', key: 'K1' })).json.scope).toBe('Public')
    expect(h.stub.calls[1]!.query.get('select')).toBe('{key}')
  })

  it('write_storage merges and verifies, including null deletions', async () => {
    h = await harness({
      stub: new FetchStub()
        .post('/storage/v1/notes/mine', { key: 'mine' })
        .get('/storage/v1/notes/mine', { key: 'mine', publicData: { color: 'white' }, userData: {} }),
    })
    const r = await h.call('write_storage', { group: 'notes', key: 'mine', publicData: { color: 'white', size: null } })
    expect(h.stub.writes[0]!.body).toEqual({ publicData: { color: 'white', size: null } })
    expect(r.json.notPersisted).toBeUndefined()
  })

  it('rejects path-like group names', async () => {
    h = await harness()
    const r = await h.call('write_storage', { group: '../x', publicData: { a: 1 } })
    expect(r.isError).toBe(true)
    expect(h.stub.calls).toHaveLength(0)
  })

  it('delete_storage reads first', async () => {
    h = await harness({ stub: new FetchStub().get('/storage/v1/notes/k', { Message: 'Not found' }, { status: 404 }) })
    const r = await h.call('delete_storage', { group: 'notes', key: 'k' })
    expect(r.text).toMatch(/nothing was deleted/)
    expect(h.stub.writes).toHaveLength(0)
  })
})

describe('write_attachment', () => {
  it('uploads multipart with generalId and file, then confirms the attachment', async () => {
    h = await harness({
      stub: new FetchStub()
        .post('/UploadFile.ashx', '<html>ok</html>')
        .get('/api/v1/Attachments', { Items: [] }, { times: 1 })
        .get('/api/v1/Attachments', { Items: [{ Id: 77, Name: 'notes.txt', Date: '/Date(0)/' }] }),
    })
    const r = await h.call('write_attachment', { id: 42, files: [{ name: 'notes.txt', contentBase64: Buffer.from('hello').toString('base64'), mimeType: 'text/plain' }] })
    expect(r.isError, r.text).toBe(false)
    const form = h.stub.writes[0]!.body as FormData
    expect(form.get('generalId')).toBe('42')
    const file = form.get('file') as File
    expect(file.name).toBe('notes.txt')
    expect(await file.text()).toBe('hello')
    expect(r.json).toMatchObject({ uploaded: ['notes.txt'], attachments: [{ id: 77, name: 'notes.txt' }] })
    expect(h.stub.writes[0]!.query.get('access_token')).toBeTruthy()
  })

  it('reports an upload that did not arrive', async () => {
    h = await harness({ stub: new FetchStub().post('/UploadFile.ashx', '').get('/api/v1/Attachments', { Items: [] }) })
    const r = await h.call('write_attachment', { id: 42, files: [{ name: 'a.bin', contentBase64: 'AA==' }] })
    expect(r.json.notPersisted).toEqual(['a.bin did not appear as a new attachment'])
  })

  it('an older attachment with the same name does not pass for a failed upload', async () => {
    const old = { Items: [{ Id: 5, Name: 'a.bin', Date: '/Date(0)/' }] }
    h = await harness({ stub: new FetchStub().post('/UploadFile.ashx', '').get('/api/v1/Attachments', old) })
    const r = await h.call('write_attachment', { id: 42, files: [{ name: 'a.bin', contentBase64: 'AA==' }] })
    expect(r.json.notPersisted).toEqual(['a.bin did not appear as a new attachment'])
  })

  it('takes base64 content only, never a local path', async () => {
    h = await harness()
    expect((await h.call('write_attachment', { id: 42, files: [{ name: 'x' }] })).isError).toBe(true)
    expect((await h.call('write_attachment', { id: 42, files: [{ name: 'x', path: '/home/me/.env' }] })).isError).toBe(true)
    expect((await h.call('write_attachment', { id: 42, files: [{ name: 'x', contentBase64: 'not base64!' }] })).text).toMatch(/not valid base64/)
    expect(h.stub.calls).toHaveLength(0)
  })
})

describe('admin_undelete', () => {
  it('posts one item to /undelete and several to /undelete/bulk', async () => {
    h = await harness({ stub: new FetchStub().post('/api/v1/undelete', '').post('/api/v1/undelete/bulk', '') })
    await h.call('admin_undelete', { items: [{ id: 8, entityType: 'UserStory' }] })
    await h.call('admin_undelete', { items: [{ id: 8, entityType: 'UserStory' }, { id: 2, entityType: 'Bug' }] })
    expect(h.stub.writes.map((c) => [c.path, c.body])).toEqual([
      ['/api/v1/undelete', { Id: 8, EntityType: 'UserStory' }],
      ['/api/v1/undelete/bulk', [{ Id: 8, EntityType: 'UserStory' }, { Id: 2, EntityType: 'Bug' }]],
    ])
  })

  it('explains a 403 and refuses entities that cannot be undeleted', async () => {
    h = await harness({ stub: new FetchStub().post('/api/v1/undelete', { Message: 'Access denied' }, { status: 403 }) })
    expect((await h.call('admin_undelete', { items: [{ id: 8, entityType: 'UserStory' }] })).text).toMatch(/^Undelete refused: an administrator token is required/)
    expect((await h.call('admin_undelete', { items: [{ id: 8, entityType: 'Comment' }] })).text).toMatch(/cannot be undeleted/)
  })
})
