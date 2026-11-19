import type { Logger } from '../log.js'
import { err, ok, type Result } from '../http/result.js'
import type { V1Client } from '../http/v1.js'
import type { CatalogCollection, CatalogData, CatalogField, CatalogResource } from './types.js'

export interface IndexEntry {
  name: string
  path: string
  description: string
}

/**
 * `/api/v1/Index/meta` repeats the key `"ResourceMetadataDescription"` once per
 * resource, so `JSON.parse` would keep only the last one. Scan the raw text and
 * parse each occurrence's object on its own.
 */
export function parseIndex(text: string): IndexEntry[] {
  const key = '"ResourceMetadataDescription":'
  const entries: IndexEntry[] = []
  let from = 0
  for (;;) {
    const at = text.indexOf(key, from)
    if (at < 0) break
    const start = text.indexOf('{', at + key.length)
    if (start < 0) break
    const end = matchingBrace(text, start)
    if (end < 0) break
    from = end + 1
    try {
      const raw = JSON.parse(text.slice(start, end + 1)) as { Name?: string; Uri?: string; Description?: string }
      if (!raw.Name || !raw.Uri) continue
      entries.push({ name: raw.Name, path: pathFromUri(raw.Uri), description: raw.Description ?? '' })
    } catch {
      // skip one malformed entry rather than losing the whole index
    }
  }
  return entries
}

function matchingBrace(text: string, start: number): number {
  let depth = 0
  let inString = false
  for (let i = start; i < text.length; i++) {
    const c = text[i]
    if (inString) {
      if (c === '\\') i++
      else if (c === '"') inString = false
      continue
    }
    if (c === '"') inString = true
    else if (c === '{') depth++
    else if (c === '}') {
      depth--
      if (depth === 0) return i
    }
  }
  return -1
}

/** `https://x/api/v1/UserStories/meta` or `https://x/api/v1/UserStories` → `UserStories`. */
export function pathFromUri(uri: string): string {
  const after = uri.split('/api/v1/')[1] ?? uri
  return after.split(/[/?]/)[0] ?? after
}

interface RawField {
  Name?: string
  Type?: string
  CanSet?: boolean
  IsRequired?: boolean
  IsDeprecated?: boolean
  CanAdd?: boolean
  CanRemove?: boolean
}

export interface RawMeta {
  Name?: string
  Uri?: string
  Description?: string
  CanCreate?: boolean
  CanUpdate?: boolean
  CanDelete?: boolean
  ResourceMetadataHierarchyDescription?: {
    ResourceMetadataBaseResourceDescription?: { Items?: { Name?: string }[] }
  }
  ResourceMetadataPropertiesDescription?: {
    ResourceMetadataPropertiesResourceValuesDescription?: { Items?: RawField[] }
    ResourceMetadataPropertiesResourceReferencesDescription?: { Items?: RawField[] }
    ResourceMetadataPropertiesResourceCollectionsDescription?: { Items?: RawField[] }
  }
}

function xmlAttributes(tag: string): Record<string, string> {
  const attrs: Record<string, string> = {}
  for (const m of tag.matchAll(/([A-Za-z]+)="([^"]*)"/g)) attrs[m[1] as string] = decodeXml(m[2] as string)
  return attrs
}

function decodeXml(value: string): string {
  return value
    .replace(/&lt;/g, '<')
    .replace(/&gt;/g, '>')
    .replace(/&quot;/g, '"')
    .replace(/&apos;/g, "'")
    .replace(/&#(\d+);/g, (_, n: string) => String.fromCharCode(Number(n)))
    .replace(/&amp;/g, '&')
}

function xmlBool(value: string | undefined): boolean | undefined {
  return value === undefined ? undefined : value === 'true'
}

function xmlFields(xml: string, section: string): RawField[] {
  const body = new RegExp(`<${section}>([\\s\\S]*?)</${section}>`).exec(xml)?.[1] ?? ''
  return [...body.matchAll(/<[A-Za-z]+\s[^>]*?\/>/g)].map((m) => {
    const a = xmlAttributes(m[0])
    const raw: RawField = {}
    if (a.Name !== undefined) raw.Name = a.Name
    if (a.Type !== undefined) raw.Type = a.Type
    const flags = { CanSet: a.CanSet, IsRequired: a.IsRequired, IsDeprecated: a.IsDeprecated, CanAdd: a.CanAdd, CanRemove: a.CanRemove }
    for (const [key, value] of Object.entries(flags)) {
      const b = xmlBool(value)
      if (b !== undefined) raw[key as 'CanSet'] = b
    }
    return raw
  })
}

/**
 * Some `/meta` endpoints (e.g. `Context/meta`) answer XML regardless of
 * `format=json`. Converts that XML to the JSON shape.
 */
export function parseMetaXml(xml: string): RawMeta | undefined {
  const root = /<ResourceMetadataDescription\s[^>]*>/.exec(xml)?.[0]
  if (!root) return undefined
  const a = xmlAttributes(root)
  const bases = [...xml.matchAll(/<ResourceMetadataBaseResourceItemDescription\s[^>]*?\/>/g)]
    .map((m) => xmlAttributes(m[0]).Name)
    .filter((n): n is string => Boolean(n))
    .map((Name) => ({ Name }))
  const meta: RawMeta = {
    ResourceMetadataHierarchyDescription: { ResourceMetadataBaseResourceDescription: { Items: bases } },
    ResourceMetadataPropertiesDescription: {
      ResourceMetadataPropertiesResourceValuesDescription: { Items: xmlFields(xml, 'ResourceMetadataPropertiesResourceValuesDescription') },
      ResourceMetadataPropertiesResourceReferencesDescription: { Items: xmlFields(xml, 'ResourceMetadataPropertiesResourceReferencesDescription') },
      ResourceMetadataPropertiesResourceCollectionsDescription: { Items: xmlFields(xml, 'ResourceMetadataPropertiesResourceCollectionsDescription') },
    },
  }
  if (a.Name !== undefined) meta.Name = a.Name
  if (a.Uri !== undefined) meta.Uri = a.Uri
  if (a.Description !== undefined) meta.Description = a.Description
  const flags = { CanCreate: xmlBool(a.CanCreate), CanUpdate: xmlBool(a.CanUpdate), CanDelete: xmlBool(a.CanDelete) }
  for (const [key, value] of Object.entries(flags)) if (value !== undefined) meta[key as 'CanCreate'] = value
  return meta
}

/** GET `/{path}/meta`, accepting JSON or the XML some endpoints insist on. */
async function fetchMeta(v1: V1Client, path: string): Promise<Result<RawMeta>> {
  const text = await v1.getText(`${path}/meta`)
  if (!text.ok) return text
  const body = text.data.trim()
  if (body.startsWith('{')) {
    try {
      return ok(JSON.parse(body) as RawMeta, text.status)
    } catch {
      return err(text.status, `Unreadable JSON metadata for ${path}`, body.slice(0, 2000))
    }
  }
  const xml = parseMetaXml(body)
  return xml ? ok(xml, text.status) : err(text.status, `Unreadable metadata for ${path}`, body.slice(0, 2000))
}

function field(raw: RawField): CatalogField {
  return {
    name: raw.Name ?? '',
    type: raw.Type ?? '',
    canSet: raw.CanSet === true,
    required: raw.IsRequired === true,
    deprecated: raw.IsDeprecated === true,
  }
}

function collection(raw: RawField): CatalogCollection {
  return { ...field(raw), canAdd: raw.CanAdd === true, canRemove: raw.CanRemove === true }
}

/** Converts one `/{Resource}/meta` payload. */
export function parseMeta(raw: RawMeta, fallback: { name: string; path: string; listed: boolean; description?: string }): CatalogResource {
  const props = raw.ResourceMetadataPropertiesDescription
  const named = (items: RawField[] | undefined) => (items ?? []).filter((i) => i.Name)
  return {
    name: raw.Name ?? fallback.name,
    path: raw.Uri ? pathFromUri(raw.Uri) : fallback.path,
    description: raw.Description ?? fallback.description ?? '',
    listed: fallback.listed,
    available: true,
    canCreate: raw.CanCreate === true,
    canUpdate: raw.CanUpdate === true,
    canDelete: raw.CanDelete === true,
    bases: (raw.ResourceMetadataHierarchyDescription?.ResourceMetadataBaseResourceDescription?.Items ?? [])
      .map((i) => i.Name)
      .filter((n): n is string => Boolean(n)),
    values: named(props?.ResourceMetadataPropertiesResourceValuesDescription?.Items).map(field),
    references: named(props?.ResourceMetadataPropertiesResourceReferencesDescription?.Items).map(field),
    collections: named(props?.ResourceMetadataPropertiesResourceCollectionsDescription?.Items).map(collection),
  }
}

/** A resource to read `/meta` for: an index entry, or a probed unlisted one. */
export interface Target {
  name: string
  path: string
  listed: boolean
  description?: string
}

function unavailable(target: Target): CatalogResource {
  return {
    name: target.name,
    path: target.path,
    description: target.description ?? '',
    listed: true,
    available: false,
    canCreate: false,
    canUpdate: false,
    canDelete: false,
    bases: [],
    values: [],
    references: [],
    collections: [],
  }
}

export async function mapLimit<T, R>(items: T[], limit: number, fn: (item: T) => Promise<R>): Promise<R[]> {
  const out = new Array<R>(items.length)
  let next = 0
  const workers = Array.from({ length: Math.min(limit, items.length) }, async () => {
    for (;;) {
      const i = next++
      if (i >= items.length) return
      out[i] = await fn(items[i] as T)
    }
  })
  await Promise.all(workers)
  return out
}

/** One retry for transient failures; a 404 means the resource does not exist here. */
async function fetchMetaRetrying(v1: V1Client, path: string): Promise<Result<RawMeta>> {
  const first = await fetchMeta(v1, path)
  if (first.ok || first.status === 404) return first
  return fetchMeta(v1, path)
}

/** `/api/v1/Index/meta`: every listed resource. */
export async function loadIndex(v1: V1Client): Promise<Result<IndexEntry[]>> {
  const index = await v1.getText('Index/meta')
  if (!index.ok) return index
  const entries = parseIndex(index.data)
  if (entries.length === 0) return err(index.status, 'The resource index was empty or unreadable', index.data.slice(0, 2000))
  return ok(entries, index.status)
}

/**
 * Resources the index does not list but that answer `/meta` (56 on our
 * instance): every `{X}Histories` and `{X}SimpleHistories`, and `GeneralConversions`.
 */
export function unlistedTargets(entries: IndexEntry[]): Target[] {
  const listed = new Set(entries.map((e) => e.path.toLowerCase()))
  return [...entries.flatMap((e) => [`${e.name}Histories`, `${e.name}SimpleHistories`]), 'GeneralConversions']
    .filter((path) => !listed.has(path.toLowerCase()))
    .map((path) => ({ name: path, path, listed: false }))
}

export interface LoadedResource {
  /** Undefined when an unlisted resource does not exist here. */
  resource: CatalogResource | undefined
  /** False when `/meta` failed for a reason other than 404: the answer may change, so it is not kept. */
  final: boolean
  /** The resource came from the fallback (snapshot), not from the instance. */
  fromFallback?: true
}

/**
 * One resource's `/meta`. A 404 is final: a listed resource becomes
 * unavailable (e.g. SickLeave), an unlisted one does not exist. Any other
 * failure uses the fallback entry when there is one.
 */
export async function loadResource(
  v1: V1Client,
  target: Target,
  options: { log?: Logger; fallback?: (path: string) => CatalogResource | undefined } = {},
): Promise<LoadedResource> {
  const meta = await fetchMetaRetrying(v1, target.path)
  if (meta.ok && meta.data) return { resource: parseMeta(meta.data, target), final: true }
  if (!meta.ok && meta.status === 404) return { resource: target.listed ? unavailable(target) : undefined, final: true }
  const log = options.log ?? (() => {})
  const reason = meta.ok ? 'empty' : meta.message
  const known = options.fallback?.(target.path)
  if (known) {
    log(`catalog: ${target.path}/meta failed (${reason}); using the snapshot entry`)
    return { resource: { ...known, listed: target.listed }, final: false, fromFallback: true }
  }
  log(`catalog: ${target.path}/meta failed (${reason}); ${target.listed ? 'marked unavailable' : 'skipped'}`)
  return { resource: target.listed ? unavailable(target) : undefined, final: false }
}

/** The Targetprocess version `Context` reports, when readable. */
export async function loadVersion(v1: V1Client): Promise<string | undefined> {
  const context = await v1.getPath<{ Version?: string }>('Context', { include: '[Version]' })
  return context.ok ? context.data?.Version : undefined
}

/** The entry of a path in a baseline catalog, when it is usable. */
export function baselineEntry(baseline: CatalogData | undefined, path: string): CatalogResource | undefined {
  return baseline?.resources.find((r) => r.path.toLowerCase() === path.toLowerCase() && r.available)
}

export interface LoadOptions {
  concurrency?: number
  log?: Logger
  now?: () => Date
  /** Baseline used for a resource whose /meta fails for a reason other than 404 (after one retry). */
  fallback?: () => CatalogData | undefined
}

/**
 * Reads the whole catalog from the instance (`npm run snapshot`): the index,
 * every listed resource's `/meta`, plus the unlisted ones. One broken resource
 * never fails the load; a failed index does.
 */
export async function loadCatalog(v1: V1Client, options: LoadOptions = {}): Promise<Result<CatalogData>> {
  let baseline: CatalogData | undefined | null = null
  const fallback = (path: string) => {
    if (baseline === null) baseline = options.fallback?.()
    return baselineEntry(baseline, path)
  }
  const index = await loadIndex(v1)
  if (!index.ok) return index
  const targets = [...index.data.map((e) => ({ ...e, listed: true })), ...unlistedTargets(index.data)]
  const loaded = await mapLimit(targets, options.concurrency ?? 8, (t) =>
    loadResource(v1, t, { fallback, ...(options.log ? { log: options.log } : {}) }),
  )
  const resources = loaded.flatMap((l) => (l.resource ? [l.resource] : [])).sort((a, b) => a.name.localeCompare(b.name))
  const version = await loadVersion(v1)
  return ok({
    instance: v1.http.baseUrl,
    ...(version ? { version } : {}),
    capturedAt: (options.now?.() ?? new Date()).toISOString(),
    resources,
  })
}
