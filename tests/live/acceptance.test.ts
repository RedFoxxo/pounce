import { afterAll, beforeAll, describe, expect, it } from 'vitest'
import { LIVE, liveClient, liveStory } from './helpers.js'

type Live = Awaited<ReturnType<typeof liveClient>>

/**
 * Live acceptance run: the #36410 batch against a user story YOU name. It
 * WRITES to Targetprocess: the story's state, assignments, role efforts and
 * BackEnd/FrontEnd fields change, and a task and a bug are created under it
 * (deleted again at the end unless TP_LIVE_KEEP=1).
 *
 *   TP_LIVE=1 TP_LIVE_STORY=<story id> \
 *   TP_LIVE_DEVELOPERS="<person>,<person>" TP_LIVE_PRODUCT_OWNER="<person>" \
 *   [TP_LIVE_BUG_ASSIGNEE="<person>"] [TP_LIVE_TEAM="Core Team"] \
 *   npm run test:live -- tests/live/acceptance.test.ts
 */
const developers = (process.env.TP_LIVE_DEVELOPERS ?? '').split(',').map((s) => s.trim()).filter(Boolean)
const productOwner = process.env.TP_LIVE_PRODUCT_OWNER?.trim()
const bugAssignee = process.env.TP_LIVE_BUG_ASSIGNEE?.trim() || developers[0]
const team = process.env.TP_LIVE_TEAM?.trim() || 'Core Team'
const keep = process.env.TP_LIVE_KEEP === '1'
const configured = LIVE && liveStory !== undefined && developers.length === 2 && Boolean(productOwner)

describe.runIf(configured)('live acceptance: the #36410 batch', () => {
  let c: Live
  const story = liveStory as number
  const created: number[] = []

  beforeAll(async () => {
    c = await liveClient()
    const card = await c.call('read_card', { id: story })
    expect(card.isError, card.text).toBe(false)
    expect(card.json.type).toBe('UserStory')
  })
  afterAll(async () => {
    if (!keep) for (const id of created.reverse()) await c.call('delete_card', { id })
    await c.close()
  })

  it('story: state Ready', async () => {
    const r = await c.call('write_set_state', { id: story, state: 'Ready' })
    expect(r.isError, r.text).toBe(false)
    expect(r.json.notPersisted).toBeUndefined()
  })

  it('story: two Developers and a Product Owner', async () => {
    for (const [user, role] of [[developers[0], 'Developer'], [developers[1], 'Developer'], [productOwner, 'Product Owner']] as const) {
      const r = await c.call('write_assign', { id: story, user: user as string, role })
      expect(r.isError, r.text).toBe(false)
      expect(r.json.notPersisted).toBeUndefined()
    }
  })

  it('story: role efforts', async () => {
    const r = await c.call('write_set_role_effort', { id: story, efforts: [{ role: 'Developer', effort: 16 }, { role: 'Product Owner', effort: 2 }] })
    expect(r.isError, r.text).toBe(false)
    expect(r.json.notPersisted).toBeUndefined()
  })

  it('story: clear BackEnd, set FrontEnd', async () => {
    const r = await c.call('write_set_custom_fields', { id: story, fields: { BackEnd: null, FrontEnd: 'Doing' } })
    expect(r.isError, r.text).toBe(false)
    expect(r.json.notPersisted).toBeUndefined()
  })

  it('task in Coded with the team, 1h Developer effort and one assignee', async () => {
    const r = await c.call('write_create_card', {
      type: 'Task',
      parent: story,
      name: 'pounce acceptance task',
      state: 'Coded',
      teams: [team],
      roleEfforts: [{ role: 'Developer', effort: 1 }],
      assignees: [{ user: developers[0], role: 'Developer' }],
    })
    expect(r.isError, r.text).toBe(false)
    created.push(r.json.created.id)
    expect(r.json.notPersisted).toBeUndefined()
    expect(r.json.card.assignments).toHaveLength(1)
    expect(r.json.parent).toBeDefined()
  })

  it('bug assigned to one person, defaults removed', async () => {
    const r = await c.call('write_create_card', { type: 'Bug', parent: story, name: 'pounce acceptance bug', assignees: [{ user: bugAssignee, role: 'Developer' }] })
    expect(r.isError, r.text).toBe(false)
    created.push(r.json.created.id)
    expect(r.json.notPersisted).toBeUndefined()
    expect(r.json.card.assignments).toHaveLength(1)
  })
})
