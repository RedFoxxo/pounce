const V1_DATE = /^\/Date\((-?\d+)([+-]\d{4})?\)\/$/

/** `/Date(1789125687000+0200)/` → `2026-09-11T09:21:27.000Z`. Other values are returned unchanged. */
export function v1DateToIso(value: string): string {
  const match = V1_DATE.exec(value)
  if (!match?.[1]) return value
  return new Date(Number(match[1])).toISOString()
}

/** Deep-converts every v1 date string in a parsed payload to ISO 8601. */
export function normalizeV1Dates<T>(value: T): T {
  if (typeof value === 'string') return v1DateToIso(value) as T
  if (Array.isArray(value)) return value.map((item) => normalizeV1Dates(item)) as T
  if (value && typeof value === 'object') {
    const out: Record<string, unknown> = {}
    for (const [key, item] of Object.entries(value)) out[key] = normalizeV1Dates(item)
    return out as T
  }
  return value
}
