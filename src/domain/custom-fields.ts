import { sameDateValue } from '../format/dates.js'
import { customFieldOptions, type TpCustomField } from '../resolve/directory.js'
import type { Resolved } from '../resolve/match.js'
import type { ToolContext } from '../tools/context.js'

export interface CustomFieldValue {
  Name: string
  Value: unknown
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

/** Checks one value against the field's type; dropdown values must be one of its options. */
export function checkCustomValue(field: TpCustomField, value: unknown): { ok: true; value: unknown } | { ok: false; message: string } {
  if (field.IsSystem || field.Config?.CalculationModel) {
    return { ok: false, message: `${field.Name} is ${field.IsSystem ? 'a system' : 'a calculated'} field; only Targetprocess sets it` }
  }
  if (value === null) return { ok: true, value: null }
  const type = (field.FieldType ?? '').toLowerCase()
  const options = customFieldOptions(field)
  if (options) {
    const wanted = Array.isArray(value) ? value : type === 'multipleselectionlist' && typeof value === 'string' ? value.split(',') : [value]
    const canonical: string[] = []
    for (const w of wanted) {
      const s = typeof w === 'string' ? w.trim() : String(w)
      const hit = options.find((o) => o.toLowerCase() === s.toLowerCase())
      if (!hit) return { ok: false, message: `"${s}" is not an option of ${field.Name}; options: ${options.join(', ')}` }
      canonical.push(hit)
    }
    if (type === 'dropdown' && canonical.length !== 1) return { ok: false, message: `${field.Name} takes exactly one option` }
    return { ok: true, value: type === 'dropdown' ? canonical[0] : canonical.join(',') }
  }
  if (['number', 'money', 'percent'].includes(type)) {
    const n = typeof value === 'string' && value.trim() !== '' ? Number(value) : value
    if (typeof n !== 'number' || Number.isNaN(n)) return { ok: false, message: `${field.Name} is a ${field.FieldType} field; got ${JSON.stringify(value)}` }
    return { ok: true, value: n }
  }
  if (type === 'checkbox') {
    if (typeof value !== 'boolean') return { ok: false, message: `${field.Name} is a checkbox; pass true or false` }
    return { ok: true, value }
  }
  if (type === 'url') {
    if (typeof value === 'string') return { ok: true, value: { Url: value, Label: value } }
    if (isRecord(value) && typeof value.Url === 'string') return { ok: true, value: { Url: value.Url, Label: typeof value.Label === 'string' ? value.Label : value.Url } }
    return { ok: false, message: `${field.Name} is a URL field; pass a URL or {"Url","Label"}` }
  }
  if (type === 'entity' || type === 'multipleentities') {
    if (!isRecord(value) || typeof value.Id !== 'number') return { ok: false, message: `${field.Name} references an entity; pass {"Id": n, "Kind": "Release"}` }
    return { ok: true, value }
  }
  return { ok: true, value }
}

/** Resolves field names for the card type/process and validates every value before anything is sent. */
export async function prepareCustomFields(
  ctx: ToolContext,
  processId: number,
  entityType: string,
  values: Record<string, unknown>,
): Promise<Resolved<CustomFieldValue[]>> {
  const out: CustomFieldValue[] = []
  const problems: string[] = []
  for (const [name, value] of Object.entries(values)) {
    const field = await ctx.directory.customField(processId, entityType, name)
    if (!field.ok) {
      if (field.reason === 'error') return field
      problems.push(field.message)
      continue
    }
    const checked = checkCustomValue(field.value, value)
    if (!checked.ok) problems.push(checked.message)
    else out.push({ Name: field.value.Name, Value: checked.value })
  }
  if (problems.length) return { ok: false, reason: 'none', message: `Custom fields rejected:\n- ${problems.join('\n- ')}` }
  return { ok: true, value: out }
}

function sameCustomValue(want: unknown, got: unknown): boolean {
  if (want === null || want === undefined) return got === null || got === undefined || got === ''
  if (isRecord(want) && typeof want.Url === 'string') {
    const gotUrl = isRecord(got) ? got.Url : got
    return gotUrl === want.Url
  }
  if (isRecord(want) && typeof want.Id === 'number') return isRecord(got) && got.Id === want.Id
  if (typeof want === 'string' && /^\d{4}-\d{2}-\d{2}/.test(want) && typeof got === 'string') return sameDateValue(want, got)
  if (typeof want === 'string' && typeof got === 'string' && want.includes(',')) {
    const norm = (s: string) => s.split(',').map((x) => x.trim()).sort().join(',')
    return norm(want) === norm(got)
  }
  return JSON.stringify(got) === JSON.stringify(want)
}

/** Compares requested custom field values with a read-back `CustomFields` array. */
export function unpersistedCustomFields(requested: CustomFieldValue[], readBack: unknown): string[] {
  const list = Array.isArray(readBack) ? readBack.filter(isRecord) : []
  return requested.flatMap((req) => {
    const field = list.find((f) => f.Name === req.Name)
    if (!field) return [`${req.Name}: not present on the card after writing`]
    return sameCustomValue(req.Value, field.Value) ? [] : [`${req.Name}: requested ${JSON.stringify(req.Value)}, got ${JSON.stringify(field.Value ?? null)}`]
  })
}
