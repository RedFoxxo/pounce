import { z } from 'zod'

const ID = /^\d+$/

/**
 * Every id is validated as `^\d+$`; numbers and numeric strings are accepted.
 * Numeric strings are converted before validation, so the published JSON
 * schema stays `{"type":"number"}`: it is repeated in most tools.
 */
export const id = z.preprocess(
  (v) => (typeof v === 'string' && ID.test(v) ? Number(v) : v),
  z.number({ error: 'must be a numeric id (^\\d+$)' }).refine((v) => Number.isSafeInteger(v) && v > 0, 'must be a positive integer id'),
)

export const ids = (max: number) => z.array(id).min(1).max(max)

/** A name, or a positive integer id. */
export const nameOrId = z.union([
  z.string().min(1),
  z.number().refine((v) => Number.isSafeInteger(v) && v > 0, 'must be a positive integer id'),
])

export const resource = z
  .string()
  .min(1)
  .describe('Resource name (UserStory) or plural path (UserStories); see read_meta')

/** `Id,Name,Project[Name]` or `[Id,Name,Project[Name]]`. */
export const include = z.string().min(1).optional()

export const limit = (defaultValue: number, max: number) =>
  z.number().int().min(1).max(max).optional().describe(`Default ${defaultValue}; a capped result says truncated: true`)

/** Wraps a field list in the brackets Targetprocess expects. */
export function bracket(list: string | undefined): string | undefined {
  if (!list) return undefined
  const trimmed = list.trim()
  if (!trimmed) return undefined
  return trimmed.startsWith('[') ? trimmed : `[${trimmed}]`
}

export const fields = z
  .record(z.string(), z.unknown())
  .describe('Entity fields as Targetprocess JSON, e.g. {"Name":"x","Project":{"Id":2}}')
