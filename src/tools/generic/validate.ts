import { suggestions, type Catalog } from '../../catalog/catalog.js'
import type { CatalogCollection, CatalogField, CatalogResource } from '../../catalog/types.js'
import { sameDateValue } from '../../format/dates.js'

export type Prepared<T> = { ok: true; value: T } | { ok: false; message: string }

/** Collections Targetprocess takes as a plain array rather than `{"Items": [...]}`. */
const ARRAY_COLLECTIONS = new Set(['tagobjects', 'customfields'])

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

function settableNames(resource: CatalogResource): string[] {
  return [
    ...resource.values.filter((f) => f.canSet && f.name !== 'Id').map((f) => f.name),
    ...resource.references.filter((f) => f.canSet).map((f) => f.name),
    ...resource.collections.filter((c) => c.canAdd).map((c) => c.name),
  ]
}

function kindOf(resource: CatalogResource, member: CatalogField): 'value' | 'reference' | 'collection' {
  if (resource.collections.includes(member as CatalogCollection)) return 'collection'
  if (resource.references.includes(member)) return 'reference'
  return 'value'
}

/** A reference is `{"Id": n}` (numeric string accepted) or `null` to clear it. Never an empty reference. */
export function prepareReference(value: unknown, label: string): Prepared<{ Id: number } | null> {
  if (value === null) return { ok: true, value: null }
  const idValue = isRecord(value) ? value.Id ?? value.id : value
  if (typeof idValue === 'number' && Number.isSafeInteger(idValue) && idValue > 0) return { ok: true, value: { Id: idValue } }
  if (typeof idValue === 'string' && /^\d+$/.test(idValue) && Number(idValue) > 0) return { ok: true, value: { Id: Number(idValue) } }
  return {
    ok: false,
    message: `${label} must be a reference {"Id": <number>} (or null to clear); got ${JSON.stringify(value) ?? 'undefined'}. Empty references are never sent.`,
  }
}

/**
 * Validates a create/update payload against the catalog and returns it with
 * canonical field names. Nested collection items are validated against the
 * collection's own resource when the catalog knows it.
 */
export function prepareBody(
  catalog: Catalog,
  resource: CatalogResource,
  body: Record<string, unknown>,
  mode: 'create' | 'update',
  path = '',
): Prepared<Record<string, unknown>> {
  const problems: string[] = []
  const out: Record<string, unknown> = {}
  const label = (key: string) => (path ? `${path}.${key}` : key)

  for (const [key, value] of Object.entries(body)) {
    if (key.toLowerCase() === 'id') {
      problems.push(
        mode === 'create'
          ? `${label(key)}: remove Id when creating (posting an Id updates an existing entity instead)`
          : `${label(key)}: pass the id as the "id" argument, not inside fields`,
      )
      continue
    }
    const member = catalog.member(resource, key)
    if (!member) {
      const close = suggestions(key, settableNames(resource), 5)
      const customHint = resource.collections.some((c) => c.name === 'CustomFields')
        ? ' Custom fields go in "CustomFields": [{"Name": "...", "Value": ...}].'
        : ''
      problems.push(
        `${label(key)}: ${resource.name} has no field "${key}".` + (close.length ? ` Did you mean: ${close.join(', ')}?` : '') + customHint,
      )
      continue
    }

    const kind = kindOf(resource, member)
    if (kind === 'value') {
      if (!member.canSet) {
        problems.push(`${label(member.name)}: ${resource.name}.${member.name} is read-only`)
        continue
      }
      out[member.name] = value
    } else if (kind === 'reference') {
      if (!member.canSet) {
        problems.push(`${label(member.name)}: ${resource.name}.${member.name} is read-only`)
        continue
      }
      const ref = prepareReference(value, label(member.name))
      if (!ref.ok) problems.push(ref.message)
      else out[member.name] = ref.value
    } else {
      const items = prepareItems(catalog, resource, member as CatalogCollection, value, label(member.name))
      if (!items.ok) problems.push(items.message)
      else out[member.name] = items.value
    }
  }

  if (problems.length > 0) {
    const settable = settableNames(resource)
    return {
      ok: false,
      message:
        `${problems.length === 1 ? 'Invalid field' : `${problems.length} invalid fields`} for ${resource.name}:\n- ${problems.join('\n- ')}` +
        (path ? '' : `\nSettable on ${resource.name}: ${settable.join(', ')}`),
    }
  }
  return { ok: true, value: out }
}

/**
 * Validates the items posted into a collection and returns the wire shape:
 * `{"Items": [...]}`, or a plain array for TagObjects and CustomFields.
 */
export function prepareItems(
  catalog: Catalog,
  resource: CatalogResource,
  collection: CatalogCollection,
  value: unknown,
  label: string,
): Prepared<unknown> {
  if (!collection.canAdd) {
    return { ok: false, message: `${label}: items cannot be added to ${resource.name}.${collection.name}` }
  }
  const raw = Array.isArray(value) ? value : isRecord(value) && Array.isArray(value.Items) ? value.Items : undefined
  if (!raw) {
    return { ok: false, message: `${label}: expected an array of items or {"Items": [...]}` }
  }
  if (raw.length === 0) return { ok: false, message: `${label}: no items given` }

  const problems: string[] = []
  const items: Record<string, unknown>[] = []
  const child = collection.name === 'CustomFields' ? undefined : catalog.find(collection.type)

  raw.forEach((item, i) => {
    const itemLabel = `${label}[${i}]`
    if (!isRecord(item)) {
      problems.push(`${itemLabel}: expected an object`)
      return
    }
    if (collection.name === 'CustomFields') {
      if (typeof item.Name !== 'string' || !item.Name) problems.push(`${itemLabel}: custom field needs a "Name"`)
      else items.push({ Name: item.Name, Value: item.Value ?? null })
      return
    }
    const keys = Object.keys(item)
    if (keys.length === 1 && keys[0]?.toLowerCase() === 'id') {
      const ref = prepareReference(item, itemLabel)
      if (!ref.ok) problems.push(ref.message)
      else items.push(ref.value as { Id: number })
      return
    }
    if (!child?.available) {
      items.push(item)
      return
    }
    const nested = prepareBody(catalog, child, item, 'create', itemLabel)
    if (!nested.ok) problems.push(nested.message)
    else items.push(nested.value)
  })

  if (problems.length > 0) return { ok: false, message: problems.join('\n- ') }
  return { ok: true, value: ARRAY_COLLECTIONS.has(collection.name.toLowerCase()) ? items : { Items: items } }
}

/** Requested scalar values or reference ids that the read-back does not show. */
export function unpersisted(requested: Record<string, unknown>, returned: unknown): string[] {
  if (!isRecord(returned)) return []
  const missing: string[] = []
  for (const [key, want] of Object.entries(requested)) {
    if (!(key in returned)) continue
    const got = returned[key]
    if (want === null) {
      if (got !== null) missing.push(`${key}: requested null, got ${JSON.stringify(got)}`)
    } else if (isRecord(want) && 'Id' in want && Object.keys(want).length === 1) {
      const gotId = isRecord(got) ? got.Id : undefined
      if (gotId !== want.Id) missing.push(`${key}: requested Id ${String(want.Id)}, got ${JSON.stringify(gotId ?? got)}`)
    } else if (typeof want === 'number' || typeof want === 'boolean') {
      if (got !== want) missing.push(`${key}: requested ${JSON.stringify(want)}, got ${JSON.stringify(got)}`)
    } else if (typeof want === 'string') {
      if (typeof got === 'string' && got !== want && !sameDate(want, got) && !sameMarkup(want, got)) {
        missing.push(`${key}: requested ${JSON.stringify(want)}, got ${JSON.stringify(got)}`)
      }
    }
  }
  return missing
}

function sameDate(want: string, got: string): boolean {
  return /^\d{4}-\d{2}-\d{2}/.test(want) && sameDateValue(want, got)
}

/** Targetprocess may wrap or entity-encode text it stores (e.g. descriptions in `<div>`). */
function sameMarkup(want: string, got: string): boolean {
  const plain = (s: string) =>
    s
      .replace(/<[^>]*>/g, '')
      .replace(/&#(\d+);/g, (_, n: string) => String.fromCharCode(Number(n)))
      .replace(/&nbsp;/g, ' ')
      .replace(/&amp;/g, '&')
      .replace(/&lt;/g, '<')
      .replace(/&gt;/g, '>')
      .replace(/&quot;/g, '"')
      .replace(/\s+/g, ' ')
      .trim()
  return plain(want) === plain(got)
}
