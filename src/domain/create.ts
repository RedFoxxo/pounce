import type { CatalogResource } from '../catalog/types.js'
import type { CardInfo } from '../resolve/card.js'

/**
 * How a new card points at its parent. Most types use the reference named
 * after the parent's type (Task.UserStory, UserStory.Feature, Feature.Epic,
 * Epic.PortfolioEpic, Bug.UserStory/Feature); test cases join a test plan's
 * TestPlans collection; a test plan links the card it covers via LinkedGeneral.
 */
export function parentLink(
  child: CatalogResource,
  parent: CardInfo,
): { ok: true; body: Record<string, unknown>; field: string } | { ok: false; message: string } {
  if (child.name === 'TestCase' && parent.entityType === 'TestPlan') {
    return { ok: true, field: 'TestPlans', body: { TestPlans: { Items: [{ Id: parent.id }] } } }
  }
  if (child.name === 'TestPlan') {
    return { ok: true, field: 'LinkedGeneral', body: { LinkedGeneral: { Id: parent.id } } }
  }
  const direct = child.references.find((r) => r.canSet && (r.name === parent.entityType || r.type === parent.entityType) && !r.name.startsWith('Linked'))
  if (direct) return { ok: true, field: direct.name, body: { [direct.name]: { Id: parent.id } } }

  const cardTypes = new Set(['PortfolioEpic', 'Epic', 'Feature', 'UserStory', 'Task', 'Bug', 'Request', 'TestPlan'])
  const valid = child.references.filter((r) => r.canSet && cardTypes.has(r.type) && !r.name.startsWith('Linked')).map((r) => r.type)
  if (child.name === 'TestCase') valid.push('TestPlan')
  return {
    ok: false,
    message: `A ${child.name} cannot be created under a ${parent.entityType}. Valid parent types: ${[...new Set(valid)].join(', ') || 'none'}.`,
  }
}

/** Card types pounce creates through write_create_card. */
export function isCardResource(resource: CatalogResource): boolean {
  return (
    resource.canCreate &&
    resource.values.some((f) => f.name === 'Name' && f.canSet) &&
    resource.references.some((r) => r.name === 'Project' && r.canSet)
  )
}
