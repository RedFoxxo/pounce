import { mkdirSync, writeFileSync } from 'node:fs'
import { dirname, join, relative } from 'node:path'
import { afterAll, afterEach, beforeEach, expect, vi } from 'vitest'

/**
 * Records what every harness test does, for the Rust port to replay: each tool
 * call with its arguments, every request it sent to Targetprocess with the
 * answer it got, and the tool's result. Written only with POUNCE_RECORD=1, to
 * tests/golden/transcripts/. The clock is fixed so dates in requests and
 * results are reproducible.
 */
export const RECORD_NOW = '2026-11-26T10:00:00.000Z'

type Event = Record<string, unknown>
interface Recorded {
  name: string
  /** A request outside any tool call: the test drove the context directly. */
  direct: boolean
  /** Tool calls overlapped. */
  concurrent: boolean
  events: Event[]
  open: number
}

const fresh = (name: string): Recorded => ({ name, direct: false, concurrent: false, events: [], open: 0 })

let current: Recorded | undefined
/** A harness made in beforeAll serves every test of the file: one transcript for the whole file. */
let shared: Recorded | undefined
let testPath: string | undefined
const done: Recorded[] = []

const target = () => shared ?? current

export const recorder = {
  push(event: Event): void {
    if (event.type === 'harness' && !current && !shared) shared = fresh('(shared harness)')
    const t = target()
    if (!t) return
    if (event.type === 'http' && t.open === 0) t.direct = true
    t.events.push(event)
  },
  open(): void {
    const t = target()
    if (!t) return
    t.open++
    if (t.open > 1) t.concurrent = true
  },
  close(): void {
    const t = target()
    if (t) t.open--
  },
}

vi.useFakeTimers({ toFake: ['Date'], now: new Date(RECORD_NOW) })

beforeEach(() => {
  vi.useFakeTimers({ toFake: ['Date'], now: new Date(RECORD_NOW) })
  const state = expect.getState()
  testPath ??= state.testPath
  const name = state.currentTestName ?? ''
  if (shared) shared.events.push({ type: 'test', name })
  else current = fresh(name)
})

afterEach(() => {
  if (current?.events.some((e) => e.type === 'harness')) done.push(current)
  current = undefined
})

afterAll(() => {
  vi.useRealTimers()
  if (shared) done.push(shared)
  if (!process.env.POUNCE_RECORD || !testPath || done.length === 0) return
  const root = join(dirname(new URL(import.meta.url).pathname), '..', '..')
  const name = relative(join(root, 'tests'), testPath).replace(/\.test\.ts$/, '.json')
  const file = join(root, 'tests', 'golden', 'transcripts', name)
  mkdirSync(dirname(file), { recursive: true })
  const tests = done.map(({ open: _, ...test }) => test)
  writeFileSync(file, `${JSON.stringify({ now: RECORD_NOW, tests }, null, 1)}\n`)
})
