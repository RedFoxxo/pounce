import { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js'
import { ListToolsRequestSchema, type Tool, type ToolAnnotations } from '@modelcontextprotocol/sdk/types.js'
import { z } from 'zod'
import { isRecord } from './format/shape.js'
import type { ToolContext } from './tools/context.js'
import { toolsFor, type AnyToolDef } from './tools/index.js'
import { TOOLSETS, tierOf } from './tools/types.js'
import { VERSION } from './version.js'

/**
 * Only reads are marked. Everything else keeps the MCP defaults (not read-only,
 * destructive, open world): writes overwrite fields and append to collections.
 */
const READ_ONLY: ToolAnnotations = { readOnlyHint: true }

export function createServer(ctx: ToolContext, tools: AnyToolDef[] = toolsFor(ctx.config.toolsets ?? TOOLSETS)): McpServer {
  const server = new McpServer({ name: 'pounce', version: VERSION })
  const seen = new Set<string>()
  const listed: Tool[] = []

  for (const tool of tools) {
    const tier = tierOf(tool.name)
    if (!tier) throw new Error(`Tool "${tool.name}" has no tier prefix (read_/write_/delete_/admin_)`)
    if (seen.has(tool.name)) throw new Error(`Tool "${tool.name}" is registered twice`)
    seen.add(tool.name)

    const annotations = tier === 'read' ? { annotations: READ_ONLY } : {}
    server.registerTool(
      tool.name,
      { description: tool.description, inputSchema: tool.input, ...annotations },
      async (args: Record<string, unknown>) => tool.handler(args, ctx),
    )
    // The input JSON Schema as the SDK derives it, without keywords that only restate defaults.
    const inputSchema = prune(z.toJSONSchema(z.object(tool.input), { target: 'draft-7', io: 'input' })) as Tool['inputSchema']
    listed.push({ name: tool.name, description: tool.description, inputSchema, ...annotations })
  }

  // The SDK's own tools/list adds `$schema`, `execution` and JSON Schema defaults
  // to every tool, and the client sends the list to the model on every request.
  // Calls are still validated by the SDK against the zod schemas registered above.
  if (listed.length > 0) server.server.setRequestHandler(ListToolsRequestSchema, () => ({ tools: listed }))
  return server
}

/**
 * Drops `$schema`, `propertyNames: {"type":"string"}` (JSON keys always are
 * strings) and `additionalProperties: {}` (anything is the default).
 * Keys under `properties` are parameter names, never keywords.
 */
function prune(schema: Record<string, unknown>): Record<string, unknown> {
  const sub = (value: unknown): unknown => (Array.isArray(value) ? value.map(sub) : isRecord(value) ? prune(value) : value)
  return Object.fromEntries(
    Object.entries(schema).flatMap(([key, value]): [string, unknown][] => {
      if (key === '$schema') return []
      if (key === 'propertyNames' && isRecord(value) && value.type === 'string' && Object.keys(value).length === 1) return []
      if (key === 'additionalProperties' && isRecord(value) && Object.keys(value).length === 0) return []
      if (key === 'properties' && isRecord(value)) return [[key, Object.fromEntries(Object.entries(value).map(([name, s]) => [name, sub(s)]))]]
      return [[key, sub(value)]]
    }),
  )
}
