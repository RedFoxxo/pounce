import type { FetchLike } from '../../src/http/core.js'

export interface StubCall {
  method: string
  url: URL
  path: string
  query: URLSearchParams
  /** Parsed JSON body, the raw string, or the FormData. */
  body: unknown
  headers: Record<string, string>
}

/** Returned from a body function to answer with a specific status. */
export class StubReply {
  constructor(
    readonly status: number,
    readonly body: unknown,
  ) {}
}

export interface StubRoute {
  method?: 'GET' | 'POST' | 'DELETE'
  /** Exact path (`/api/v1/UserStories/1`) or a pattern. */
  path: string | RegExp
  /** Query parameters that must match exactly, or a predicate. */
  query?: Record<string, string> | ((query: URLSearchParams, call: StubCall) => boolean)
  status?: number
  /** Objects are sent as JSON; strings are sent as-is. A function builds the reply from the call. */
  body?: unknown | ((call: StubCall) => unknown)
  headers?: Record<string, string>
  /** How many times the route may answer; unlimited when omitted. */
  times?: number
  /** Reject the fetch instead of answering (network failure). */
  networkError?: string
}

/**
 * Stubbed `fetch` that answers with the payloads Targetprocess really returns,
 * so tests drive the real client. Unmatched requests answer 599 and are
 * recorded, so a test that forgot a route fails loudly.
 */
export class FetchStub {
  readonly calls: StubCall[] = []
  readonly unmatched: StubCall[] = []
  private readonly routes: (StubRoute & { used: number })[] = []

  on(route: StubRoute): this {
    this.routes.push({ ...route, used: 0 })
    return this
  }

  /** Registers a route that takes precedence over every route registered so far. */
  first(route: StubRoute): this {
    this.routes.unshift({ ...route, used: 0 })
    return this
  }

  get(path: string | RegExp, body: unknown, extra: Partial<StubRoute> = {}): this {
    return this.on({ method: 'GET', path, body, ...extra })
  }

  post(path: string | RegExp, body: unknown, extra: Partial<StubRoute> = {}): this {
    return this.on({ method: 'POST', path, body, ...extra })
  }

  delete(path: string | RegExp, body: unknown = '', extra: Partial<StubRoute> = {}): this {
    return this.on({ method: 'DELETE', path, body, ...extra })
  }

  /** Calls whose method and path match. */
  find(method: string, path: string | RegExp): StubCall[] {
    return this.calls.filter((c) => c.method === method && matches(path, c.path))
  }

  /** Requests that changed data. */
  get writes(): StubCall[] {
    return this.calls.filter((c) => c.method !== 'GET')
  }

  readonly fetch: FetchLike = async (input, init = {}) => {
    const url = new URL(input)
    const method = (init.method ?? 'GET').toUpperCase()
    let body: unknown = init.body
    if (typeof body === 'string') {
      try {
        body = JSON.parse(body)
      } catch {
        // keep the raw string
      }
    }
    const call: StubCall = {
      method,
      url,
      path: url.pathname,
      query: url.searchParams,
      body,
      headers: (init.headers ?? {}) as Record<string, string>,
    }
    this.calls.push(call)

    const route = this.routes.find(
      (r) =>
        (r.times === undefined || r.used < r.times) &&
        (r.method ?? 'GET') === method &&
        matches(r.path, call.path) &&
        queryMatches(r.query, call),
    )
    if (!route) {
      this.unmatched.push(call)
      return new Response(`UNSTUBBED ${method} ${call.path}?${call.query.toString()}`, { status: 599 })
    }
    route.used++
    if (route.networkError) throw new TypeError(route.networkError)
    let payload = await (typeof route.body === 'function' ? (route.body as (c: StubCall) => unknown)(call) : route.body)
    let status = route.status ?? 200
    if (payload instanceof StubReply) {
      status = payload.status
      payload = payload.body
    }
    const text = typeof payload === 'string' ? payload : JSON.stringify(payload ?? null)
    return new Response(text, {
      status,
      headers: { 'content-type': typeof payload === 'string' ? 'text/plain' : 'application/json', ...route.headers },
    })
  }
}

function matches(pattern: string | RegExp, path: string): boolean {
  return typeof pattern === 'string' ? pattern.toLowerCase() === path.toLowerCase() : pattern.test(path)
}

function queryMatches(query: StubRoute['query'], call: StubCall): boolean {
  if (!query) return true
  if (typeof query === 'function') return query(call.query, call)
  return Object.entries(query).every(([k, v]) => call.query.get(k) === v)
}
