import { describe, expect, it } from 'vitest'
import { HttpCore } from '../src/http/core.js'
import { redact } from '../src/http/redact.js'
import { V1Client, innerItems } from '../src/http/v1.js'
import { normalizeV1Dates, v1DateToIso } from '../src/format/dates.js'
import { FetchStub } from './helpers/fetch-stub.js'

const TOKEN = 'SECRETTOKEN123=='

function client(stub: FetchStub, log: string[] = []) {
  const http = new HttpCore({ baseUrl: 'https://tp.example', token: TOKEN, fetch: stub.fetch, log: (l) => log.push(l) })
  return new V1Client(http)
}

describe('redact', () => {
  it('removes the token from URLs and bodies', () => {
    expect(redact(`https://x/api?format=json&access_token=${TOKEN}&take=1`, TOKEN)).toBe(
      'https://x/api?format=json&access_token=***&take=1',
    )
    expect(redact(`echo ${TOKEN} and ${encodeURIComponent(TOKEN)}`, TOKEN)).toBe('echo *** and ***')
  })
})

describe('dates', () => {
  it('converts v1 dates to ISO 8601', () => {
    expect(v1DateToIso('/Date(1789125687000+0200)/')).toBe('2026-09-11T13:21:27.000+02:00')
    // Local-midnight dates keep their calendar day.
    expect(v1DateToIso('/Date(1789682400000+0200)/')).toBe('2026-09-18T00:00:00.000+02:00')
    expect(Date.parse(v1DateToIso('/Date(1789682400000+0200)/'))).toBe(1789682400000)
    expect(v1DateToIso('not a date')).toBe('not a date')
    expect(normalizeV1Dates({ a: ['/Date(0)/'], b: { c: '/Date(0+0000)/' }, d: 1 })).toEqual({
      a: ['1970-01-01T00:00:00.000Z'],
      b: { c: '1970-01-01T00:00:00.000+00:00' },
      d: 1,
    })
  })
})

describe('HttpCore', () => {
  it('sends the token, format=json and Accept, and logs a redacted line', async () => {
    const stub = new FetchStub().get('/api/v1/Users/LoggedUser', { Id: 2286 })
    const log: string[] = []
    const result = await client(stub, log).getPath('Users/LoggedUser')
    expect(result).toEqual({ ok: true, status: 200, data: { Id: 2286 } })
    const call = stub.calls[0]!
    expect(call.query.get('access_token')).toBe(TOKEN)
    expect(call.query.get('format')).toBe('json')
    expect(call.headers.Accept).toBe('application/json')
    expect(log).toEqual(['GET https://tp.example/api/v1/Users/LoggedUser?format=json&access_token=***'])
    expect(log.join()).not.toContain(TOKEN)
  })

  it("carries Targetprocess's JSON error message and the raw body", async () => {
    const body = {
      Status: 'NotFound',
      Message: 'General with Id 36410 not found or access is forbidden.',
      Type: 'Presentational',
      ErrorId: 'e2a27ff8',
    }
    const stub = new FetchStub().get('/api/v1/Generals/36410', body, { status: 404 })
    const result = await client(stub).get('Generals', 36410)
    expect(result.ok).toBe(false)
    if (result.ok) return
    expect(result.status).toBe(404)
    expect(result.message).toBe('404: General with Id 36410 not found or access is forbidden.')
    expect(JSON.parse(result.body)).toEqual(body)
  })

  it('reads the message out of an XML error', async () => {
    const xml = '<Error>\n  <Status>BadRequest</Status>\n  <Message>Error during parameters parsing.</Message>\n</Error>'
    const stub = new FetchStub().get('/api/v1/UserStories', xml, { status: 400 })
    const result = await client(stub).list('UserStories')
    expect(result.ok || result.message).toBe('400: Error during parameters parsing.')
  })

  it('redacts the token from error bodies', async () => {
    const stub = new FetchStub().get('/api/v1/Bugs/1', `bad token ${TOKEN}`, { status: 401 })
    const result = await client(stub).get('Bugs', 1)
    expect(result.ok).toBe(false)
    if (!result.ok) {
      expect(result.body).toBe('bad token ***')
      expect(result.message).not.toContain(TOKEN)
    }
  })

  it('reports network failures with status 0', async () => {
    const stub = new FetchStub().on({ path: '/api/v1/Bugs/1', networkError: 'fetch failed' })
    const result = await client(stub).get('Bugs', 1)
    expect(result).toMatchObject({ ok: false, status: 0 })
    expect(result.ok || result.message).toMatch(/No response from Targetprocess/)
  })

  it('treats a redirect as a failure instead of following it', async () => {
    const stub = new FetchStub().get('/api/v1/Bugs/1', '', { status: 302, headers: { location: '/404.aspx' } })
    const result = await client(stub).get('Bugs', 1)
    expect(result).toMatchObject({ ok: false, status: 302 })
  })

  it('rejects an HTML page where JSON was expected', async () => {
    const stub = new FetchStub().get('/api/v1/Bugs/1', '<!DOCTYPE html><html></html>')
    const result = await client(stub).get('Bugs', 1)
    expect(result.ok || result.message).toMatch(/got an HTML page/)
  })

  it('encodes path segments and drops dot segments', async () => {
    const stub = new FetchStub().get(/.*/, {})
    await client(stub).get('Bugs/../Users?x', 1)
    expect(stub.calls[0]!.url.pathname).toBe('/api/v1/Bugs/Users%3Fx/1')
  })
})

describe('V1Client.list', () => {
  const page = (ids: number[], next: boolean) => ({
    ...(next ? { Next: 'https://tp.example/api/v1/Users/?take=1000&skip=1000' } : {}),
    Items: ids.map((Id) => ({ ResourceType: 'User', Id, CreateDate: '/Date(0+0000)/' })),
  })

  it('pages with take=1000 until Next is absent', async () => {
    const first = Array.from({ length: 1000 }, (_, i) => i + 1)
    const stub = new FetchStub()
      .get('/api/v1/Users', page(first, true), { query: { skip: '0', take: '1000' } })
      .get('/api/v1/Users', page([1001, 1002], false), { query: { skip: '1000', take: '1000' } })
    const result = await client(stub).list<{ Id: number; CreateDate: string }>('Users', { where: "(IsActive eq 'true')" })
    expect(result.ok).toBe(true)
    if (!result.ok) return
    expect(result.data.items).toHaveLength(1002)
    expect(result.data.truncated).toBe(false)
    expect(result.data.items[0]!.CreateDate).toBe('1970-01-01T00:00:00.000+00:00')
    expect(stub.calls.map((c) => c.query.get('where'))).toEqual(["(IsActive eq 'true')", "(IsActive eq 'true')"])
  })

  it('reports truncated when the cap is reached with pages left', async () => {
    const stub = new FetchStub().get('/api/v1/Users', page([1, 2, 3], true), { query: { take: '3' } })
    const result = await client(stub).list('Users', { limit: 3 })
    expect(result.ok && result.data).toEqual({ items: expect.any(Array), truncated: true })
    expect(stub.calls).toHaveLength(1)
  })

  it('passes innerTake and orderByDesc through', async () => {
    const stub = new FetchStub().get('/api/v1/UserStories', page([1], false))
    await client(stub).list('UserStories', { include: '[Id,Tasks]', innerTake: 1000, orderByDesc: 'CreateDate' })
    const q = stub.calls[0]!.query
    expect(q.get('innerTake')).toBe('1000')
    expect(q.get('orderByDesc')).toBe('CreateDate')
    expect(q.get('include')).toBe('[Id,Tasks]')
  })

  it('returns the failure of any page', async () => {
    const stub = new FetchStub()
      .get('/api/v1/Users', page([1], true), { query: { skip: '0' } })
      .get('/api/v1/Users', { Message: 'boom' }, { status: 500, query: { skip: '1' } })
    const result = await client(stub).list('Users')
    expect(result).toMatchObject({ ok: false, status: 500, message: '500: boom' })
  })
})

describe('V1Client writes', () => {
  it('update posts the Id in the body and asks for a JSON result', async () => {
    const stub = new FetchStub().post('/api/v1/UserStories/5', { Id: 5, Name: 'x' })
    await client(stub).update('UserStories', 5, { Name: 'x' }, { resultInclude: '[Id,Name]' })
    const call = stub.calls[0]!
    expect(call.body).toEqual({ Name: 'x', Id: 5 })
    expect(call.query.get('resultFormat')).toBe('json')
    expect(call.query.get('resultInclude')).toBe('[Id,Name]')
  })

  it('removeFromCollection deletes each child by path (the ?childrenIds= form answers 500 live)', async () => {
    const stub = new FetchStub().delete(/^\/api\/v1\/UserStories\/5\/Assignments\/\d+$/, '')
    const r = await client(stub).removeFromCollection('UserStories', 5, 'Assignments', [7, 8])
    expect(stub.calls.map((c) => c.path)).toEqual(['/api/v1/UserStories/5/Assignments/7', '/api/v1/UserStories/5/Assignments/8'])
    expect(r.ok && r.data.removed).toEqual([7, 8])
  })

  it('removeFromCollection reports the children removed before a failure', async () => {
    const stub = new FetchStub()
      .delete('/api/v1/UserStories/5/Assignments/7', '')
      .delete('/api/v1/UserStories/5/Assignments/8', { Message: 'boom' }, { status: 500 })
    const r = await client(stub).removeFromCollection('UserStories', 5, 'Assignments', [7, 8, 9])
    expect(r.ok).toBe(false)
    if (!r.ok) expect(r.removed).toEqual([7])
    expect(stub.calls).toHaveLength(2)
  })

  it('deleteBulk sends [{Id}]', async () => {
    const stub = new FetchStub().delete('/api/v1/Tasks/bulk', '')
    await client(stub).deleteBulk('Tasks', [1, 2])
    expect(stub.calls[0]!.body).toEqual([{ Id: 1 }, { Id: 2 }])
  })
})

describe('innerItems', () => {
  it('reads included collections', () => {
    expect(innerItems({ Items: [1] })).toEqual([1])
    expect(innerItems(undefined)).toEqual([])
  })
})

describe('review regressions (HTTP)', () => {
  it('redaction never corrupts JSON bodies or rewrites user links', async () => {
    const body = '{"Id":1,"Description":"<a href=\\"https://x.io/?token=abc\\">link</a>"}'
    const stub = new FetchStub().get('/api/v1/Bugs/1', body)
    const result = await client(stub).get<{ Description: string }>('Bugs', 1)
    expect(result.ok).toBe(true)
    if (result.ok) expect(result.data.Description).toBe('<a href="https://x.io/?token=abc">link</a>')
  })

  it('still removes the exact token from bodies', async () => {
    const stub = new FetchStub().get('/api/v1/Bugs/1', { Echo: `access_token=${TOKEN}` })
    const result = await client(stub).get<{ Echo: string }>('Bugs', 1)
    expect(result.ok && result.data.Echo).toBe('access_token=***')
  })

  it('a write whose response body cannot be read says it was most likely applied', async () => {
    const http = new HttpCore({
      baseUrl: 'https://tp.example',
      token: TOKEN,
      fetch: async () => {
        const res = new Response('{}', { status: 201 })
        Object.defineProperty(res, 'text', { value: () => Promise.reject(new Error('socket hang up')) })
        return res
      },
    })
    const r = await new V1Client(http).create('Bugs', { Name: 'x' })
    expect(r.ok || r.message).toMatch(/answered 201, so the write was most likely applied: check before retrying/)
  })

  it('caps large error bodies', async () => {
    const stub = new FetchStub().get('/api/v1/Bugs/1', `<html>${'x'.repeat(10_000)}</html>`, { status: 500 })
    const r = await client(stub).get('Bugs', 1)
    expect(r.ok || r.body.length).toBeLessThan(4100)
    expect(r.ok || r.body).toMatch(/more characters\)$/)
  })
})
