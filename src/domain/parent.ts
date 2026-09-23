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

/** State and role efforts of a card's nearest parent, for before/after side-effect reports. */
export async function parentSnapshot(ctx: ToolContext, raw: Raw): Promise<ParentSnapshot | undefined> {
  const parent = parentOf(raw)
  return parent ? snapshotOf(ctx, parent) : undefined
}

/** State and role efforts of one specific card (the parent being watched). */
export async function snapshotOf(ctx: ToolContext, parent: ParentRef): Promise<ParentSnapshot | undefined> {
  const catalog = await ctx.catalog()
  let resource = parent.resourceType ? catalog.find(parent.resourceType) : undefined
  if (!resource || resource.name === 'General' || resource.name === 'Assignable') {
    const info = await cardInfo(ctx, parent.id)
    if (!info.ok) return undefined
    resource = info.value.resource
  }
  const fields = ['Id', 'Name', 'EntityState[Id,Name]']
  if (catalog.member(resource, 'Effort')) fields.push('Effort')
  if (catalog.member(resource, 'RoleEfforts')) fields.push('RoleEfforts[Id,Role[Id,Name],Effort]')
  const r = await ctx.v1.get<Raw>(resource.path, parent.id, { include: `[${fields.join(',')}]`, innerTake: 1000 })
  if (!r.ok) return undefined
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
  return snapshot
}

/** Re-reads the parent captured in `before`, so before/after always describe the same card. */
export function snapshotAgain(ctx: ToolContext, before: ParentSnapshot | undefined): Promise<ParentSnapshot | undefined> {
  return before ? snapshotOf(ctx, { id: before.id, resourceType: before.type }) : Promise.resolve(undefined)
}

export interface ParentChange {
  id: number
  type: string
  name?: string
  state?: { before?: string; after?: string; changed: boolean }
  effort?: { before?: number; after?: number }
  roleEfforts?: { role: string; before?: number; after?: number }[]
}

/** What changed on the parent between two snapshots; always reports the state, even when unchanged. */
export function parentChange(before: ParentSnapshot | undefined, after: ParentSnapshot | undefined): ParentChange | undefined {
  const p = after ?? before
  if (!p) return undefined
  const change: ParentChange = { id: p.id, type: p.type }
  if (p.name) change.name = p.name
  const sb = before?.state?.name
  const sa = after?.state?.name
  change.state = { changed: before?.state?.id !== after?.state?.id, ...(sb ? { before: sb } : {}), ...(sa ? { after: sa } : {}) }
  if (before?.effort !== after?.effort) {
    change.effort = { ...(before?.effort !== undefined ? { before: before.effort } : {}), ...(after?.effort !== undefined ? { after: after.effort } : {}) }
  }
  const roles = new Set([...(before?.roleEfforts ?? []), ...(after?.roleEfforts ?? [])].map((e) => e.role))
  const efforts = [...roles].flatMap((role) => {
    const b = before?.roleEfforts.find((e) => e.role === role)?.effort
    const a = after?.roleEfforts.find((e) => e.role === role)?.effort
    return a === b ? [] : [{ role, ...(b !== undefined ? { before: b } : {}), ...(a !== undefined ? { after: a } : {}) }]
  })
  if (efforts.length) change.roleEfforts = efforts
  return change
}
