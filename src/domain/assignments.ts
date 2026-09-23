import { items, num, person, ref } from '../format/shape.js'
import type { Result } from '../http/result.js'
import type { ToolContext } from '../tools/context.js'

type Raw = Record<string, unknown>

export interface AssignmentRow {
  id: number
  user: { id: number; name: string; login?: string }
  role: { id: number; name?: string }
}

export function assignmentRows(raw: Raw): AssignmentRow[] {
  return items(raw.Assignments).flatMap((a) => {
    const id = num(a.Id)
    const user = person(a.GeneralUser)
    const role = ref(a.Role)
    return id !== undefined && user && role ? [{ id, user, role }] : []
  })
}

/** Every assignment of a card, read fresh from `/Assignments` (fully paged). */
export async function listAssignments(ctx: ToolContext, cardId: number): Promise<Result<AssignmentRow[]>> {
  const r = await ctx.v1.list<Raw>('Assignments', {
    where: `(Assignable.Id eq ${cardId})`,
    include: '[Id,Role[Id,Name],GeneralUser[Id,FirstName,LastName,Login]]',
  })
  if (!r.ok) return r
  return { ok: true, status: r.status, data: assignmentRows({ Assignments: { Items: r.data.items } }) }
}

/** Creates one assignment. Posting to `/Assignments` adds exactly this row and nothing else. */
export function addAssignment(ctx: ToolContext, cardId: number, userId: number, roleId: number): Promise<Result<Raw>> {
  return ctx.v1.create<Raw>('Assignments', { Assignable: { Id: cardId }, GeneralUser: { Id: userId }, Role: { Id: roleId } })
}

export function removeAssignment(ctx: ToolContext, assignmentId: number): Promise<Result<unknown>> {
  return ctx.v1.delete('Assignments', assignmentId)
}

export function describeAssignment(a: AssignmentRow): { id: number; user: string; role: string } {
  return { id: a.id, user: `${a.user.name} (${a.user.id})`, role: a.role.name ?? String(a.role.id) }
}
