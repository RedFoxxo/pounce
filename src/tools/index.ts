import { extraAdminTools, extraDeleteTools, extraReadTools, extraWriteTools } from './generic/extras.js'
import { genericReadTools } from './generic/read.js'
import { adminTools, genericDeleteTools, genericWriteTools } from './generic/write.js'
import type { ToolDef } from './types.js'
import { workflowReadTools } from './workflow/read.js'
import { workflowWriteTools } from './workflow/write.js'
import { workflowDeleteTools } from './workflow/delete.js'

export type AnyToolDef = ToolDef<any>

/** Every tool pounce registers, in registration order. */
export function allTools(): AnyToolDef[] {
  return [
    ...workflowReadTools,
    ...genericReadTools,
    ...extraReadTools,
    ...workflowWriteTools,
    ...genericWriteTools,
    ...extraWriteTools,
    ...workflowDeleteTools,
    ...genericDeleteTools,
    ...extraDeleteTools,
    ...adminTools,
    ...extraAdminTools,
  ]
}
