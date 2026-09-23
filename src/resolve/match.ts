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

/** Case-, accent- and whitespace-insensitive form used for every name comparison. */
export const norm = (s: string) => s.normalize('NFKD').replace(/[\u0300-\u036f]/g, '').trim().toLowerCase().replace(/\s+/g, ' ')

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

  // Exact matches, by key priority: a hit on the first key (the name) beats a hit on a later one (an abbreviation).
  const exact = exactByPriority(q, spec, spec.items)
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

function exactByPriority<T>(q: string, spec: MatchSpec<T>, items: T[]): T[] {
  const width = Math.max(0, ...items.map((item) => spec.keys(item).length))
  for (let k = 0; k < width; k++) {
    const hits = items.filter((item) => {
      const key = spec.keys(item)[k]
      return key ? norm(key) === q : false
    })
    if (hits.length) return hits
  }
  return []
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

/**
 * Like `match`, but for directories with inactive entries. An exact name (or
 * id) is looked up across everyone first, so an exact hit on an inactive entry
 * is reported as inactive instead of silently resolving to an active entry
 * that merely contains the name. Partial matches consider active entries only.
 */
export function matchActive<T>(input: string | number, spec: MatchSpec<T>, isActive: (item: T) => boolean): Resolved<T> {
  const raw = String(input).trim()
  const q = norm(raw)
  const exact = /^\d+$/.test(raw) ? spec.items.filter((item) => spec.id(item) === Number(raw)) : exactByPriority(q, spec, spec.items)
  if (exact.length > 0) {
    const active = exact.filter(isActive)
    if (active.length === 1) return { ok: true, value: active[0] as T }
    if (active.length > 1) return ambiguous(spec, raw, active)
    const first = exact[0] as T
    return { ok: false, reason: 'none', message: `${spec.kind.replace(/^./, (c) => c.toUpperCase())} ${spec.name(first)} (${spec.id(first)}) is inactive or deleted.` }
  }
  const found = match(raw, { ...spec, items: spec.items.filter(isActive) })
  if (found.ok || found.reason !== 'none') return found
  const inactive = match(raw, { ...spec, items: spec.items.filter((i) => !isActive(i)) })
  if (inactive.ok) {
    return { ok: false, reason: 'none', message: `${spec.kind.replace(/^./, (c) => c.toUpperCase())} ${spec.name(inactive.value)} (${spec.id(inactive.value)}) is inactive or deleted.` }
  }
  return found
}
