import type { z } from 'zod'
import type { CallToolResult } from '@modelcontextprotocol/sdk/types.js'
import type { ToolContext } from './context.js'

export type Tier = 'read' | 'write' | 'delete' | 'admin'

export const TIERS: readonly Tier[] = ['read', 'write', 'delete', 'admin']

export type ToolOutput = CallToolResult

export interface ToolDef<S extends z.ZodRawShape = z.ZodRawShape> {
  /** Registered without a `pounce_` prefix; the tier prefix is the permission contract. */
  name: string
  description: string
  input: S
  handler: (args: z.output<z.ZodObject<S>>, ctx: ToolContext) => Promise<ToolOutput>
}

/** Keeps the handler's argument type tied to its schema. */
export function defineTool<S extends z.ZodRawShape>(def: ToolDef<S>): ToolDef<S> {
  return def
}

export function tierOf(name: string): Tier | undefined {
  const prefix = name.split('_', 1)[0]
  return TIERS.find((t) => t === prefix)
}
