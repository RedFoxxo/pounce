import { z } from 'zod'

const ID = /^\d+$/

/** Every id is validated as `^\d+$`; numbers and numeric strings are accepted. */
export const id = z
  .union([z.number().int().nonnegative(), z.string().regex(ID, 'must be a numeric id (^\\d+$)')])
  .transform((v) => Number(v))
  .refine((v) => Number.isSafeInteger(v) && v > 0, 'must be a positive integer id')

export const ids = (max: number) => z.array(id).min(1).max(max)

export const resource = z
  .string()
  .min(1)
  .describe('Resource name (UserStory) or plural path (UserStories); see read_meta')

/** `Id,Name,Project[Name]` or `[Id,Name,Project[Name]]`. */
export const include = z.string().min(1).optional()

export const limit = (defaultValue: number, max: number) =>
  z
    .number()
    .int()
    .min(1)
    .max(max)
    .optional()
    .describe(`Maximum items to return (default ${defaultValue}, max ${max}). A capped result says truncated: true.`)

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
