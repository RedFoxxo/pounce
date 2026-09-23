import type { CatalogResource } from '../catalog/types.js'
import type { Err } from '../http/result.js'
import type { ToolContext } from '../tools/context.js'

/**
 * Where time is logged. Standard Targetprocess uses `Time` entries, available
 * only in processes with the "Time Tracking" practice. Some instances turn that
 * off and track time in a custom `TimeRecord` type (live: an "Hours" number and
 * a "Date" custom field, linked to the user and the card; an automation names
 * the record and fills in day/week/month periods). pounce uses it only when it
 * is there and has an hours field.
 */
export type TimeBackend =
  | { kind: 'Time' }
  | {
      kind: 'TimeRecord'
      resource: CatalogResource
      /** Custom field holding the hours, e.g. "Hours". */
      hours: string
      /** Custom field holding the day, e.g. "Date", when the type has one. */
      date?: string
      /** Card references a record can carry, most specific first, e.g. Task, Bug, UserStory. */
      cardRefs: string[]
    }

export const TIME_TRACKING_PRACTICE = 'Time Tracking'

/** Card types a record can link to, most specific first (a task's record also carries its story). */
const CARD_TYPES = ['Task', 'Bug', 'UserStory', 'Request', 'Feature', 'Epic', 'PortfolioEpic']

/** The TimeRecord backend if this instance has one, whatever the process. */
export async function timeRecordBackend(ctx: ToolContext): Promise<{ ok: true; value: Extract<TimeBackend, { kind: 'TimeRecord' }> | undefined } | { ok: false; message: string; error: Err }> {
  const resource = (await ctx.catalog()).find('TimeRecord')
  if (!resource?.available || !resource.canCreate) return { ok: true, value: undefined }
  const fields = await ctx.directory.entityCustomFields(resource.name)
  if (!fields.ok) return { ok: false, message: `Could not read the ${resource.name} custom fields`, error: fields }
  const hours = fields.data.find((f) => /^hours?$/i.test(f.Name) && /^(number|money|percent)$/i.test(f.FieldType ?? ''))
  if (!hours) return { ok: true, value: undefined }
  const date = fields.data.find((f) => /^date$/i.test(f.Name) && /^date$/i.test(f.FieldType ?? ''))
  const cardRefs = resource.references
    .filter((r) => r.canSet && CARD_TYPES.includes(r.type))
    .sort((a, b) => CARD_TYPES.indexOf(a.type) - CARD_TYPES.indexOf(b.type))
    .map((r) => r.name)
  return { ok: true, value: { kind: 'TimeRecord', resource, hours: hours.Name, ...(date ? { date: date.Name } : {}), cardRefs } }
}

/**
 * The backend for a card's process: `Time` where the process tracks time,
 * otherwise `TimeRecord` when the instance has it.
 */
export async function timeBackend(
  ctx: ToolContext,
  processId: number | undefined,
): Promise<{ ok: true; value: TimeBackend } | { ok: false; message: string; error?: Err }> {
  if (processId !== undefined) {
    const practices = await ctx.directory.practices(processId)
    if (!practices.ok) return { ok: false, message: `Could not read the practices of process ${processId}`, error: practices }
    if (practices.data.includes(TIME_TRACKING_PRACTICE)) return { ok: true, value: { kind: 'Time' } }
  }
  const record = await timeRecordBackend(ctx)
  if (!record.ok) return record
  if (record.value) return { ok: true, value: record.value }
  if (processId === undefined) return { ok: true, value: { kind: 'Time' } }
  return {
    ok: false,
    message: `Time tracking is off in this card's process (no "${TIME_TRACKING_PRACTICE}" practice) and the instance has no TimeRecord type with an Hours field`,
  }
}
