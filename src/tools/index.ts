import { extraAdminTools, extraCardTools, extraRawTools } from './generic/extras.js'
import { genericReadTools } from './generic/read.js'
import { adminTools, genericDeleteTools, genericWriteTools } from './generic/write.js'
import { TOOLSETS, type ToolDef, type Toolset } from './types.js'
import { workflowReadTools } from './workflow/read.js'
import { workflowWriteTools } from './workflow/write.js'
import { workflowDeleteTools } from './workflow/delete.js'

export type AnyToolDef = ToolDef<any>

const GROUPS: Record<Toolset, AnyToolDef[]> = {
  cards: [...workflowReadTools, ...workflowWriteTools, ...workflowDeleteTools, ...extraCardTools],
  raw: [...genericReadTools, ...genericWriteTools, ...genericDeleteTools, ...extraRawTools],
  admin: [...adminTools, ...extraAdminTools],
}

/** The tools of the given toolsets, in registration order. */
export function toolsFor(toolsets: readonly Toolset[]): AnyToolDef[] {
  return TOOLSETS.filter((t) => toolsets.includes(t)).flatMap((t) => GROUPS[t])
}

/** Every tool pounce can register. */
export function allTools(): AnyToolDef[] {
  return toolsFor(TOOLSETS)
}
