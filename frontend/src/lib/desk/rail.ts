// Grouping a session's child conversations under its journal (AGENT-012 §2b;
// AGENT-014 §9; Set S task S-11.13).
//
// # Why this is derived from the agenda rather than from the conversation
//
// §2b says the children are "grouped under the journal in the left rail and
// carry a chip back to their agenda card". `agent_conversations.parent_
// conversation_id` exists for exactly that and `api::session_driver` writes it
// when it opens a child — but **`GET /api/agent/conversations` does not select
// it**. `conversations::list`'s SQL names six columns and that is not one of
// them, so the rail cannot see the edge the database holds.
//
// What *is* served is the other end of the same edge: an agenda item carries
// `child_conversation_id`, and `GET …/{id}/agenda` sends it. That is the edge
// this file walks, which also happens to be the edge that carries the chip —
// the child has to name its agenda card, and the agenda card is where the
// question lives.
//
// `parent_conversation_id` is read too, optionally, and takes precedence when
// it is there: the day the list route selects it, a child whose agenda item has
// since been pruned is still grouped, and nothing here has to change.

import type { AgendaItem, DeskSession } from '@/api/desk'

/** The bit of a conversation row the rail needs. */
export interface RailConversation {
  conversation_id: string
  title: string | null
  /**
   * Not sent by `GET /api/agent/conversations` today. Optional so that the rail
   * uses it the moment it is, and groups from the agenda until then.
   */
  parent_conversation_id?: string | null
}

/** The chip a child carries back to the agenda card that started it. */
export interface AgendaChip {
  session_id: string
  /** The journal conversation, which is what the chip navigates to. */
  conversation_id: string
  item_id: string
  question: string
}

export interface RailChild<C extends RailConversation> {
  conversation: C
  /** `null` when the child is grouped by its parent id and no item claims it. */
  chip: AgendaChip | null
}

export interface RailGroup<C extends RailConversation> {
  conversation: C
  /** True when this conversation is a continuous session's journal. */
  journal: boolean
  children: Array<RailChild<C>>
}

/**
 * Every child conversation an agenda names, keyed by the child.
 *
 * Built from the agendas the rail has actually read. A session whose agenda has
 * not been fetched contributes nothing, which means its children stay at the
 * top level rather than disappearing.
 */
export function childrenFromAgendas(
  agendas: ReadonlyArray<{ session_id: string; conversation_id: string; items: readonly AgendaItem[] }>,
): Map<string, AgendaChip> {
  const out = new Map<string, AgendaChip>()
  for (const a of agendas) {
    for (const item of a.items) {
      const child = item.child_conversation_id
      if (typeof child !== 'string' || child === '') continue
      out.set(child, {
        session_id: a.session_id,
        conversation_id: a.conversation_id,
        item_id: item.item_id,
        question: item.question,
      })
    }
  }
  return out
}

/**
 * The rail's list: journals with their children nested, everything else flat,
 * all in the order the conversations arrived.
 *
 * A child whose journal is **not** in the list stays at the top level. Hiding
 * it would be worse than not grouping it: the conversation exists, the person
 * can open it, and a rail that silently drops rows is a rail nobody can trust
 * to be the whole list.
 */
export function railGroups<C extends RailConversation>(
  conversations: readonly C[],
  sessions: readonly DeskSession[] | null | undefined,
  chips: ReadonlyMap<string, AgendaChip>,
): Array<RailGroup<C>> {
  const journals = new Set(
    (sessions ?? []).filter((s) => s.continuous).map((s) => s.conversation_id),
  )
  const byId = new Map(conversations.map((c) => [c.conversation_id, c]))

  const parentOf = (c: C): string | null => {
    const declared = c.parent_conversation_id
    if (typeof declared === 'string' && declared !== '' && byId.has(declared)) return declared
    const chip = chips.get(c.conversation_id)
    if (chip && byId.has(chip.conversation_id)) return chip.conversation_id
    return null
  }

  const groups: Array<RailGroup<C>> = []
  const index = new Map<string, RailGroup<C>>()
  for (const c of conversations) {
    // A conversation that is its own parent, or a journal, is never a child.
    if (parentOf(c) !== null && parentOf(c) !== c.conversation_id) continue
    const g: RailGroup<C> = { conversation: c, journal: journals.has(c.conversation_id), children: [] }
    groups.push(g)
    index.set(c.conversation_id, g)
  }
  for (const c of conversations) {
    const parent = parentOf(c)
    if (parent === null || parent === c.conversation_id) continue
    const g = index.get(parent)
    const chip = chips.get(c.conversation_id) ?? null
    if (g) g.children.push({ conversation: c, chip })
    else {
      // The parent is itself somebody's child — a chain the rail does not draw
      // two levels of. Keep the row at the top level rather than lose it.
      const own: RailGroup<C> = { conversation: c, journal: false, children: [] }
      groups.push(own)
      index.set(c.conversation_id, own)
    }
  }
  return groups
}
