import { defineConfig } from 'vitest/config'

export default defineConfig({
  test: {
    environment: 'node',
    include: ['tests/**/*.test.ts'],
    exclude: ['tests/live/**'],
    // Fixed clock, and transcripts for the Rust port with POUNCE_RECORD=1.
    setupFiles: ['tests/helpers/record.ts'],
  },
})
