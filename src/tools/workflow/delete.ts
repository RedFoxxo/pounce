import { z } from 'zod'
import { readCardRaw, shapeCard } from '../../domain/cards.js'
import { parentChange, parentSnapshot, snapshotAgain } from '../../domain/parent.js'
import { num, ref } from '../../format/shape.js'
import { resolveWritable } from '../generic/policy.js'
import { failure, invalid, success } from '../respond.js'
import { id } from '../schema.js'
import { defineTool } from '../types.js'

type Raw = Record<string, unknown>

export const deleteCard = defineTool({
  name: 'delete_card',
  description:
    'Delete a card by id alone (its type is resolved). Reports what was deleted, how many children it had (Targetprocess may delete them ' +
    "with it), and the parent card's state and efforts before/after.",
  input: { id },
  handler: async (args, ctx) => {
    const card = await readCardRaw(ctx, args.id)
    if (!card.ok) return card.error ? failure(card.message, card.error) : invalid(card.message)
    const { info, raw } = card.value
    const allowed = resolveWritable(await ctx.catalog(), info.resource.name, 'delete', 'delete')
    if (!allowed.ok) return invalid(allowed.message)
    const shaped = shapeCard(info, raw)
    const parentBefore = await parentSnapshot(ctx, raw)

    const r = await ctx.v1.delete(info.resource.path, info.id)
    if (!r.ok) return failure(`Could not delete ${info.entityType} ${info.id}`, r)
    const gone = await ctx.v1.get(info.resource.path, info.id, { include: '[Id]' })
    const parentAfter = await snapshotAgain(ctx, parentBefore)
    return success({
      deleted: { id: info.id, type: info.entityType, name: shaped.name ?? info.name, state: shaped.state?.name, project: shaped.project },
      ...(shaped.counts && Object.values(shaped.counts).some((n) => n > 0) ? { hadChildren: shaped.counts } : {}),
      ...(parentBefore ? { parent: parentChange(parentBefore, parentAfter) } : {}),
      ...(gone.ok ? { notPersisted: ['the card can still be read after deleting it'] } : {}),
    })
  },
})

export const deleteRelation = defineTool({
  name: 'delete_relation',
  description: 'Delete the relation between two cards, in either direction. If they have several relations, pass relation (type name) to pick one.',
  input: { id, to: id, relation: z.string().optional() },
  handler: async (args, ctx) => {
    const include = '[Id,RelationType[Id,Name],Master[Id,Name],Slave[Id,Name]]'
    const [a, b] = await Promise.all([
      ctx.v1.list<Raw>('Relations', { where: `(Master.Id eq ${args.id}) and (Slave.Id eq ${args.to})`, include }),
      ctx.v1.list<Raw>('Relations', { where: `(Master.Id eq ${args.to}) and (Slave.Id eq ${args.id})`, include }),
    ])
    if (!a.ok) return failure('Could not read relations', a)
    if (!b.ok) return failure('Could not read relations', b)
    let found = [...a.data.items, ...b.data.items]
    if (args.relation) found = found.filter((r) => ref(r.RelationType)?.name?.toLowerCase() === args.relation!.trim().toLowerCase())
    const describe = (r: Raw) => ({ id: num(r.Id), type: ref(r.RelationType)?.name, master: ref(r.Master), slave: ref(r.Slave) })
    if (found.length === 0) return invalid(`no${args.relation ? ` ${args.relation}` : ''} relation between ${args.id} and ${args.to}`)
    if (found.length > 1) return invalid(`${found.length} relations between ${args.id} and ${args.to} (${found.map((r) => ref(r.RelationType)?.name).join(', ')}); pass relation`)
    const target = found[0] as Raw
    const r = await ctx.v1.delete('Relations', num(target.Id) as number)
    if (!r.ok) return failure(`Could not delete relation ${String(target.Id)}`, r)
    return success({ deleted: describe(target) })
  },
})

export const workflowDeleteTools = [deleteCard, deleteRelation]
