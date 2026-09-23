// Regenerates src/catalog/snapshot.json from the configured instance: `npm run snapshot`.
import { writeFileSync } from 'node:fs'
import { loadConfig } from '../config.js'
import { HttpCore } from '../http/core.js'
import { V1Client } from '../http/v1.js'
import { stderrLogger } from '../log.js'
import { loadCatalog } from './loader.js'

const config = loadConfig()
const v1 = new V1Client(new HttpCore({ baseUrl: config.baseUrl, token: config.token }))
const result = await loadCatalog(v1, { log: stderrLogger })
if (!result.ok) {
  process.stderr.write(`snapshot: failed: ${result.message}\n`)
  process.exit(1)
}

const resources = result.data.resources
const listed = resources.filter((r) => r.listed)
const collections = resources.flatMap((r) => r.collections)
// The snapshot ships in the package: never record which instance it came from.
writeFileSync('src/catalog/snapshot.json', `${JSON.stringify({ ...result.data, instance: 'baseline snapshot' }, null, 1)}\n`)
process.stdout.write(
  `snapshot: ${listed.length} listed (${listed.filter((r) => !r.available).length} unavailable), ` +
    `${resources.length - listed.length} unlisted, ${collections.length} collections, ` +
    `${collections.filter((c) => c.canAdd || c.canRemove).length} addable/removable\n`,
)
