import { readFileSync } from 'node:fs'
import { describe, expect, it } from 'vitest'
import { VERSION } from '../src/version.js'
import { allTools } from '../src/tools/index.js'
import { tierOf } from '../src/tools/types.js'
import { harness } from './helpers/harness.js'

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
      expect(tool.annotations?.readOnlyHint, tool.name).toBe(tool.name.startsWith('read_'))
    }
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
