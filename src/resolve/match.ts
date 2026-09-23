import type { Err } from '../http/result.js'

export interface Candidate {
  id: number
  name: string
  [extra: string]: unknown
}

export type Resolved<T> =
  | { ok: true; value: T }
  | {
      ok: false
      /** One-line reason, e.g. `No role matches "Dev".` */
      message: string
      /** `none`: nothing matched; `ambiguous`: several did; `error`: the lookup failed. */
      reason?: 'none' | 'ambiguous' | 'error'
      /** Every candidate for an ambiguous name, or close suggestions for an unknown one. */
      candidates?: Candidate[]
      /** Set when the lookup itself failed at the HTTP level. */
      error?: Err
    }

export interface MatchSpec<T> {
  /** What is being resolved, e.g. `user`, `role`. */
  kind: string
  items: T[]
  id: (item: T) => number
  /** Display name. */
  name: (item: T) => string
  /** Every string the input may equal, e.g. full name, login, email. */
  keys: (item: T) => (string | null | undefined)[]
  /** Extra fields shown for candidates. */
  describe?: (item: T) => Record<string, unknown>
}

const norm = (s: string) => s.normalize('NFKD').replace(/[\u0300-\u036f]/g, '').trim().toLowerCase().replace(/\s+/g, ' ')

function candidate<T>(spec: MatchSpec<T>, item: T): Candidate {
  return { id: spec.id(item), name: spec.name(item), ...spec.describe?.(item) }
}

/**
 * Resolves a name or numeric id. Never guesses:
 * - numeric input matches the id exactly;
 * - otherwise one exact (case/accent-insensitive) match on any key wins;
 * - otherwise one partial match (every word of the input is found) wins;
 * - several matches at the same level → error listing every candidate;
 * - no match → error with close suggestions.
 */
export function match<T>(input: string | number, spec: MatchSpec<T>): Resolved<T> {
  const raw = String(input).trim()
  if (/^\d+$/.test(raw)) {
    const found = spec.items.find((item) => spec.id(item) === Number(raw))
    if (found) return { ok: true, value: found }
    return { ok: false, reason: 'none', message: `No ${spec.kind} has id ${raw}.` }
  }
  const q = norm(raw)
  if (!q) return { ok: false, reason: 'none', message: `Empty ${spec.kind} name.` }

  const exact = spec.items.filter((item) => spec.keys(item).some((k) => k && norm(k) === q))
  if (exact.length === 1) return { ok: true, value: exact[0] as T }
  if (exact.length > 1) return ambiguous(spec, raw, exact)

  const words = q.split(' ')
  const partial = spec.items.filter((item) =>
    spec.keys(item).some((k) => {
      if (!k) return false
      const key = norm(k)
      return words.every((w) => key.includes(w))
    }),
  )
  if (partial.length === 1) return { ok: true, value: partial[0] as T }
  if (partial.length > 1) return ambiguous(spec, raw, partial)

  const close = spec.items
    .map((item) => ({ item, score: Math.min(...spec.keys(item).filter((k): k is string => Boolean(k)).map((k) => distance(q, norm(k)))) }))
    .filter((s) => s.score <= Math.max(2, Math.floor(q.length / 3)))
    .sort((a, b) => a.score - b.score)
    .slice(0, 5)
    .map((s) => candidate(spec, s.item))
  return {
    ok: false,
    reason: 'none',
    message: `No ${spec.kind} matches "${raw}".` + (close.length ? ` Did you mean: ${close.map((c) => `${c.name} (${c.id})`).join(', ')}?` : ''),
    ...(close.length ? { candidates: close } : {}),
  }
}

function ambiguous<T>(spec: MatchSpec<T>, raw: string, items: T[]): Resolved<T> {
  const candidates = items.map((item) => candidate(spec, item))
  return {
    ok: false,
    reason: 'ambiguous',
    message: `"${raw}" matches ${items.length} ${spec.kind}s; pass one id: ${candidates.map((c) => `${c.name} (${c.id})`).join(', ')}.`,
    candidates,
  }
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
