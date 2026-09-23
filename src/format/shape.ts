import { htmlToText } from './html.js'

type Raw = Record<string, unknown> | null | undefined

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

/** `{Id, Name, ...}` → `{id, name}`; undefined when absent. */
export function ref(value: unknown): { id: number; name?: string } | undefined {
  if (!isRecord(value) || typeof value.Id !== 'number') return undefined
  return typeof value.Name === 'string' ? { id: value.Id, name: value.Name } : { id: value.Id }
}

/** A user/general-user reference with a display name and login. */
export function person(value: unknown): { id: number; name: string; login?: string } | undefined {
  if (!isRecord(value) || typeof value.Id !== 'number') return undefined
  const name =
    (typeof value.FullName === 'string' && value.FullName) ||
    [value.FirstName, value.LastName].filter((p) => typeof p === 'string' && p).join(' ') ||
    (typeof value.Login === 'string' ? value.Login : `User ${value.Id}`)
  return typeof value.Login === 'string' && value.Login ? { id: value.Id, name, login: value.Login } : { id: value.Id, name }
}

/** Items of a v1 inner collection value (`{Items: [...]}`) or a plain array. */
export function items(value: unknown): Record<string, unknown>[] {
  if (Array.isArray(value)) return value.filter(isRecord)
  if (isRecord(value) && Array.isArray(value.Items)) return value.Items.filter(isRecord)
  return []
}

export function text(value: unknown): string | undefined {
  return typeof value === 'string' ? htmlToText(value) || undefined : undefined
}

export function num(value: unknown): number | undefined {
  return typeof value === 'number' ? value : undefined
}

/** Drops keys whose value is undefined or null. */
export function compact<T extends Record<string, unknown>>(value: T): Partial<T> {
  return Object.fromEntries(Object.entries(value).filter(([, v]) => v !== undefined && v !== null)) as Partial<T>
}

export function tags(value: unknown): string[] | undefined {
  if (typeof value !== 'string' || !value.trim()) return undefined
  return value
    .split(',')
    .map((t) => t.trim())
    .filter(Boolean)
}

/** `[{Name, Type, Value}]` → `{Name: Value}`. */
export function customFields(value: unknown): Record<string, unknown> | undefined {
  if (!Array.isArray(value)) return undefined
  const out: Record<string, unknown> = {}
  for (const f of value) {
    if (isRecord(f) && typeof f.Name === 'string') out[f.Name] = f.Type === 'RichText' ? text(f.Value) ?? null : f.Value ?? null
  }
  return out
}

export function assignment(raw: Raw) {
  if (!raw) return undefined
  return compact({ id: num(raw.Id), user: person(raw.GeneralUser), role: ref(raw.Role) })
}

export function roleEffort(raw: Raw) {
  if (!raw) return undefined
  return compact({
    id: num(raw.Id),
    role: ref(raw.Role),
    effort: num(raw.Effort),
    completed: num(raw.EffortCompleted),
    toDo: num(raw.EffortToDo),
  })
}

export function teamAssignment(raw: Raw) {
  if (!raw) return undefined
  return compact({ id: num(raw.Id), team: ref(raw.Team), state: ref(raw.EntityState) })
}

/** Compact one-line listing of a card from a query. */
export function cardSummary(raw: Record<string, unknown>) {
  return compact({
    id: num(raw.Id),
    type: isRecord(raw.EntityType) ? raw.EntityType.Name : raw.ResourceType,
    name: raw.Name,
    state: isRecord(raw.EntityState) ? raw.EntityState.Name : undefined,
    project: isRecord(raw.Project) ? raw.Project.Name : undefined,
    tags: tags(raw.Tags),
    modified: raw.ModifyDate,
  })
}
