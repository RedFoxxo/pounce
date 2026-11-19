import { readFileSync } from 'node:fs'
import type { Logger } from '../log.js'
import type { V1Client } from '../http/v1.js'
import { baselineEntry, loadIndex, loadResource, loadVersion, mapLimit, unlistedTargets, type IndexEntry, type LoadedResource, type Target } from './loader.js'
import type { CatalogCollection, CatalogData, CatalogField, CatalogResource } from './types.js'

export type CatalogSource = 'live' | 'snapshot'

export type Lookup<T> = { ok: true; value: T } | { ok: false; message: string }

/** Closest names first: prefix/substring matches, then small edit distance. */
export function suggestions(input: string, names: string[], max = 8): string[] {
  const q = input.toLowerCase()
  const threshold = Math.max(2, Math.floor(q.length / 3))
  const scored = names.map((name) => {
    const n = name.toLowerCase()
    const d = distance(q, n)
    const contains = n.includes(q)
    return { name, keep: d <= threshold || contains, score: d - (n.startsWith(q) ? 3 : 0) - (contains ? 1 : 0) }
  })
  return scored
    .filter((s) => s.keep)
    .sort((a, b) => a.score - b.score || a.name.localeCompare(b.name))
    .slice(0, max)
    .map((s) => s.name)
}

function distance(a: string, b: string): number {
  const row = Array.from({ length: b.length + 1 }, (_, i) => i)
  for (let i = 1; i <= a.length; i++) {
    let prev = row[0] as number
    row[0] = i
    for (let j = 1; j <= b.length; j++) {
      const tmp = row[j] as number
      row[j] = Math.min((row[j] as number) + 1, (row[j - 1] as number) + 1, prev + (a[i - 1] === b[j - 1] ? 0 : 1))
      prev = tmp
    }
  }
  return row[b.length] as number
}

export class Catalog {
  private readonly byKey = new Map<string, CatalogResource>()

  constructor(
    readonly data: CatalogData,
    readonly source: CatalogSource,
    /** Every resource name the instance lists, when `data` holds only the resources loaded so far. */
    private readonly names?: string[],
  ) {
    for (const r of data.resources) {
      this.byKey.set(r.name.toLowerCase(), r)
      this.byKey.set(r.path.toLowerCase(), r)
    }
  }

  get resources(): CatalogResource[] {
    return this.data.resources
  }

  /** By singular name (`UserStory`) or plural path (`UserStories`), case-insensitive. */
  find(nameOrPath: string): CatalogResource | undefined {
    return this.byKey.get(nameOrPath.trim().toLowerCase())
  }

  resource(nameOrPath: string): Lookup<CatalogResource> {
    const found = this.find(nameOrPath)
    if (found && found.available) return { ok: true, value: found }
    if (found) {
      return {
        ok: false,
        message: `Resource "${found.name}" is listed by the instance but its metadata is unavailable (/meta failed), so it cannot be used.`,
      }
    }
    const names = this.names ?? this.resources.filter((r) => r.available).map((r) => r.name)
    const close = suggestions(nameOrPath, names)
    return {
      ok: false,
      message:
        `Unknown resource "${nameOrPath}".` +
        (close.length ? ` Did you mean: ${close.join(', ')}?` : '') +
        ' Use read_meta without a resource to list them all.',
    }
  }

  /** Any value, reference or collection of the resource, by name (case-insensitive). */
  member(resource: CatalogResource, name: string): CatalogField | CatalogCollection | undefined {
    const n = name.toLowerCase()
    return (
      resource.values.find((f) => f.name.toLowerCase() === n) ??
      resource.references.find((f) => f.name.toLowerCase() === n) ??
      resource.collections.find((f) => f.name.toLowerCase() === n)
    )
  }

  collection(resource: CatalogResource, name: string): Lookup<CatalogCollection> {
    const n = name.toLowerCase()
    const found = resource.collections.find((c) => c.name.toLowerCase() === n)
    if (found) return { ok: true, value: found }
    const close = suggestions(
      name,
      resource.collections.map((c) => c.name),
    )
    return {
      ok: false,
      message:
        `${resource.name} has no collection "${name}".` +
        (close.length ? ` Did you mean: ${close.join(', ')}?` : '') +
        ` Collections: ${resource.collections.map((c) => c.name).join(', ') || '(none)'}.`,
    }
  }
}

/** The committed baseline catalog, used when the instance's metadata cannot be read. */
export function readSnapshot(): CatalogData {
  return JSON.parse(readFileSync(new URL('./snapshot.json', import.meta.url), 'utf8')) as CatalogData
}

/** The resources a tool needs, by name or path (`UserStory`, `UserStoryHistories`), or `all`. */
export type CatalogRequest = readonly string[] | 'all'

export interface CatalogProviderOptions {
  log?: Logger
  snapshot?: () => CatalogData
}

const CONCURRENCY = 8

/**
 * Reads the instance's metadata lazily: nothing at startup, then the index and
 * each resource's `/meta` the first time a tool asks for it (single-flight:
 * concurrent callers share one request). Answers are kept for the life of the
 * process; a failed request is not kept, so the next call tries again, and in
 * the meantime the committed snapshot stands in.
 */
export class CatalogProvider {
  private index?: Promise<IndexEntry[] | undefined>
  private version?: Promise<string | undefined>
  /** In-flight and settled loads, by lower-case path. Failed loads are removed once settled. */
  private readonly loads = new Map<string, Promise<LoadedResource>>()
  /** Settled final answers, by lower-case path. */
  private readonly known = new Map<string, CatalogResource>()
  private readonly log: Logger
  private readonly snapshot: () => CatalogData
  private snapshotData?: CatalogData | null

  constructor(
    private readonly v1: V1Client,
    options: CatalogProviderOptions = {},
  ) {
    this.log = options.log ?? (() => {})
    this.snapshot = options.snapshot ?? readSnapshot
  }

  /** A catalog holding at least the requested resources (and every one loaded before). */
  async get(request: CatalogRequest = []): Promise<Catalog> {
    const entries = await this.loadIndex()
    if (!entries) {
      this.log('catalog: the resource index could not be read; using the snapshot for this call')
      return new Catalog(this.safeSnapshot() ?? { instance: 'none', capturedAt: '', resources: [] }, 'snapshot')
    }
    const targets =
      request === 'all'
        ? [...entries.map((e) => ({ ...e, listed: true })), ...unlistedTargets(entries)]
        : request.flatMap((name) => target(name, entries) ?? [])
    const loaded = await mapLimit(targets, CONCURRENCY, (t) => this.loadResource(t))
    const resources = new Map(this.known)
    for (const l of loaded) if (l.resource) resources.set(l.resource.path.toLowerCase(), l.resource)
    const version = request === 'all' ? await this.loadVersion() : undefined
    const data: CatalogData = {
      instance: this.v1.http.baseUrl,
      ...(version ? { version } : {}),
      capturedAt: new Date().toISOString(),
      resources: [...resources.values()].sort((a, b) => a.name.localeCompare(b.name)),
    }
    return new Catalog(data, loaded.some((l) => l.fromFallback) ? 'snapshot' : 'live', entries.map((e) => e.name))
  }

  /** The snapshot, read once on first need (it is about 1 MB of JSON). */
  private safeSnapshot(): CatalogData | undefined {
    if (this.snapshotData === undefined) {
      try {
        this.snapshotData = this.snapshot()
      } catch (error) {
        this.log(`catalog: snapshot unreadable (${error instanceof Error ? error.message : String(error)})`)
        this.snapshotData = null
      }
    }
    return this.snapshotData ?? undefined
  }

  private loadIndex(): Promise<IndexEntry[] | undefined> {
    this.index ??= loadIndex(this.v1).then((r) => {
      if (r.ok) return r.data
      this.log(`catalog: Index/meta failed (${r.message})`)
      this.index = undefined
      return undefined
    })
    return this.index
  }

  private loadVersion(): Promise<string | undefined> {
    this.version ??= loadVersion(this.v1).then((v) => {
      if (v === undefined) this.version = undefined
      return v
    })
    return this.version
  }

  private loadResource(t: Target): Promise<LoadedResource> {
    const key = t.path.toLowerCase()
    let pending = this.loads.get(key)
    if (!pending) {
      pending = loadResource(this.v1, t, { log: this.log, fallback: (path) => baselineEntry(this.safeSnapshot(), path) }).then((l) => {
        if (!l.final) this.loads.delete(key)
        else if (l.resource) this.known.set(key, l.resource)
        return l
      })
      this.loads.set(key, pending)
    }
    return pending
  }
}

/**
 * What to load for a name: its index entry (by name or path), or an unlisted
 * history (`UserStoryHistory`, `UserStorySimpleHistories`) or `GeneralConversions`.
 * Undefined when the instance has no such resource.
 */
function target(name: string, entries: IndexEntry[]): Target | undefined {
  const n = name.trim()
  const entry = entries.find((e) => e.name.toLowerCase() === n.toLowerCase() || e.path.toLowerCase() === n.toLowerCase())
  if (entry) return { ...entry, listed: true }
  const history = /^(.+?)(simple)?histor(?:y|ies)$/i.exec(n)
  const base = history && entries.find((e) => e.name.toLowerCase() === history[1]!.toLowerCase())
  if (base) {
    const path = `${base.name}${history[2] ? 'Simple' : ''}Histories`
    return { name: path, path, listed: false }
  }
  if (/^generalconversions?$/i.test(n)) return { name: 'GeneralConversions', path: 'GeneralConversions', listed: false }
  return undefined
}
