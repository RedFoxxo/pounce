import { readSnapshot } from '../../src/catalog/catalog.js'
import type { CatalogCollection, CatalogField } from '../../src/catalog/types.js'
import { StubReply, type FetchStub, type StubCall } from './fetch-stub.js'
import { BASE } from './harness.js'

function rawField(f: CatalogField | CatalogCollection) {
  return {
    Name: f.name,
    Type: f.type,
    CanSet: f.canSet,
    IsRequired: f.required,
    IsDeprecated: f.deprecated,
    ...('canAdd' in f ? { CanAdd: f.canAdd, CanRemove: f.canRemove } : {}),
  }
}

/**
 * Answers `Index/meta` and every `/{Resource}/meta` from the committed
 * snapshot, in Targetprocess's own shapes, so tests can run against the real
 * lazily loaded catalog. Takes precedence over every route registered so far.
 */
export function serveMetadata(stub: FetchStub): FetchStub {
  const resources = readSnapshot().resources
  const byPath = new Map(resources.map((r) => [r.path.toLowerCase(), r]))
  // The index repeats one key per resource, as the real one does.
  const index = `{"IndexUri":"${BASE}/api/v1/index/meta",${resources
    .filter((r) => r.listed)
    .map((r) => `"ResourceMetadataDescription":${JSON.stringify({ Name: r.name, Uri: `${BASE}/api/v1/${r.path}/meta`, Description: r.description })}`)
    .join(',')}}`
  return stub.first({
    method: 'GET',
    path: /^\/api\/v1\/[^/]+\/meta$/,
    body: (call: StubCall) => {
      const path = call.path.split('/')[3] ?? ''
      if (path === 'Index') return index
      const r = byPath.get(path.toLowerCase())
      if (!r?.available) return new StubReply(404, { Status: 'NotFound', Message: 'Not Found' })
      return {
        Name: r.name,
        Uri: `${BASE}/api/v1/${r.path}`,
        Description: r.description,
        CanCreate: r.canCreate,
        CanUpdate: r.canUpdate,
        CanDelete: r.canDelete,
        ResourceMetadataHierarchyDescription: { ResourceMetadataBaseResourceDescription: { Items: r.bases.map((Name) => ({ Name })) } },
        ResourceMetadataPropertiesDescription: {
          ResourceMetadataPropertiesResourceValuesDescription: { Items: r.values.map(rawField) },
          ResourceMetadataPropertiesResourceReferencesDescription: { Items: r.references.map(rawField) },
          ResourceMetadataPropertiesResourceCollectionsDescription: { Items: r.collections.map(rawField) },
        },
      }
    },
  })
}
