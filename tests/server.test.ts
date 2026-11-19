import { readFileSync } from 'node:fs'
import { describe, expect, it } from 'vitest'
import { VERSION } from '../src/version.js'
import { allTools } from '../src/tools/index.js'
import { tierOf } from '../src/tools/types.js'
import { general } from './fixtures/reference.js'
import { FetchStub } from './helpers/fetch-stub.js'
import { harness } from './helpers/harness.js'

const fixture = (name: string) => readFileSync(new URL(`./fixtures/${name}`, import.meta.url), 'utf8')

describe('server', () => {
  it('reports the package version', () => {
    const pkg = JSON.parse(readFileSync(new URL('../package.json', import.meta.url), 'utf8')) as { version: string }
    expect(VERSION).toBe(pkg.version)
  })

  it('completes the MCP handshake and lists every tool', async () => {
    const h = await harness()
    const tools = await h.listTools()
    expect(tools.map((t) => t.name).sort()).toEqual(allTools().map((t) => t.name).sort())
    await h.close()
  })

  it('exposes only the configured toolsets', async () => {
    const h = await harness({ config: { toolsets: ['cards'] } })
    const names = (await h.listTools()).map((t) => t.name)
    expect(names).toContain('read_card')
    expect(names).toContain('read_history')
    expect(names).not.toContain('read_query')
    expect(names.some((n) => n.startsWith('admin_'))).toBe(false)
    expect((await h.call('write_create', { resource: 'UserStory', fields: { Name: 'x' } })).isError).toBe(true)
    expect(h.stub.calls).toHaveLength(0)
    await h.close()
  })

  it('sends no request to Targetprocess at startup', async () => {
    const h = await harness({ config: { toolsets: ['cards'] }, liveCatalog: true })
    await h.listTools()
    expect(h.stub.calls).toHaveLength(0)
    await h.close()
  })

  it('a card read loads only the metadata it uses, and only once', async () => {
    const stub = new FetchStub()
      .get('/api/v1/Generals/36216', general(36216, 'UserStory'))
      .get('/api/v1/Index/meta', fixture('index-meta.json'))
      .get('/api/v1/UserStories/meta', JSON.parse(fixture('userstory-meta.json')))
      .get('/api/v1/UserStories/36216', { Id: 36216, Name: 'Story', EntityState: { Id: 684, Name: 'Designable' } })
    const h = await harness({ stub, config: { toolsets: ['cards'] }, liveCatalog: true })
    const reads = await Promise.all([h.call('read_card', { id: 36216 }), h.call('read_card', { id: 36216 })])
    for (const r of reads) expect(r.json).toMatchObject({ id: 36216, type: 'UserStory', state: { name: 'Designable' } })
    const metadata = stub.calls.filter((c) => c.path.endsWith('/meta')).map((c) => c.path)
    expect(metadata).toEqual(['/api/v1/Index/meta', '/api/v1/UserStories/meta'])
    expect(stub.unmatched).toEqual([])
    await h.close()
  })
})

describe('tool naming contract', () => {
  const tools = allTools()

  it('every tool has a tier prefix and no pounce_ prefix', () => {
    for (const tool of tools) {
      expect(tierOf(tool.name), tool.name).toBeDefined()
      expect(tool.name.startsWith('pounce_')).toBe(false)
    }
  })

  it('names are unique', () => {
    const names = tools.map((t) => t.name)
    expect(new Set(names).size).toBe(names.length)
  })

  it('read tools are annotated read-only and nothing else is', async () => {
    const h = await harness()
    for (const tool of await h.listTools()) {
      expect(tool.annotations?.readOnlyHint === true, tool.name).toBe(tool.name.startsWith('read_'))
    }
    await h.close()
  })

  it('lists tools without schema noise: no $schema, execution, or default keywords', async () => {
    const h = await harness()
    const listed = JSON.stringify(await h.listTools())
    for (const noise of ['"$schema"', '"execution"', '"propertyNames"', '"additionalProperties":{}', '"destructiveHint"', '"openWorldHint"']) {
      expect(listed).not.toContain(noise)
    }
    const tools = (await h.listTools()) as unknown as { name: string; inputSchema: { properties: Record<string, unknown> } }[]
    const assign = tools.find((t) => t.name === 'write_assign')!
    expect(assign.inputSchema.properties.role).toEqual({ type: 'string', description: 'Role name or id, e.g. "Developer", "Product Owner"' })
    expect(tools.find((t) => t.name === 'write_set_custom_fields')!.inputSchema.properties.fields).toEqual({ type: 'object' })
    await h.close()
  })

  it('every id parameter is validated as ^\\d+$', async () => {
    const h = await harness()
    const r = await h.call('read_get', { resource: 'UserStory', id: '12a' })
    expect(r.isError).toBe(true)
    expect(r.text).toMatch(/numeric id/)
    expect(h.stub.calls).toHaveLength(0)
    await h.close()
  })
})
