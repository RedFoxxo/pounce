import { items, num, ref } from '../format/shape.js'
import type { ToolContext } from '../tools/context.js'
import { cardInfo } from '../resolve/card.js'
import { parentOf } from './cards.js'

type Raw = Record<string, unknown>

export interface ParentSnapshot {
  id: number
  type: string
  name?: string
  state?: { id: number; name?: string }
  effort?: number
  roleEfforts: { role: string; roleId: number; effort: number }[]
}

export interface ParentRef {
  id: number
  resourceType?: string
}

/** A parent read: its snapshot, or why it could not be read (never mistaken for "no parent"). */
export type ParentRead = { ok: true; snapshot: ParentSnapshot } | { ok: false; id: number; type?: string; message: string }

/** State and role efforts of a card's nearest parent; `undefined` only when the card has no parent. */
export async function parentSnapshot(ctx: ToolContext, raw: Raw): Promise<ParentRead | undefined> {
  const parent = parentOf(raw)
  return parent ? snapshotOf(ctx, parent) : undefined
}

/** State and role efforts of one specific card (the parent being watched). */
export async function snapshotOf(ctx: ToolContext, parent: ParentRef): Promise<ParentRead> {
  const catalog = await ctx.catalog()
  let resource = parent.resourceType ? catalog.find(parent.resourceType) : undefined
  if (!resource || resource.name === 'General' || resource.name === 'Assignable') {
    const info = await cardInfo(ctx, parent.id)
    if (!info.ok) return { ok: false, id: parent.id, message: info.message }
    resource = info.value.resource
  }
  const fields = ['Id', 'Name', 'EntityState[Id,Name]']
  if (catalog.member(resource, 'Effort')) fields.push('Effort')
  if (catalog.member(resource, 'RoleEfforts')) fields.push('RoleEfforts[Id,Role[Id,Name],Effort]')
  const r = await ctx.v1.get<Raw>(resource.path, parent.id, { include: `[${fields.join(',')}]`, innerTake: 1000 })
  if (!r.ok) return { ok: false, id: parent.id, type: resource.name, message: r.message }
  const state = ref(r.data.EntityState)
  const snapshot: ParentSnapshot = {
    id: parent.id,
    type: resource.name,
    roleEfforts: items(r.data.RoleEfforts).flatMap((e) => {
      const role = ref(e.Role)
      const effort = num(e.Effort)
      return role && effort !== undefined ? [{ role: role.name ?? String(role.id), roleId: role.id, effort }] : []
    }),
  }
  if (typeof r.data.Name === 'string') snapshot.name = r.data.Name
  if (state) snapshot.state = state
  const effort = num(r.data.Effort)
  if (effort !== undefined) snapshot.effort = effort
  return { ok: true, snapshot }
}

/** Re-reads the parent captured in `before`, so before/after always describe the same card. */
export function snapshotAgain(ctx: ToolContext, before: ParentRead | undefined): Promise<ParentRead | undefined> {
  if (!before) return Promise.resolve(undefined)
  const type = before.ok ? before.snapshot.type : before.type
  return snapshotOf(ctx, { id: before.ok ? before.snapshot.id : before.id, ...(type ? { resourceType: type } : {}) })
}

export interface ParentChange {
  id: number
  type?: string
  name?: string
  state?: { before?: string; after?: string; changed: boolean }
  effort?: { before?: number; after?: number }
  roleEfforts?: { role: string; before?: number; after?: number }[]
  /** Set when the parent could not be read before or after: no change is claimed either way. */
  unknown?: string
}

/** What changed on the parent between two reads; always reports the state, even when unchanged. */
export function parentChange(before: ParentRead | undefined, after: ParentRead | undefined): ParentChange | undefined {
  if (!before && !after) return undefined
  if (!before?.ok || !after?.ok) {
    const failed = [before, after].find((r): r is Extract<ParentRead, { ok: false }> => Boolean(r && !r.ok))
    const known = before?.ok ? before.snapshot : after?.ok ? after.snapshot : undefined
    return {
      id: known?.id ?? failed?.id ?? 0,
      ...(known?.type ?? failed?.type ? { type: known?.type ?? failed?.type } : {}),
      ...(known?.name ? { name: known.name } : {}),
      unknown: `the parent could not be read ${!before?.ok ? 'before' : 'after'} the write (${failed?.message ?? 'no response'}); side effects are unknown`,
    }
  }
  const b = before.snapshot
  const a = after.snapshot
  const change: ParentChange = { id: a.id, type: a.type }
  if (a.name) change.name = a.name
  change.state = { changed: b.state?.id !== a.state?.id, ...(b.state?.name ? { before: b.state.name } : {}), ...(a.state?.name ? { after: a.state.name } : {}) }
  if (b.effort !== a.effort) {
    change.effort = { ...(b.effort !== undefined ? { before: b.effort } : {}), ...(a.effort !== undefined ? { after: a.effort } : {}) }
  }
  const roles = new Set([...b.roleEfforts, ...a.roleEfforts].map((e) => e.role))
  const efforts = [...roles].flatMap((role) => {
    const eb = b.roleEfforts.find((e) => e.role === role)?.effort
    const ea = a.roleEfforts.find((e) => e.role === role)?.effort
    return ea === eb ? [] : [{ role, ...(eb !== undefined ? { before: eb } : {}), ...(ea !== undefined ? { after: ea } : {}) }]
  })
  if (efforts.length) change.roleEfforts = efforts
  return change
}
