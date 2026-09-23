#!/usr/bin/env node
import { StdioServerTransport } from '@modelcontextprotocol/sdk/server/stdio.js'
import { ConfigError, loadConfig } from './config.js'
import { createServer } from './server.js'

async function main(): Promise<void> {
  let config
  try {
    config = loadConfig()
  } catch (error) {
    if (error instanceof ConfigError) {
      process.stderr.write(`${error.message}\n`)
      process.exit(1)
    }
    throw error
  }

  const server = createServer({ config })
  await server.connect(new StdioServerTransport())
  process.stderr.write('pounce: ready on stdio\n')
}

main().catch((error: unknown) => {
  process.stderr.write(`pounce: fatal: ${error instanceof Error ? error.message : String(error)}\n`)
  process.exit(1)
})
