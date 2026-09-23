/**
 * Outcome of every request. HTTP failures are values, never exceptions, and
 * never a bare `Error`: narrow on `ok`.
 */
export type Result<T> = Ok<T> | Err

export interface Ok<T> {
  ok: true
  status: number
  data: T
}

export interface Err {
  ok: false
  /** HTTP status, or 0 when no response was received (network error, timeout). */
  status: number
  /** One-line summary: Targetprocess's own `Message` when it sent one. */
  message: string
  /** Raw response body (redacted), or the reason no response was received. */
  body: string
  /** `METHOD redacted-url`, for diagnostics. */
  request?: string
}

export function ok<T>(data: T, status = 200): Ok<T> {
  return { ok: true, status, data }
}

export function err(status: number, message: string, body = '', request?: string): Err {
  const e: Err = { ok: false, status, message, body }
  if (request !== undefined) e.request = request
  return e
}
