import { readFileSync } from 'node:fs'
import type { Logger } from '../log.js'
import type { V1Client } from '../http/v1.js'
import { loadCatalog } from './loader.js'
import type { CatalogCollection, CatalogData, CatalogField, CatalogResource } from './types.js'

export type CatalogSource = 'live' | 'snapshot'

export type Lookup<T> = { ok: true; value: T } | { ok: false; message: string }

/** Closest names first: prefix/substring matches, then small edit distance. */
export function suggestions(input: string, names: string[], max = 8): string[] {
  const q = input.toLowerCase()
  const scored = names.map((name) => {
    const n = name.toLowerCase()
    let score = distance(q, n)
    if (n.startsWith(q) || q.startsWith(n)) score -= 100
    else if (n.includes(q) || q.includes(n)) score -= 50
    return { name, score }
  })
  return scored
    .filter((s) => s.score <= Math.max(3, Math.floor(q.length / 2)))
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
    const names = this.resources.filter((r) => r.available).map((r) => r.name)
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

export interface CatalogProviderOptions {
  log?: Logger
  /** Give up on the live catalog after this long and use the snapshot. Default 60 s. */
  timeoutMs?: number
  snapshot?: () => CatalogData
}

/**
 * Loads the live catalog once (in the background from `start()`), caching it in
 * memory; falls back to the committed snapshot if the instance's metadata is
 * unavailable.
 */
export class CatalogProvider {
  private pending?: Promise<Catalog>
  private readonly log: Logger
  private readonly timeoutMs: number
  private readonly snapshot: () => CatalogData

  constructor(
    private readonly v1: V1Client,
    options: CatalogProviderOptions = {},
  ) {
    this.log = options.log ?? (() => {})
    this.timeoutMs = options.timeoutMs ?? 60_000
    this.snapshot = options.snapshot ?? readSnapshot
  }

  start(): void {
    void this.get()
  }

  get(): Promise<Catalog> {
    this.pending ??= this.load()
    return this.pending
  }

  private async load(): Promise<Catalog> {
    let timer: NodeJS.Timeout | undefined
    const timeout = new Promise<'timeout'>((resolve) => {
      timer = setTimeout(() => resolve('timeout'), this.timeoutMs)
      timer.unref?.()
    })
    try {
      const live = await Promise.race([loadCatalog(this.v1, { log: this.log }), timeout])
      if (live !== 'timeout' && live.ok) {
        this.log(`catalog: loaded ${live.data.resources.length} resources from the instance`)
        return new Catalog(live.data, 'live')
      }
      this.log(`catalog: live metadata unavailable (${live === 'timeout' ? 'timed out' : live.message}); using snapshot`)
    } catch (error) {
      this.log(`catalog: live load failed (${error instanceof Error ? error.message : String(error)}); using snapshot`)
    } finally {
      if (timer) clearTimeout(timer)
    }
    return new Catalog(this.snapshot(), 'snapshot')
  }
}
