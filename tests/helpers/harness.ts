import { Client } from '@modelcontextprotocol/sdk/client/index.js'
import { InMemoryTransport } from '@modelcontextprotocol/sdk/inMemory.js'
import { Catalog, readSnapshot } from '../../src/catalog/catalog.js'
import type { Config } from '../../src/config.js'
import { createServer } from '../../src/server.js'
import { createContext, type ToolContext } from '../../src/tools/context.js'
import { FetchStub } from './fetch-stub.js'
import { recorder } from './record.js'

export const BASE = 'https://tp.example'
export const TOKEN = 'test-token-0000'

let snapshotCatalog: Catalog | undefined
export function snapshot(): Catalog {
  snapshotCatalog ??= new Catalog(readSnapshot(), 'snapshot')
  return snapshotCatalog
}

export interface CallResult {
  isError: boolean
  text: string
  /** Parsed JSON body of a successful call. */
  json: any
}

export interface Harness {
  stub: FetchStub
  ctx: ToolContext
  call: (name: string, args?: Record<string, unknown>) => Promise<CallResult>
  listTools: () => Promise<{ name: string; description?: string; annotations?: Record<string, unknown> }[]>
  close: () => Promise<void>
}

/**
 * The real server, real clients and real handlers, connected to an MCP client
 * over an in-memory transport. Only `fetch` is stubbed. The catalog is the
 * snapshot, unless `liveCatalog` asks for the real one read through `fetch`.
 */
export async function harness(options: { stub?: FetchStub; config?: Partial<Config>; liveCatalog?: boolean } = {}): Promise<Harness> {
  const stub = options.stub ?? new FetchStub()
  const config: Config = { baseUrl: BASE, token: TOKEN, ...options.config }
  const catalog = snapshot()
  const ctx = createContext(config, { fetch: stub.fetch, settleDelaysMs: [0], ...(options.liveCatalog ? {} : { catalog: async () => catalog }) })
  const { baseUrl: _b, token: _t, ...settings } = config
  recorder.push({ type: 'harness', config: settings, liveCatalog: Boolean(options.liveCatalog) })
  const server = createServer(ctx)
  const [clientTransport, serverTransport] = InMemoryTransport.createLinkedPair()
  await server.connect(serverTransport)
  const client = new Client({ name: 'test', version: '0' })
  await client.connect(clientTransport)

  return {
    stub,
    ctx,
    async call(name, args = {}) {
      recorder.open()
      recorder.push({ type: 'call', tool: name, args })
      const result = await client.callTool({ name, arguments: args }).finally(() => recorder.close())
      const content = result.content as { type: string; text: string }[]
      const text = content.map((c) => c.text).join('\n')
      let json: unknown
      if (!result.isError) {
        try {
          json = JSON.parse(text)
        } catch {
          json = undefined
        }
      }
      recorder.push({ type: 'result', tool: name, isError: Boolean(result.isError), text })
      return { isError: Boolean(result.isError), text, json }
    },
    async listTools() {
      const tools = (await client.listTools()).tools
      recorder.push({ type: 'listTools', tools })
      return tools as { name: string; description?: string; annotations?: Record<string, unknown> }[]
    },
    async close() {
      await client.close()
    },
  }
}
