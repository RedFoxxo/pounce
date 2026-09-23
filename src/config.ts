export interface Config {
  /** Instance root without a trailing slash, e.g. `https://mamami.tpondemand.com`. */
  baseUrl: string
  token: string
  defaultProjectId?: number
  defaultTeamId?: number
}

export class ConfigError extends Error {
  override name = 'ConfigError'
}

const ID = /^\d+$/

function optionalId(env: NodeJS.ProcessEnv, key: string, problems: string[]): number | undefined {
  const raw = env[key]?.trim()
  if (!raw) return undefined
  const value = Number(raw)
  if (!ID.test(raw) || !Number.isSafeInteger(value) || value <= 0) {
    problems.push(`${key} must be a positive numeric id, got "${raw}"`)
    return undefined
  }
  return value
}

/** Reads and validates the environment. Throws `ConfigError` listing every problem at once. */
export function loadConfig(env: NodeJS.ProcessEnv = process.env): Config {
  const problems: string[] = []

  const rawUrl = env.TP_BASE_URL?.trim()
  let baseUrl = ''
  if (!rawUrl) {
    problems.push('TP_BASE_URL is required, e.g. https://yourcompany.tpondemand.com')
  } else {
    try {
      const url = new URL(rawUrl)
      const local = ['localhost', '127.0.0.1', '[::1]'].includes(url.hostname)
      if (url.protocol !== 'https:' && !(url.protocol === 'http:' && local)) {
        problems.push(`TP_BASE_URL must be an https URL (the token travels in the query string), got "${rawUrl}"`)
      } else if (/\/api(\/|$)/i.test(url.pathname)) {
        problems.push(`TP_BASE_URL must be the instance root without /api/..., got "${rawUrl}"`)
      } else if (url.search || url.hash) {
        problems.push('TP_BASE_URL must not contain a query string or fragment')
      } else {
        baseUrl = `${url.origin}${url.pathname}`.replace(/\/+$/, '')
      }
    } catch {
      problems.push(`TP_BASE_URL is not a valid URL: "${rawUrl}"`)
    }
  }

  const token = env.TP_TOKEN?.trim() ?? ''
  if (!token) problems.push('TP_TOKEN is required (a Targetprocess access token)')

  const defaultProjectId = optionalId(env, 'TP_DEFAULT_PROJECT_ID', problems)
  const defaultTeamId = optionalId(env, 'TP_DEFAULT_TEAM_ID', problems)

  if (problems.length > 0) {
    throw new ConfigError(`Invalid pounce configuration:\n- ${problems.join('\n- ')}`)
  }

  const config: Config = { baseUrl, token }
  if (defaultProjectId !== undefined) config.defaultProjectId = defaultProjectId
  if (defaultTeamId !== undefined) config.defaultTeamId = defaultTeamId
  return config
}
