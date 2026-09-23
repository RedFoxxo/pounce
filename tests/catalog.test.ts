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

describe('loadCatalog', () => {
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
  it('does not cache a failed load and never rejects from start()', async () => {
    const stub = new FetchStub().get('/api/v1/Index/meta', { Message: 'down' }, { status: 503 })
    let calls = 0
    const provider = new CatalogProvider(v1(stub), {
      snapshot: () => {
        calls++
        if (calls === 1) throw new Error('snapshot missing')
        return { instance: 'x', capturedAt: 'y', resources: [] }
      },
    })
    provider.start()
    await expect(provider.get()).rejects.toThrow('snapshot missing')
    await expect(provider.get()).resolves.toMatchObject({ source: 'snapshot' })
  })

  it('serves the snapshot when the live catalog is slow, then swaps the live one in', async () => {
    let release!: () => void
    const gate = new Promise<void>((resolve) => {
      release = resolve
    })
    const stub = new FetchStub()
      .get('/api/v1/Index/meta', async () => {
        await gate
        return fixture('index-meta.json')
      })
      .get('/api/v1/Context', { Version: 'x' })
      .get(/\/meta$/, { Status: 'NotFound', Message: 'Not Found' }, { status: 404 })
    const provider = new CatalogProvider(v1(stub), { timeoutMs: 10, snapshot: () => ({ instance: 'x', capturedAt: 'y', resources: [] }) })
    expect((await provider.get()).source).toBe('snapshot')
    release()
    await new Promise((r) => setTimeout(r, 50))
    expect((await provider.get()).source).toBe('live')
  })

  it('falls back to the snapshot when live metadata fails', async () => {
    const stub = new FetchStub().get('/api/v1/Index/meta', { Message: 'down' }, { status: 503 })
    const snapshot = { instance: 'x', capturedAt: 'y', resources: [] }
    const catalog = await new CatalogProvider(v1(stub), { snapshot: () => snapshot }).get()
    expect(catalog.source).toBe('snapshot')
  })

  it('loads once and caches', async () => {
    const stub = new FetchStub().get('/api/v1/Index/meta', { Message: 'down' }, { status: 503 })
    const provider = new CatalogProvider(v1(stub), { snapshot: () => ({ instance: 'x', capturedAt: 'y', resources: [] }) })
    await Promise.all([provider.get(), provider.get()])
    await provider.get()
    expect(stub.find('GET', '/api/v1/Index/meta')).toHaveLength(1)
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
