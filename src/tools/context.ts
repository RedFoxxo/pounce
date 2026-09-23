import type { Config } from '../config.js'
import type { Catalog } from '../catalog/catalog.js'
import { CatalogProvider } from '../catalog/catalog.js'
import { HttpCore, type FetchLike } from '../http/core.js'
import { V1Client } from '../http/v1.js'
import { V2Client } from '../http/v2.js'
import { silentLogger, type Logger } from '../log.js'
import { Directory } from '../resolve/directory.js'

/** Everything a tool handler may use. Built once per server. */
export interface ToolContext {
  config: Config
  http: HttpCore
  v1: V1Client
  v2: V2Client
  catalog: () => Promise<Catalog>
  /** Cached reference data for resolving names (users, roles, teams, projects, states, custom fields). */
  directory: Directory
  log: Logger
}

export interface ContextOptions {
  fetch?: FetchLike
  log?: Logger
  /** Override the catalog (tests). Defaults to the live catalog with snapshot fallback. */
  catalog?: () => Promise<Catalog>
}

export function createContext(config: Config, options: ContextOptions = {}): ToolContext {
  const log = options.log ?? silentLogger
  const httpOptions = { baseUrl: config.baseUrl, token: config.token, log }
  const http = new HttpCore(options.fetch ? { ...httpOptions, fetch: options.fetch } : httpOptions)
  const v1 = new V1Client(http)
  let catalog = options.catalog
  if (!catalog) {
    const provider = new CatalogProvider(v1, { log })
    provider.start()
    catalog = () => provider.get()
  }
  return { config, http, v1, v2: new V2Client(http), catalog, directory: new Directory(v1), log }
}
