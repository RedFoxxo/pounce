import { readFileSync } from 'node:fs'
import { describe, expect, it } from 'vitest'
import { Client } from '@modelcontextprotocol/sdk/client/index.js'
import { InMemoryTransport } from '@modelcontextprotocol/sdk/inMemory.js'
import { createServer } from '../src/server.js'
import { VERSION } from '../src/version.js'

describe('server', () => {
  it('reports the package version', () => {
    const pkg = JSON.parse(readFileSync(new URL('../package.json', import.meta.url), 'utf8')) as { version: string }
    expect(VERSION).toBe(pkg.version)
  })

  it('starts and completes the MCP handshake', async () => {
    const server = createServer({ config: { baseUrl: 'https://example.tpondemand.com', token: 't' } }, [])
    const [clientTransport, serverTransport] = InMemoryTransport.createLinkedPair()
    await server.connect(serverTransport)
    const client = new Client({ name: 'test', version: '0' })
    await client.connect(clientTransport)
    expect(client.getServerVersion()?.name).toBe('pounce')
    await client.close()
  })
})
