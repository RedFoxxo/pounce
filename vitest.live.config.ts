import { defineConfig } from 'vitest/config'

// Live smoke tests: real requests against TP_BASE_URL. Never part of `npm test`.
export default defineConfig({
  test: {
    environment: 'node',
    include: ['tests/live/**/*.test.ts'],
    testTimeout: 60_000,
    hookTimeout: 120_000,
    fileParallelism: false,
  },
})
