// Copies non-TypeScript runtime assets from src/ into build/ after `tsc`.
import { cpSync, existsSync } from 'node:fs'

const assets = ['catalog/snapshot.json']

for (const asset of assets) {
  const from = `src/${asset}`
  if (existsSync(from)) cpSync(from, `build/${asset}`)
}
