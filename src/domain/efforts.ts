import { items, num, ref } from '../format/shape.js'
import type { Err } from '../http/result.js'
import type { ToolContext } from '../tools/context.js'

type Raw = Record<string, unknown>

export interface RoleEffortRow {
  id: number
  role: { id: number; name?: string }
  effort: number
}

export function roleEffortRows(raw: Raw): RoleEffortRow[] {
  return items(raw.RoleEfforts).flatMap((e) => {
    const id = num(e.Id)
    const role = ref(e.Role)
    return id !== undefined && role ? [{ id, role, effort: num(e.Effort) ?? 0 }] : []
  })
}

export interface EffortRequest {
  roleId: number
  roleName: string
  effort: number
}

export interface EffortOutcome {
  applied: { role: string; before?: number; after: number; row: number; created: boolean }[]
  error?: { message: string; error: Err }
}

/**
 * Writes effort to the card's RoleEffort rows (never the card's Effort total,
 * which Targetprocess computes). Updates the existing row for a role, or
 * creates one when the card has none. Sequential: one card, one write at a time.
 */
export async function applyRoleEfforts(ctx: ToolContext, cardId: number, rows: RoleEffortRow[], requests: EffortRequest[]): Promise<EffortOutcome> {
  const outcome: EffortOutcome = { applied: [] }
  for (const req of requests) {
    const row = rows.find((r) => r.role.id === req.roleId)
    if (row && row.effort === req.effort) {
      outcome.applied.push({ role: req.roleName, before: row.effort, after: req.effort, row: row.id, created: false })
      continue
    }
    const r = row
      ? await ctx.v1.update<Raw>('RoleEfforts', row.id, { Effort: req.effort })
      : await ctx.v1.create<Raw>('RoleEfforts', { Assignable: { Id: cardId }, Role: { Id: req.roleId }, Effort: req.effort })
    if (!r.ok) {
      outcome.error = { message: `Could not set ${req.roleName} effort to ${req.effort}`, error: r }
      return outcome
    }
    outcome.applied.push({
      role: req.roleName,
      ...(row ? { before: row.effort } : {}),
      after: req.effort,
      row: row?.id ?? num(r.data?.Id) ?? 0,
      created: !row,
    })
  }
  return outcome
}
