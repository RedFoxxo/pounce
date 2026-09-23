import { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js'
import type { ToolContext } from './tools/context.js'
import { allTools, type AnyToolDef } from './tools/index.js'
import { tierOf } from './tools/types.js'
import { VERSION } from './version.js'

export function createServer(ctx: ToolContext, tools: AnyToolDef[] = allTools()): McpServer {
  const server = new McpServer({ name: 'pounce', version: VERSION })
  const seen = new Set<string>()

  for (const tool of tools) {
    const tier = tierOf(tool.name)
    if (!tier) throw new Error(`Tool "${tool.name}" has no tier prefix (read_/write_/delete_/admin_)`)
    if (seen.has(tool.name)) throw new Error(`Tool "${tool.name}" is registered twice`)
    seen.add(tool.name)

    server.registerTool(
      tool.name,
      {
        description: tool.description,
        inputSchema: tool.input,
        annotations: {
          readOnlyHint: tier === 'read',
          // Writes overwrite fields and append to collections; only reads are harmless.
          destructiveHint: tier !== 'read',
          openWorldHint: true,
        },
      },
      async (args: Record<string, unknown>) => tool.handler(args, ctx),
    )
  }

  return server
}
