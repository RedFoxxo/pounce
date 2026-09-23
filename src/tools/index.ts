import type { ToolDef } from './types.js'

export type AnyToolDef = ToolDef<any>

/** Every tool pounce registers, in registration order. */
export function allTools(): AnyToolDef[] {
  return []
}
