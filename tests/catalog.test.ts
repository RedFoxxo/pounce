import { readFileSync } from 'node:fs'
import { describe, expect, it } from 'vitest'
import { Catalog, CatalogProvider, readSnapshot, suggestions } from '../src/catalog/catalog.js'
import { loadCatalog, parseIndex, parseMetaXml } from '../src/catalog/loader.js'
import { HttpCore } from '../src/http/core.js'
import { V1Client } from '../src/http/v1.js'
import { FetchStub, StubReply } from './helpers/fetch-stub.js'

const fixture = (name: string) => readFileSync(new URL(`./fixtures/${name}`, import.meta.url), 'utf8')

function v1(stub: FetchStub) {
  return new V1Client(new HttpCore({ baseUrl: 'https://tp.example', token: 't0ken', fetch: stub.fetch }))
}

describe('parseIndex', () => {
  it('reads every entry despite the repeated JSON key', () => {
    const text = fixture('index-meta.json')
    expect(Object.keys(JSON.parse(text))).toHaveLength(2) // what JSON.parse sees: IndexUri + one entry
    const entries = parseIndex(text)
    expect(entries.map((e) => [e.name, e.path])).toEqual([
      ['AcceptanceCriterion', 'AcceptanceCriterions'],
      ['ActionItem', 'ActionItems'],
      ['Context', 'Context'],
      ['SickLeave', 'SickLeaves'],
      ['UserStory', 'UserStories'],
    ])
  })
})

describe('parseMetaXml', () => {
  it('converts XML-only metadata (Context/meta)', () => {
    const meta = parseMetaXml(fixture('context-meta.xml'))
    expect(meta?.Name).toBe('Context')
    expect(meta?.CanCreate).toBe(true)
    const collections = meta?.ResourceMetadataPropertiesDescription?.ResourceMetadataPropertiesResourceCollectionsDescription?.Items
    expect(collections?.map((c) => c.Name)).toEqual(['SelectedProjects', 'SelectedTeams', 'Processes', 'GlobalTerms', 'CustomFields'])
    expect(collections?.[0]).toMatchObject({ CanAdd: false, CanRemove: false, CanSet: false })
  })
})

/** An instance with five listed resources (SickLeave broken) and two unlisted ones. */
function instance() {
  return new FetchStub()
    .get('/api/v1/Index/meta', fixture('index-meta.json'))
    .get('/api/v1/UserStories/meta', JSON.parse(fixture('userstory-meta.json')))
    .get('/api/v1/Context/meta', fixture('context-meta.xml'))
    .get('/api/v1/SickLeaves/meta', { Status: 'NotFound', Message: 'Not Found' }, { status: 404 })
    .get('/api/v1/AcceptanceCriterions/meta', { Name: 'AcceptanceCriterion', Uri: 'https://tp.example/api/v1/AcceptanceCriterions', CanCreate: true, CanUpdate: true, CanDelete: true })
    .get('/api/v1/ActionItems/meta', { Name: 'ActionItem', Uri: 'https://tp.example/api/v1/ActionItems', CanCreate: true, CanUpdate: true, CanDelete: true })
    .get('/api/v1/UserStoryHistories/meta', { Name: 'UserStoryHistory', Uri: 'https://tp.example/api/v1/UserStoryHistories', CanCreate: false, CanUpdate: false, CanDelete: false })
    .get('/api/v1/GeneralConversions/meta', { Name: 'GeneralConversion', Uri: 'https://tp.example/api/v1/GeneralConversions' })
    .get('/api/v1/Context', { Version: '2609.2.0.3492' })
    .get(/Histories\/meta$/, { Status: 'NotFound', Message: 'Not Found' }, { status: 404 })
}

describe('loadCatalog', () => {
  it('loads listed resources, marks broken ones unavailable and probes unlisted ones', async () => {
    const stub = instance()
    const result = await loadCatalog(v1(stub), { now: () => new Date('2026-09-23T00:00:00Z') })
    expect(result.ok).toBe(true)
    if (!result.ok) return
    const byName = new Map(result.data.resources.map((r) => [r.name, r]))
    expect(result.data.version).toBe('2609.2.0.3492')

    expect(byName.get('SickLeave')).toMatchObject({ listed: true, available: false })
    expect(byName.get('UserStoryHistory')).toMatchObject({ listed: false, available: true, path: 'UserStoryHistories' })
    expect(byName.get('GeneralConversion')).toMatchObject({ listed: false, path: 'GeneralConversions' })
    expect(byName.get('Context')?.collections).toHaveLength(5)

    const story = byName.get('UserStory')!
    expect(story).toMatchObject({ path: 'UserStories', canCreate: true, bases: ['Assignable'] })
    expect(story.values.find((f) => f.name === 'Name')).toMatchObject({ required: true, canSet: true })
    expect(story.collections.find((c) => c.name === 'Comments')).toMatchObject({ canAdd: true, canRemove: true })

    for (const name of ['SickLeave', 'UserStory', 'Context']) {
      expect(stub.find('GET', `/api/v1/${name}Histories/meta`)).toHaveLength(1)
      expect(stub.find('GET', `/api/v1/${name}SimpleHistories/meta`)).toHaveLength(1)
    }
    expect(stub.unmatched).toEqual([])
  })

  it('fails when the index itself is unavailable', async () => {
    const stub = new FetchStub().get('/api/v1/Index/meta', { Message: 'Unauthorized' }, { status: 401 })
    expect(await loadCatalog(v1(stub))).toMatchObject({ ok: false, status: 401 })
  })
})

describe('loadCatalog transient failures', () => {
  it('retries a failed /meta once, then uses the snapshot entry instead of dropping the resource', async () => {
    const snapshot = readSnapshot()
    let storyMetaCalls = 0
    const stub = new FetchStub()
      .get('/api/v1/Index/meta', fixture('index-meta.json'))
      .get('/api/v1/UserStories/meta', () => {
        storyMetaCalls++
        return new StubReply(503, { Message: 'Service Unavailable' })
      })
      .get('/api/v1/Context', { Version: 'x' })
      .get(/\/meta$/, { Status: 'NotFound', Message: 'Not Found' }, { status: 404 })
    const result = await loadCatalog(v1(stub), { fallback: () => snapshot })
    expect(storyMetaCalls).toBe(2)
    const story = result.ok ? result.data.resources.find((r) => r.name === 'UserStory') : undefined
    expect(story).toMatchObject({ available: true, listed: true, path: 'UserStories' })
    expect(story?.collections.length).toBeGreaterThan(10)
  })

  it('a 404 is final: no retry and no fallback', async () => {
    const stub = new FetchStub()
      .get('/api/v1/Index/meta', fixture('index-meta.json'))
      .get('/api/v1/Context', { Version: 'x' })
      .get(/\/meta$/, { Status: 'NotFound', Message: 'Not Found' }, { status: 404 })
    const result = await loadCatalog(v1(stub), { fallback: () => readSnapshot() })
    expect(stub.find('GET', '/api/v1/SickLeaves/meta')).toHaveLength(1)
    expect(result.ok && result.data.resources.find((r) => r.name === 'SickLeave')?.available).toBe(false)
  })
})

describe('CatalogProvider', () => {
  const empty = () => ({ instance: 'x', capturedAt: 'y', resources: [] })

  it('loads only the requested resources, once, for concurrent callers', async () => {
    const stub = instance()
    const provider = new CatalogProvider(v1(stub), { snapshot: empty })
    const [a, b] = await Promise.all([provider.get(['UserStory']), provider.get(['userstories'])])
    for (const catalog of [a, b]) {
      expect(catalog.source).toBe('live')
      expect(catalog.find('UserStory')).toMatchObject({ path: 'UserStories', available: true, listed: true })
    }
    await provider.get(['UserStory'])
    expect(stub.calls.map((c) => c.path)).toEqual(['/api/v1/Index/meta', '/api/v1/UserStories/meta'])
    // Unknown names are checked against the index, which lists every resource.
    const unknown = a.resource('UserStorie')
    expect(unknown.ok ? '' : unknown.message).toMatch(/Unknown resource "UserStorie"\. Did you mean: UserStory/)
  })

  it('resolves unlisted history resources on demand; a 404 is final', async () => {
    const stub = instance()
    const provider = new CatalogProvider(v1(stub), { snapshot: empty })
    const catalog = await provider.get(['UserStoryHistories', 'UserStorySimpleHistory', 'SickLeave', 'GeneralConversions'])
    expect(catalog.find('UserStoryHistory')).toMatchObject({ path: 'UserStoryHistories', listed: false, available: true })
    expect(catalog.find('UserStorySimpleHistories')).toBeUndefined()
    expect(catalog.find('SickLeave')).toMatchObject({ listed: true, available: false })
    expect(catalog.find('GeneralConversions')).toMatchObject({ listed: false })
    await provider.get(['UserStorySimpleHistories', 'SickLeaves'])
    expect(stub.find('GET', '/api/v1/UserStorySimpleHistories/meta')).toHaveLength(1)
    expect(stub.find('GET', '/api/v1/SickLeaves/meta')).toHaveLength(1)
  })

  it('a failed /meta serves the snapshot entry for that call and is retried on the next', async () => {
    let storyMeta = 0
    const stub = instance().first({
      method: 'GET',
      path: '/api/v1/UserStories/meta',
      body: () => (++storyMeta <= 2 ? new StubReply(503, { Message: 'Service Unavailable' }) : JSON.parse(fixture('userstory-meta.json'))),
    })
    const provider = new CatalogProvider(v1(stub))
    const first = await provider.get(['UserStory'])
    expect(first.source).toBe('snapshot')
    expect(first.find('UserStory')?.collections.length).toBeGreaterThan(10)
    const second = await provider.get(['UserStory'])
    expect(second.source).toBe('live')
    expect(storyMeta).toBe(3)
  })

  it('a failed index falls back to the snapshot names and is retried on the next call', async () => {
    let index = 0
    const stub = instance().first({
      method: 'GET',
      path: '/api/v1/Index/meta',
      body: () => (++index === 1 ? new StubReply(503, { Message: 'down' }) : fixture('index-meta.json')),
    })
    const provider = new CatalogProvider(v1(stub))
    expect((await provider.get(['UserStory'])).source).toBe('snapshot')
    expect((await provider.get(['UserStory'])).source).toBe('live')
    expect(index).toBe(2)
  })

  it("'all' loads every listed resource, the unlisted ones and the version", async () => {
    const stub = instance()
    const catalog = await new CatalogProvider(v1(stub), { snapshot: empty }).get('all')
    expect(catalog.source).toBe('live')
    expect(catalog.data.version).toBe('2609.2.0.3492')
    expect(catalog.resources.map((r) => r.name).sort()).toEqual(
      ['AcceptanceCriterion', 'ActionItem', 'Context', 'GeneralConversion', 'SickLeave', 'UserStory', 'UserStoryHistory'].sort(),
    )
    expect(stub.unmatched).toEqual([])
  })
})

describe('Catalog lookups', () => {
  const catalog = new Catalog(readSnapshot(), 'snapshot')

  it('finds by name or path, case-insensitively', () => {
    expect(catalog.find('userstory')?.path).toBe('UserStories')
    expect(catalog.find('UserStories')?.name).toBe('UserStory')
  })

  it('suggests close names for unknown resources', () => {
    const lookup = catalog.resource('UserStorys')
    expect(lookup.ok).toBe(false)
    if (!lookup.ok) expect(lookup.message).toMatch(/Did you mean: .*UserStory/)
  })

  it('refuses unavailable resources', () => {
    const lookup = catalog.resource('SickLeave')
    expect(lookup.ok || lookup.message).toMatch(/metadata is unavailable/)
  })

  it('suggestions ranks prefixes first', () => {
    expect(suggestions('Bug', ['Bug', 'BugHistory', 'Build', 'Epic'])[0]).toBe('Bug')
  })
})

describe('snapshot.json', () => {
  const data = readSnapshot()
  const listed = data.resources.filter((r) => r.listed)
  const collections = data.resources.flatMap((r) => r.collections)

  it('matches the researched baseline of our instance', () => {
    expect(listed).toHaveLength(89)
    expect(listed.filter((r) => !r.available).map((r) => r.name)).toEqual(['SickLeave'])
    expect(collections).toHaveLength(1054)
    expect(collections.filter((c) => c.canAdd || c.canRemove)).toHaveLength(543)
  })

  it('includes the unlisted history and conversion resources', () => {
    const unlisted = data.resources.filter((r) => !r.listed).map((r) => r.path)
    expect(unlisted).toContain('GeneralConversions')
    expect(unlisted).toContain('TaskHistories')
    expect(unlisted).toContain('UserStorySimpleHistories')
  })

  it('has unique names and paths', () => {
    const owner = new Map<string, string>()
    for (const r of data.resources) {
      for (const key of new Set([r.name.toLowerCase(), r.path.toLowerCase()])) {
        expect(owner.get(key) ?? r.name).toBe(r.name)
        owner.set(key, r.name)
      }
    }
  })
})
