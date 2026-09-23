import type { CatalogResource } from '../../catalog/types.js'
import type { Resolved } from '../../resolve/match.js'
import { cardInfo } from '../../resolve/card.js'
import type { ToolContext } from '../context.js'
import { failure, invalid } from '../respond.js'
import type { ToolOutput } from '../types.js'

/** Turns a failed resolution into a tool error: HTTP failures carry status/body, the rest is "Not sent". */
export function unresolved(r: Extract<Resolved<unknown>, { ok: false }>): ToolOutput {
  if (r.error) return failure(r.message, r.error)
  return invalid(r.message)
}

export interface TypeScope {
  entityType: string
  resource: CatalogResource
  processId: number
  project?: { id: number; name: string }
  cardId?: number
}

/**
 * Where states/custom fields come from: a card (its own type and project), or
 * a project plus an entity type.
 */
export async function typeScope(
  ctx: ToolContext,
  args: { id?: number | undefined; project?: string | number | undefined; type?: string | undefined },
): Promise<{ ok: true; value: TypeScope } | { ok: false; output: ToolOutput }> {
  if (args.id !== undefined) {
    const info = await cardInfo(ctx, args.id)
    if (!info.ok) return { ok: false, output: unresolved(info) }
    if (info.value.processId === undefined) {
      return { ok: false, output: invalid(`${info.value.entityType} ${args.id} has no project, so it has no process workflow.`) }
    }
    return {
      ok: true,
      value: {
        entityType: info.value.entityType,
        resource: info.value.resource,
        processId: info.value.processId,
        ...(info.value.project ? { project: info.value.project } : {}),
        cardId: args.id,
      },
    }
  }
  if (args.project === undefined || !args.type) {
    return { ok: false, output: invalid('pass a card id, or both project and type (e.g. project "SBP", type "UserStory")') }
  }
  const catalog = await ctx.catalog()
  const resource = catalog.resource(args.type)
  if (!resource.ok) return { ok: false, output: invalid(resource.message) }
  const project = await ctx.directory.project(args.project)
  if (!project.ok) return { ok: false, output: unresolved(project) }
  if (!project.value.Process) return { ok: false, output: invalid(`Project ${project.value.Name} has no process.`) }
  return {
    ok: true,
    value: {
      entityType: resource.value.name,
      resource: resource.value,
      processId: project.value.Process.Id,
      project: { id: project.value.Id, name: project.value.Name },
    },
  }
}
