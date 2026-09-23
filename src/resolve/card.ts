import type { CatalogResource } from '../catalog/types.js'
import type { V1Ref } from '../http/v1.js'
import type { ToolContext } from '../tools/context.js'
import type { Resolved } from './match.js'

/** What pounce needs to know about any card before acting on it. */
export interface CardInfo {
  id: number
  name: string
  /** Entity type name, e.g. `UserStory`. */
  entityType: string
  resource: CatalogResource
  project?: { id: number; name: string }
  processId?: number
}

interface GeneralPayload {
  Id: number
  Name?: string
  EntityType?: V1Ref
  Project?: (V1Ref & { Process?: V1Ref | null }) | null
}

/** Resolves any card id to its type, resource path, project and process via `Generals/{id}`. */
export async function cardInfo(ctx: ToolContext, id: number): Promise<Resolved<CardInfo>> {
  const r = await ctx.v1.get<GeneralPayload>('Generals', id, { include: '[Id,Name,EntityType,Project[Id,Name,Process]]' })
  if (!r.ok) {
    return {
      ok: false,
      reason: r.status === 404 ? 'none' : 'error',
      message: r.status === 404 ? `No card with id ${id} (or no access to it).` : `Could not look up card ${id}`,
      error: r,
    }
  }
  const typeName = r.data.EntityType?.Name
  if (!typeName) return { ok: false, reason: 'error', message: `Card ${id} has no entity type in the response.` }
  const resource = (await ctx.catalog()).find(typeName)
  if (!resource?.available) {
    return { ok: false, reason: 'error', message: `Card ${id} is a ${typeName}, which the catalog does not describe.` }
  }
  const info: CardInfo = { id: r.data.Id, name: r.data.Name ?? '', entityType: resource.name, resource }
  if (r.data.Project) info.project = { id: r.data.Project.Id, name: r.data.Project.Name ?? '' }
  if (r.data.Project?.Process) info.processId = r.data.Project.Process.Id
  return { ok: true, value: info }
}
