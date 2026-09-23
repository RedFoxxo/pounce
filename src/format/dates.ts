const V1_DATE = /^\/Date\((-?\d+)([+-])(\d{2})(\d{2})\)\/$|^\/Date\((-?\d+)\)\/$/

function pad(n: number, width = 2): string {
  return String(n).padStart(width, '0')
}

/**
 * `/Date(1789682400000+0200)/` → `2026-09-18T00:00:00.000+02:00`. The offset
 * Targetprocess sends is kept, so date-only values stored as local midnight
 * (release and iteration dates, time entries) keep their calendar day. Values
 * without an offset become UTC (`Z`). Anything else is returned unchanged.
 */
export function v1DateToIso(value: string): string {
  const match = V1_DATE.exec(value)
  if (!match) return value
  if (match[5] !== undefined) return new Date(Number(match[5])).toISOString()
  const ms = Number(match[1])
  const sign = match[2] === '-' ? -1 : 1
  const offsetMinutes = sign * (Number(match[3]) * 60 + Number(match[4]))
  const local = new Date(ms + offsetMinutes * 60_000)
  if (Number.isNaN(local.getTime())) return value
  const date = `${pad(local.getUTCFullYear(), 4)}-${pad(local.getUTCMonth() + 1)}-${pad(local.getUTCDate())}`
  const time = `${pad(local.getUTCHours())}:${pad(local.getUTCMinutes())}:${pad(local.getUTCSeconds())}.${pad(local.getUTCMilliseconds(), 3)}`
  return `${date}T${time}${match[2]}${match[3]}:${match[4]}`
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

/**
 * Whether a requested date/time and a read-back value mean the same thing:
 * a date-only request (`YYYY-MM-DD`) compares calendar days, anything else
 * compares instants to the second.
 */
export function sameDateValue(want: string, got: string): boolean {
  if (/^\d{4}-\d{2}-\d{2}$/.test(want)) return got.slice(0, 10) === want
  const a = Date.parse(want)
  const b = Date.parse(got)
  return !Number.isNaN(a) && !Number.isNaN(b) && Math.abs(a - b) < 1000
}
