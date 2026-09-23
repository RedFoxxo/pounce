import { Client } from '@modelcontextprotocol/sdk/client/index.js'
import { InMemoryTransport } from '@modelcontextprotocol/sdk/inMemory.js'
import { loadConfig } from '../../src/config.js'
import { createServer } from '../../src/server.js'
import { createContext } from '../../src/tools/context.js'

export const LIVE = process.env.TP_LIVE === '1'

/**
 * Card ids the user has named as safe to write to, e.g. TP_LIVE_STORY=36400.
 * Write smoke tests are skipped unless the relevant variable is set.
 */
export const liveStory = process.env.TP_LIVE_STORY ? Number(process.env.TP_LIVE_STORY) : undefined

export async function liveClient() {
  const ctx = createContext(loadConfig())
  const server = createServer(ctx)
  const [a, b] = InMemoryTransport.createLinkedPair()
  await server.connect(b)
  const client = new Client({ name: 'live', version: '0' })
  await client.connect(a)
  return {
    ctx,
    async call(name: string, args: Record<string, unknown> = {}) {
      const r = await client.callTool({ name, arguments: args })
      const text = (r.content as { text: string }[]).map((c) => c.text).join('\n')
      return { isError: Boolean(r.isError), text, json: r.isError ? undefined : JSON.parse(text) }
    },
    close: () => client.close(),
  }
}
