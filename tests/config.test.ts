import { describe, expect, it } from 'vitest'
import { ConfigError, loadConfig } from '../src/config.js'

const base = { TP_BASE_URL: 'https://example.tpondemand.com', TP_TOKEN: 'secret' }

describe('loadConfig', () => {
  it('accepts the required variables', () => {
    expect(loadConfig(base)).toEqual({ baseUrl: 'https://example.tpondemand.com', token: 'secret' })
  })

  it('strips a trailing slash', () => {
    expect(loadConfig({ ...base, TP_BASE_URL: 'https://example.tpondemand.com/' }).baseUrl).toBe(
      'https://example.tpondemand.com',
    )
  })

  it('reports every missing variable at once', () => {
    expect(() => loadConfig({})).toThrow(ConfigError)
    try {
      loadConfig({})
    } catch (error) {
      expect((error as Error).message).toMatch(/TP_BASE_URL is required/)
      expect((error as Error).message).toMatch(/TP_TOKEN is required/)
    }
  })

  it('rejects an /api suffix', () => {
    expect(() => loadConfig({ ...base, TP_BASE_URL: 'https://example.tpondemand.com/api/v1' })).toThrow(/without \/api/)
  })

  it('parses optional ids and omits unset ones', () => {
    expect(loadConfig({ ...base, TP_DEFAULT_PROJECT_ID: '42', TP_DEFAULT_TEAM_ID: '' })).toEqual({
      baseUrl: 'https://example.tpondemand.com',
      token: 'secret',
      defaultProjectId: 42,
    })
  })

  it('rejects non-numeric ids', () => {
    expect(() => loadConfig({ ...base, TP_DEFAULT_TEAM_ID: 'core' })).toThrow(/TP_DEFAULT_TEAM_ID must be a numeric id/)
  })
})
