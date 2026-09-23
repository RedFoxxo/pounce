import type { Err } from '../http/result.js'
import type { ToolOutput } from './types.js'

/** Compact JSON success response. */
export function success(data: unknown): ToolOutput {
  return { content: [{ type: 'text', text: JSON.stringify(data) }] }
}

/**
 * Failure response: a one-line summary, then Targetprocess's `status` and raw
 * `body` when the failure came from a request.
 */
export function failure(summary: string, error?: Err, extra?: Record<string, unknown>): ToolOutput {
  const lines = [summary.split('\n')[0] ?? summary]
  const rest = summary.split('\n').slice(1).join('\n').trim()
  if (rest) lines.push(rest)
  if (error) {
    if (error.message && !summary.includes(error.message)) lines.push(`message: ${error.message}`)
    lines.push(`status: ${error.status}`)
    lines.push(`body: ${error.body || '(empty)'}`)
    if (error.request) lines.push(`request: ${error.request}`)
  }
  if (extra && Object.keys(extra).length > 0) lines.push(`details: ${JSON.stringify(extra)}`)
  return { content: [{ type: 'text', text: lines.join('\n') }], isError: true }
}

/** A call rejected before anything was sent. */
export function invalid(message: string): ToolOutput {
  return failure(`Not sent: ${message}`)
}
