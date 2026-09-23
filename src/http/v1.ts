import { normalizeV1Dates } from '../format/dates.js'
import type { HttpCore, Query } from './core.js'
import { ok, type Err, type Ok, type Result } from './result.js'

/** A v1 collection page, as Targetprocess sends it. Only this module reads `Items`/`Next`. */
interface V1Page<T> {
  Items?: T[]
  Next?: string
}

/** An inner collection inside an entity, e.g. `Assignments` when included. */
export interface V1Inner<T> {
  Items?: T[]
}

/** Items of an included inner collection; `[]` when absent. */
export function innerItems<T>(collection: V1Inner<T> | null | undefined): T[] {
  return collection?.Items ?? []
}

/** Reference to another entity as v1 returns it. */
export interface V1Ref {
  ResourceType?: string
  Id: number
  Name?: string
}

export interface V1ReadOptions {
  include?: string
  exclude?: string
  append?: string
  innerTake?: number
}

export interface V1ListOptions extends V1ReadOptions {
  where?: string
  orderBy?: string
  orderByDesc?: string
  /** Hard cap on items returned. Default 5000. Reaching it with more pages left sets `truncated`. */
  limit?: number
}

export interface V1WriteOptions {
  resultInclude?: string
  resultExclude?: string
  resultAppend?: string
}

export interface ListResult<T> {
  items: T[]
  /** True when more items exist than were returned. */
  truncated: boolean
}

/** Targetprocess ignores anything above 1000. */
export const PAGE_SIZE = 1000
export const DEFAULT_LIST_LIMIT = 5000
export const BULK_LIMIT = 500

const V1 = '/api/v1'

/** A string literal for a v1 `where` clause; quotes and backslashes are backslash-escaped (**live**-verified). */
export function v1String(value: string): string {
  return `'${value.replace(/\\/g, '\\\\').replace(/'/g, "\\'")}'`
}

function seg(part: string | number): string {
  return encodeURIComponent(String(part))
}

/** `UserStories/123/Tasks` → `/api/v1/UserStories/123/Tasks`, each segment encoded. */
export function v1Path(...parts: (string | number)[]): string {
  return `${V1}/${parts.flatMap((p) => String(p).split('/')).filter((p) => p && p !== '.' && p !== '..').map(seg).join('/')}`
}

function readQuery(options: V1ReadOptions = {}): Query {
  return {
    format: 'json',
    include: options.include,
    exclude: options.exclude,
    append: options.append,
    innerTake: options.innerTake,
  }
}

function writeQuery(options: V1WriteOptions = {}): Query {
  return {
    format: 'json',
    resultFormat: 'json',
    resultInclude: options.resultInclude,
    resultExclude: options.resultExclude,
    resultAppend: options.resultAppend,
  }
}

function normalized<T>(result: Result<T>): Result<T> {
  return result.ok ? ok(normalizeV1Dates(result.data), result.status) : result
}

/** REST v1 client. Every method returns a `Result`; dates are ISO 8601. */
export class V1Client {
  constructor(readonly http: HttpCore) {}

  /** GET any path below `/api/v1`, e.g. `Users/LoggedUser`, `Context`, `UserStories/meta`. */
  async getPath<T>(path: string, query: Query = {}): Promise<Result<T>> {
    return normalized(await this.http.request<T>({ method: 'GET', path: v1Path(path), query: { format: 'json', ...query } }))
  }

  /** GET the raw text of a path (for payloads JSON.parse cannot represent, e.g. `Index/meta`). */
  async getText(path: string, query: Query = {}): Promise<Result<string>> {
    return this.http.request<string>({ method: 'GET', path: v1Path(path), query: { format: 'json', ...query }, expect: 'text' })
  }

  async get<T>(collection: string, id: number, options?: V1ReadOptions): Promise<Result<T>> {
    return normalized(await this.http.request<T>({ method: 'GET', path: v1Path(collection, id), query: readQuery(options) }))
  }

  /** GET a collection (or inner collection path), following pages until exhausted or `limit` is reached. */
  async list<T>(path: string, options: V1ListOptions = {}): Promise<Result<ListResult<T>>> {
    const limit = Math.max(1, options.limit ?? DEFAULT_LIST_LIMIT)
    const items: T[] = []
    let skip = 0
    for (;;) {
      const take = Math.min(PAGE_SIZE, limit - items.length)
      const page = await this.http.request<V1Page<T>>({
        method: 'GET',
        path: v1Path(path),
        query: {
          ...readQuery(options),
          where: options.where,
          orderBy: options.orderBy,
          orderByDesc: options.orderByDesc,
          take,
          skip,
        },
      })
      if (!page.ok) return page
      const pageItems = page.data?.Items ?? []
      items.push(...normalizeV1Dates(pageItems))
      const more = Boolean(page.data?.Next) && pageItems.length > 0
      if (!more) return ok({ items, truncated: false }, page.status)
      if (items.length >= limit) return ok({ items, truncated: true }, page.status)
      skip += pageItems.length
    }
  }

  /** POST a new entity. The body must not carry an `Id` (that would update instead). */
  async create<T>(collection: string, body: Record<string, unknown>, options?: V1WriteOptions): Promise<Result<T>> {
    return normalized(
      await this.http.request<T>({ method: 'POST', path: v1Path(collection), query: writeQuery(options), json: body }),
    )
  }

  /** POST an update to an existing entity. */
  async update<T>(
    collection: string,
    id: number,
    body: Record<string, unknown>,
    options?: V1WriteOptions,
  ): Promise<Result<T>> {
    return normalized(
      await this.http.request<T>({
        method: 'POST',
        path: v1Path(collection, id),
        query: writeQuery(options),
        json: { ...body, Id: id },
      }),
    )
  }

  /** POST up to 500 entities to `/{collection}/bulk` (creates and updates). */
  async bulk<T>(collection: string, items: Record<string, unknown>[], options?: V1WriteOptions): Promise<Result<T>> {
    return normalized(
      await this.http.request<T>({ method: 'POST', path: v1Path(collection, 'bulk'), query: writeQuery(options), json: items }),
    )
  }

  async delete(collection: string, id: number): Promise<Result<unknown>> {
    return this.http.request({ method: 'DELETE', path: v1Path(collection, id), query: { format: 'json' } })
  }

  async deleteBulk(collection: string, ids: number[]): Promise<Result<unknown>> {
    return this.http.request({
      method: 'DELETE',
      path: v1Path(collection, 'bulk'),
      query: { format: 'json' },
      json: ids.map((id) => ({ Id: id })),
    })
  }

  /**
   * DELETE items from an inner collection, one `/{collection}/{id}/{inner}/{childId}`
   * per child. The documented `?childrenIds=1,2` form answers 500 on a live
   * instance; the path form works. Stops at the first failure and reports the
   * children already removed.
   */
  async removeFromCollection(
    collection: string,
    id: number,
    inner: string,
    childIds: number[],
  ): Promise<Ok<{ removed: number[] }> | (Err & { removed: number[] })> {
    const removed: number[] = []
    for (const child of childIds) {
      const r = await this.http.request({ method: 'DELETE', path: v1Path(collection, id, inner, child), query: { format: 'json' } })
      if (!r.ok) return { ...r, removed }
      removed.push(child)
    }
    return ok({ removed })
  }

  /** POST any path below `/api/v1` (e.g. `undelete`). */
  async postPath<T>(path: string, body: unknown, query: Query = {}): Promise<Result<T>> {
    return normalized(
      await this.http.request<T>({ method: 'POST', path: v1Path(path), query: { format: 'json', ...query }, json: body }),
    )
  }
}
