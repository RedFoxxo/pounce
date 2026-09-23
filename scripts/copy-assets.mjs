// Copies non-TypeScript runtime assets from src/ into build/ after `tsc`.
import { cpSync, existsSync } from 'node:fs'

const assets = ['catalog/snapshot.json']

for (const asset of assets) {
  const from = `src/${asset}`
  if (!existsSync(from)) {
    console.error(`copy-assets: ${from} is missing; the server needs it at runtime`)
    process.exit(1)
  }
  cpSync(from, `build/${asset}`)
}
