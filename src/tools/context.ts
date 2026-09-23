import type { Config } from '../config.js'

/** Everything a tool handler may use. Built once per server. */
export interface ToolContext {
  config: Config
}
