import { describe, expect, it } from 'vitest'
import { HttpCore } from '../src/http/core.js'
import { V1Client } from '../src/http/v1.js'
import { customFieldOptions, Directory, isTeamWorkflowState, type TpState } from '../src/resolve/directory.js'
import { cardInfo } from '../src/resolve/card.js'
import { FetchStub } from './helpers/fetch-stub.js'
import { harness } from './helpers/harness.js'
import { general, storyCustomFields, storyStates, withReferenceData } from './fixtures/reference.js'

function directory(stub = withReferenceData(new FetchStub())) {
  return { stub, dir: new Directory(new V1Client(new HttpCore({ baseUrl: 'https://tp.example', token: 't', fetch: stub.fetch }))) }
}

describe('people', () => {
  it('resolves by full name, login, email and id', async () => {
    const { dir } = directory()
    for (const input of ['Leszek Bielski', 'lbielski', 'LESZEK@example.com', 'bielski leszek', '2286', 2286]) {
      const r = await dir.user(input)
      expect(r.ok && r.value.Id, String(input)).toBe(2286)
    }
  })

  it('accepts a unique partial match', async () => {
    const { dir } = directory()
    const r = await dir.user('leszek')
    expect(r.ok && r.value.Id).toBe(2286)
  })

  it('lists every candidate when ambiguous', async () => {
    const { dir } = directory()
    const r = await dir.user('giorg')
    expect(r.ok).toBe(false)
    if (r.ok) return
    expect(r.reason).toBe('ambiguous')
    expect(r.candidates?.map((c) => c.id).sort()).toEqual([16, 17])
    expect(r.message).toContain('Giorgio Marchetti (16)')
    expect(r.message).toContain('Giorgia Rossi (17)')
  })

  it('refuses inactive people with a clear message', async () => {
    const { dir } = directory()
    const r = await dir.user('Rocco Amico')
    expect(r.ok || r.message).toBe('User Rocco Amico (2429) is inactive or deleted.')
  })

  it('an exact name of an inactive person is refused, not swapped for a partial active match', async () => {
    const stub = new FetchStub().get('/api/v1/Users', {
      Items: [
        { Id: 1, FirstName: 'John', LastName: 'Smith', Login: 'jsmith', IsActive: false },
        { Id: 2, FirstName: 'John', LastName: 'Smithson', Login: 'jsmithson', IsActive: true },
      ],
    })
    const { dir } = directory(stub)
    const r = await dir.user('John Smith')
    expect(r.ok || r.message).toBe('User John Smith (1) is inactive or deleted.')
    const partial = await dir.user('smithson')
    expect(partial.ok && partial.value.Id).toBe(2)
  })

  it('suggests close names when nothing matches', async () => {
    const { dir } = directory()
    const r = await dir.user('Leszk Bielski')
    expect(r.ok).toBe(false)
    if (!r.ok) expect(r.message).toMatch(/Did you mean: Leszek Bielski \(2286\)/)
  })

  it('pages the full user list', async () => {
    const stub = new FetchStub()
      .get('/api/v1/Users', { Next: 'x', Items: Array.from({ length: 1000 }, (_, i) => ({ Id: i + 100, FirstName: 'U', LastName: String(i), IsActive: true })) }, { query: { skip: '0' } })
      .get('/api/v1/Users', { Items: [{ Id: 5000, FirstName: 'Late', LastName: 'Page', IsActive: true }] }, { query: { skip: '1000' } })
    const { dir } = directory(stub)
    const r = await dir.user('Late Page')
    expect(r.ok && r.value.Id).toBe(5000)
  })

  it('caches reference data', async () => {
    const { dir, stub } = directory()
    await dir.user('lbielski')
    await dir.user('giorgio@example.com')
    expect(stub.find('GET', '/api/v1/Users')).toHaveLength(1)
  })
})

describe('roles, teams, projects', () => {
  it('prefers the exact role over partial ones', async () => {
    const { dir } = directory()
    const r = await dir.role('developer')
    expect(r.ok && r.value.Id).toBe(13)
  })

  it('reports ambiguous partial roles', async () => {
    const { dir } = directory()
    const r = await dir.role('end developer')
    expect(r.ok || r.candidates?.map((c) => c.name)).toEqual(['Backend Developer', 'Frontend Developer'])
  })

  it('refuses inactive teams', async () => {
    const { dir } = directory()
    expect((await dir.team('core')).ok).toBe(true)
    const legacy = await dir.team('Legacy Team')
    expect(legacy.ok || legacy.message).toMatch(/inactive/)
  })

  it('resolves projects by abbreviation', async () => {
    const { dir } = directory()
    const r = await dir.project('amp')
    expect(r.ok && r.value.Id).toBe(179)
  })

  it('carries the HTTP failure', async () => {
    const stub = new FetchStub().get('/api/v1/Roles', { Message: 'Forbidden' }, { status: 403 })
    const { dir } = directory(stub)
    const r = await dir.role('Developer')
    expect(r.ok).toBe(false)
    if (!r.ok) expect(r.error?.status).toBe(403)
  })
})

describe('states and custom fields', () => {
  it('resolves states within the project workflow only by default', async () => {
    const { dir } = directory()
    const coded = await dir.state(13, 'UserStory', 'coded')
    expect(coded.ok && coded.value.Id).toBe(687)
    const review = await dir.state(13, 'UserStory', 'Code Review')
    expect(review.ok).toBe(false)
    const team = await dir.state(13, 'UserStory', 'Code Review', true)
    expect(team.ok && team.value.Id).toBe(900)
  })

  it('flags team sub-workflow states', () => {
    expect(storyStates.filter((s) => isTeamWorkflowState(s as TpState)).map((s) => s.Id)).toEqual([900])
  })

  it('reads dropdown options', async () => {
    const { dir } = directory()
    const f = await dir.customField(13, 'UserStory', 'frontend')
    expect(f.ok && customFieldOptions(f.value)).toEqual(['To Do', 'Doing', 'Review', 'Done'])
    expect(customFieldOptions(storyCustomFields[0]!)).toBeUndefined()
  })
})

describe('cardInfo', () => {
  it('resolves type, resource, project and process from Generals', async () => {
    const h = await harness({ stub: new FetchStub().get('/api/v1/Generals/36217', general(36217, 'UserStory')) })
    const r = await cardInfo(h.ctx, 36217)
    expect(r.ok && { type: r.value.entityType, path: r.value.resource.path, project: r.value.project, process: r.value.processId }).toEqual({
      type: 'UserStory',
      path: 'UserStories',
      project: { id: 26080, name: 'SBP' },
      process: 13,
    })
    await h.close()
  })

  it('reports a missing card', async () => {
    const h = await harness({
      stub: new FetchStub().get('/api/v1/Generals/1', { Message: 'General with Id 1 not found or access is forbidden.' }, { status: 404 }),
    })
    const r = await cardInfo(h.ctx, 1)
    expect(r.ok || r.message).toBe('No card with id 1 (or no access to it).')
    await h.close()
  })
})

describe('custom field verification', () => {
  it('compares entity fields by id, URLs by URL, dates by day, lists as sets', async () => {
    const { unpersistedCustomFields } = await import('../src/domain/custom-fields.js')
    const back = [
      { Name: 'Rel', Value: { Id: 2, Kind: 'Release' } },
      { Name: 'Link', Value: { Url: 'https://x', Label: 'x' } },
      { Name: 'Due', Value: '2026-09-23T00:00:00.000+02:00' },
      { Name: 'Multi', Value: 'b, a' },
    ]
    expect(
      unpersistedCustomFields(
        [
          { Name: 'Rel', Value: { Id: 1, Kind: 'Release' } },
          { Name: 'Link', Value: { Url: 'https://x', Label: 'x' } },
          { Name: 'Due', Value: '2026-09-23' },
          { Name: 'Multi', Value: 'a,b' },
          { Name: 'Gone', Value: 'x' },
        ],
        back,
      ),
    ).toEqual(['Rel: requested {"Id":1,"Kind":"Release"}, got {"Id":2,"Kind":"Release"}', 'Gone: not present on the card after writing'])
  })
})
