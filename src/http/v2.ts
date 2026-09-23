import { normalizeV1Dates } from '../format/dates.js'
import type { HttpCore, Query } from './core.js'
import { ok, type Result } from './result.js'

/** A v2 page, as Targetprocess sends it. Only this module reads `items`/`next`. */
interface V2Page<T> {
  items?: T[]
  next?: string
}

export interface V2QueryOptions {
  select?: string
  where?: string
  orderBy?: string
  filter?: string
  includeDeleted?: boolean
  /** Hard cap on items returned. Default 1000. */
  limit?: number
}

export interface V2ListResult<T> {
  items: T[]
  truncated: boolean
}

const PAGE = 1000

function seg(part: string): string {
  return encodeURIComponent(part)
}

/**
 * REST v2 (read-only): camelCase, null fields omitted (absence means null),
 * ISO dates requested with `isoDate=true`. Filter syntax (`==`, `and`, `or`)
 * differs from v1; never share filter strings.
 */
export class V2Client {
  constructor(readonly http: HttpCore) {}

  /** `/api/v2/{entity}` fully paged; the query is sent unchanged on every page (TP's `next` drops some parameters). */
  async query<T>(entity: string, options: V2QueryOptions = {}, base = '/api/v2'): Promise<Result<V2ListResult<T>>> {
    const limit = Math.max(1, options.limit ?? PAGE)
    const items: T[] = []
    let skip = 0
    for (;;) {
      const take = Math.min(PAGE, limit - items.length)
      const query: Query = {
        select: options.select,
        where: options.where,
        orderBy: options.orderBy,
        filter: options.filter,
        includeDeleted: options.includeDeleted ? 'true' : undefined,
        isoDate: 'true',
        take,
        skip,
      }
      const page = await this.http.request<V2Page<T>>({ method: 'GET', path: `${base}/${seg(entity)}`, query })
      if (!page.ok) return page
      const got = page.data?.items ?? []
      items.push(...normalizeV1Dates(got))
      if (!page.data?.next || got.length === 0) return ok({ items, truncated: false }, page.status)
      if (items.length >= limit) return ok({ items, truncated: true }, page.status)
      skip += got.length
    }
  }

  /** An aggregation (`result=`) returns one object rather than items. */
  async aggregate<T>(entity: string, result: string, options: { where?: string; filter?: string } = {}): Promise<Result<T>> {
    const r = await this.http.request<T>({
      method: 'GET',
      path: `/api/v2/${seg(entity)}`,
      query: { result, where: options.where, filter: options.filter, isoDate: 'true' },
    })
    return r.ok ? ok(normalizeV1Dates(r.data), r.status) : r
  }

  /** Full history v2: `/api/history/v2/{Entity}`. */
  history<T>(entity: string, options: V2QueryOptions = {}): Promise<Result<V2ListResult<T>>> {
    return this.query<T>(entity, options, '/api/history/v2')
  }
}
