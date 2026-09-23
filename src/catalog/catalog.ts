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
  /** Serve the snapshot if the live catalog takes longer than this (default 25 s); the live one replaces it when it arrives. */
  timeoutMs?: number
  snapshot?: () => CatalogData
}

/**
 * Loads the live catalog once (in the background from `start()`), caching it in
 * memory. Falls back to the committed snapshot if the instance's metadata is
 * unavailable or slow; a live catalog that arrives late replaces the snapshot.
 * A failed load is never cached.
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
    this.timeoutMs = options.timeoutMs ?? 25_000
    this.snapshot = options.snapshot ?? readSnapshot
  }

  start(): void {
    this.get().catch((error: unknown) => {
      this.log(`catalog: no catalog available (${error instanceof Error ? error.message : String(error)}); tools needing it will report this`)
    })
  }

  get(): Promise<Catalog> {
    if (!this.pending) {
      const loading = this.load()
      this.pending = loading
      loading.catch(() => {
        if (this.pending === loading) this.pending = undefined
      })
    }
    return this.pending
  }

  private safeSnapshot(): CatalogData | undefined {
    try {
      return this.snapshot()
    } catch {
      return undefined
    }
  }

  private async load(): Promise<Catalog> {
    const live = loadCatalog(this.v1, { log: this.log, fallback: () => this.safeSnapshot() }).catch((error: unknown) => {
      this.log(`catalog: live load failed (${error instanceof Error ? error.message : String(error)})`)
      return undefined
    })
    let timer: NodeJS.Timeout | undefined
    const timeout = new Promise<'timeout'>((resolve) => {
      timer = setTimeout(() => resolve('timeout'), this.timeoutMs)
      timer.unref?.()
    })
    const first = await Promise.race([live, timeout])
    if (timer) clearTimeout(timer)

    if (first !== 'timeout' && first?.ok) {
      this.log(`catalog: loaded ${first.data.resources.length} resources from the instance`)
      return new Catalog(first.data, 'live')
    }
    if (first === 'timeout') {
      // Keep the snapshot for now and swap the live catalog in once it arrives.
      void live.then((late) => {
        if (late?.ok) {
          this.pending = Promise.resolve(new Catalog(late.data, 'live'))
          this.log(`catalog: live catalog arrived late (${late.data.resources.length} resources); now in use`)
        }
      })
    }
    this.log(`catalog: live metadata unavailable (${first === 'timeout' ? `not loaded within ${this.timeoutMs} ms` : first ? first.message : 'error'}); using snapshot`)
    return new Catalog(this.snapshot(), 'snapshot')
  }
}
