import type { Logger } from '../log.js'
import { redact } from './redact.js'
import { err, ok, type Result } from './result.js'

export type FetchLike = (input: string, init?: RequestInit) => Promise<Response>

export interface HttpOptions {
  baseUrl: string
  token: string
  fetch?: FetchLike
  log?: Logger
  timeoutMs?: number
}

export type QueryValue = string | number | boolean | undefined | null
export type Query = Record<string, QueryValue>

export interface RequestSpec {
  method: 'GET' | 'POST' | 'DELETE'
  /** Path below the instance root, starting with `/`. */
  path: string
  query?: Query
  /** JSON request body. */
  json?: unknown
  /** Multipart request body (attachment upload). */
  form?: FormData
  /** `json` (default) requires a JSON response; `text` returns the raw body. */
  expect?: 'json' | 'text'
}

const DEFAULT_TIMEOUT_MS = 60_000

/**
 * The single place where pounce talks to Targetprocess. Adds the token, logs
 * the redacted request to stderr, and turns every outcome into a `Result`.
 */
export class HttpCore {
  readonly baseUrl: string
  private readonly token: string
  private readonly fetchImpl: FetchLike
  private readonly log: Logger
  private readonly timeoutMs: number

  constructor(options: HttpOptions) {
    this.baseUrl = options.baseUrl
    this.token = options.token
    this.fetchImpl = options.fetch ?? ((input, init) => fetch(input, init))
    this.log = options.log ?? (() => {})
    this.timeoutMs = options.timeoutMs ?? DEFAULT_TIMEOUT_MS
  }

  url(path: string, query: Query = {}): string {
    const params = new URLSearchParams()
    for (const [key, value] of Object.entries(query)) {
      if (value === undefined || value === null || value === '') continue
      params.set(key, String(value))
    }
    params.set('access_token', this.token)
    return `${this.baseUrl}${path}?${params.toString()}`
  }

  redact(text: string): string {
    return redact(text, this.token)
  }

  async request<T = unknown>(spec: RequestSpec): Promise<Result<T>> {
    const url = this.url(spec.path, spec.query)
    const shown = `${spec.method} ${this.redact(decodeURIComponentSafe(url))}`
    this.log(shown)

    const headers: Record<string, string> = { Accept: 'application/json' }
    let body: string | FormData | undefined
    if (spec.json !== undefined) {
      headers['Content-Type'] = 'application/json'
      body = JSON.stringify(spec.json)
    } else if (spec.form) {
      body = spec.form
    }

    let response: Response
    try {
      response = await this.fetchImpl(url, {
        method: spec.method,
        headers,
        body,
        redirect: 'manual',
        signal: AbortSignal.timeout(this.timeoutMs),
      })
    } catch (error) {
      const reason = error instanceof Error ? `${error.name}: ${error.message}` : String(error)
      return err(0, `No response from Targetprocess (${this.redact(reason)})`, this.redact(reason), shown)
    }

    let text: string
    try {
      text = await response.text()
    } catch (error) {
      const reason = error instanceof Error ? error.message : String(error)
      return err(response.status, `Could not read the response body (${reason})`, '', shown)
    }
    text = this.redact(text)

    if (response.status >= 300 && response.status < 400) {
      const location = response.headers.get('location') ?? ''
      return err(
        response.status,
        `Targetprocess redirected to ${this.redact(location) || 'another page'}; the endpoint or method is not supported`,
        text,
        shown,
      )
    }

    if (!response.ok) {
      return err(response.status, errorMessage(response.status, text), text, shown)
    }

    if (spec.expect === 'text') return ok(text as T, response.status)

    if (text.trim() === '') return ok(null as T, response.status)
    try {
      return ok(JSON.parse(text) as T, response.status)
    } catch {
      return err(
        response.status,
        `Expected JSON from Targetprocess but got ${describeBody(text)}`,
        text.slice(0, 2000),
        shown,
      )
    }
  }
}

function decodeURIComponentSafe(url: string): string {
  try {
    return decodeURIComponent(url)
  } catch {
    return url
  }
}

function describeBody(text: string): string {
  const head = text.trimStart().slice(0, 15).toLowerCase()
  if (head.startsWith('<!doctype html') || head.startsWith('<html')) return 'an HTML page'
  if (head.startsWith('<')) return 'XML'
  return 'a non-JSON body'
}

/** Extracts Targetprocess's own message from a JSON or XML error body. */
export function errorMessage(status: number, text: string): string {
  const trimmed = text.trim()
  if (trimmed.startsWith('{')) {
    try {
      const parsed = JSON.parse(trimmed) as Record<string, unknown>
      for (const key of ['Message', 'message', 'error', 'Error']) {
        const value = parsed[key]
        if (typeof value === 'string' && value) return `${status}: ${value}`
      }
    } catch {
      // fall through
    }
  }
  const xml = /<Message>([\s\S]*?)<\/Message>/.exec(trimmed)
  if (xml?.[1]) return `${status}: ${xml[1].trim()}`
  if (trimmed.startsWith('<')) return `${status}: Targetprocess returned ${describeBody(trimmed)}`
  const firstLine = trimmed.split('\n', 1)[0]?.slice(0, 200)
  return firstLine ? `${status}: ${firstLine}` : `${status}: request failed with no body`
}
