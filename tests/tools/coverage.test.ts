import { afterAll, beforeAll, describe, expect, it } from 'vitest'
import type { CatalogResource } from '../../src/catalog/types.js'
import { isAdminResource, READ_ONLY_IN_PRACTICE } from '../../src/tools/generic/policy.js'
import { FetchStub } from '../helpers/fetch-stub.js'
import { harness, snapshot, type Harness } from '../helpers/harness.js'

/**
 * Catalog coverage: every resource × operation and every addable/removable
 * collection in snapshot.json is reachable through a layer 1 tool, driven
 * through the MCP server with a stubbed fetch.
 */
const catalog = snapshot()
const usable = catalog.resources.filter((r) => r.available && !READ_ONLY_IN_PRACTICE[r.name])

function sampleValue(type: string): unknown {
  if (/^(Int|Decimal|Double|Float|Single)/i.test(type)) return 1
  if (/^Bool/i.test(type)) return true
  if (/^DateTime/i.test(type)) return '2026-01-01'
  return 'x'
}

/** One settable field, so the payload is non-empty and valid. */
function sampleFields(r: CatalogResource): Record<string, unknown> | undefined {
  const value = r.values.find((f) => f.canSet && f.name !== 'Id')
  if (value) return { [value.name]: sampleValue(value.type) }
  const ref = r.references.find((f) => f.canSet)
  if (ref) return { [ref.name]: { Id: 1 } }
  const coll = r.collections.find((c) => c.canAdd)
  if (coll) return { [coll.name]: [coll.name === 'CustomFields' ? { Name: 'x', Value: 1 } : { Id: 1 }] }
  return undefined
}

let h: Harness
beforeAll(async () => {
  const stub = new FetchStub()
    .get(/.*/, { Id: 1, Name: 'x', Items: [] })
    .post(/.*/, { Id: 1 })
    .delete(/.*/, '')
  h = await harness({ stub })
})
afterAll(async () => {
  await h.close()
})

function tool(r: CatalogResource, op: 'create' | 'update' | 'delete' | 'add' | 'remove'): string {
  const admin = isAdminResource(r)
  return {
    create: admin ? 'admin_create' : 'write_create',
    update: admin ? 'admin_update' : 'write_update',
    delete: admin ? 'admin_delete' : 'delete_entity',
    add: admin ? 'admin_collection_add' : 'write_collection_add',
    remove: admin ? 'admin_collection_remove' : 'delete_collection_remove',
  }[op]
}

async function reached(name: string, args: Record<string, unknown>, method: string, path: string) {
  const before = h.stub.calls.length
  const r = await h.call(name, args)
  const sent = h.stub.calls.slice(before).filter((c) => c.method === method && c.path.toLowerCase() === path.toLowerCase())
  return { ok: !r.isError && sent.length > 0, text: r.text }
}

describe('catalog coverage', () => {
  it('covers the whole baseline', () => {
    expect(usable.length).toBeGreaterThanOrEqual(140)
  })

  it('every resource is readable with read_get and read_query', async () => {
    const failures: string[] = []
    for (const r of catalog.resources.filter((x) => x.available)) {
      const get = await reached('read_get', { resource: r.name, id: 1 }, 'GET', `/api/v1/${r.path}/1`)
      const query = await reached('read_query', { resource: r.name, limit: 1 }, 'GET', `/api/v1/${r.path}`)
      if (!get.ok) failures.push(`read_get ${r.name}: ${get.text}`)
      if (!query.ok) failures.push(`read_query ${r.name}: ${query.text}`)
    }
    expect(failures).toEqual([])
  })

  it('every create/update/delete in the catalog is reachable', async () => {
    const failures: string[] = []
    const noSettable: string[] = []
    for (const r of usable) {
      const fields = sampleFields(r)
      if (r.canCreate) {
        if (!fields) noSettable.push(`${r.name} (create)`)
        else {
          const res = await reached(tool(r, 'create'), { resource: r.name, fields }, 'POST', `/api/v1/${r.path}`)
          if (!res.ok) failures.push(`${tool(r, 'create')} ${r.name}: ${res.text}`)
        }
      }
      if (r.canUpdate) {
        if (!fields) noSettable.push(`${r.name} (update)`)
        else {
          const res = await reached(tool(r, 'update'), { resource: r.name, id: 1, fields }, 'POST', `/api/v1/${r.path}/1`)
          if (!res.ok) failures.push(`${tool(r, 'update')} ${r.name}: ${res.text}`)
        }
      }
      if (r.canDelete) {
        const res = await reached(tool(r, 'delete'), { resource: r.name, id: 1 }, 'DELETE', `/api/v1/${r.path}/1`)
        if (!res.ok) failures.push(`${tool(r, 'delete')} ${r.name}: ${res.text}`)
      }
    }
    expect(failures).toEqual([])
    // Resources whose metadata allows a write but exposes no settable field at all.
    expect(noSettable).toEqual([])
  })

  it('every addable and removable collection is reachable', async () => {
    const failures: string[] = []
    let count = 0
    for (const r of usable) {
      for (const c of r.collections) {
        if (c.canAdd) {
          count++
          const item = c.name === 'CustomFields' ? { Name: 'x', Value: 1 } : { Id: 1 }
          const res = await reached(tool(r, 'add'), { resource: r.name, id: 1, collection: c.name, items: [item] }, 'POST', `/api/v1/${r.path}/1`)
          if (!res.ok) failures.push(`${tool(r, 'add')} ${r.name}.${c.name}: ${res.text}`)
        }
        if (c.canRemove) {
          const res = await reached(
            tool(r, 'remove'),
            { resource: r.name, id: 1, collection: c.name, childIds: [2] },
            'DELETE',
            `/api/v1/${r.path}/1/${c.name}/2`,
          )
          if (!res.ok) failures.push(`${tool(r, 'remove')} ${r.name}.${c.name}: ${res.text}`)
        }
      }
    }
    expect(failures).toEqual([])
    expect(count).toBe(543)
  })
})
