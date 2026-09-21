/**
 * A continuous session's child conversations in the rail (AGENT-012 §2b;
 * AGENT-014 §9; Set S task S-11.13).
 *
 * §2b: "child conversations are grouped under the journal in the left rail and
 * carry a chip in the header back to their agenda card". The rail rendered one
 * flat list, so a session running four questions appeared as five unrelated
 * conversations with titles taken from the questions — and nothing anywhere
 * said which journal they belonged to.
 *
 * **The obvious edge is not on the wire.** `agent_conversations.parent_
 * conversation_id` exists and `api::session_driver` writes it when it opens a
 * child, but `conversations::list` selects six columns and that is not one of
 * them, so `GET /api/agent/conversations` never sends it. Building against it
 * would have produced a rail that groups nothing and looks correct in review.
 *
 * The edge that *is* served is the other end of the same relation:
 * `agenda_items.child_conversation_id`, which `GET …/{id}/agenda` sends — and
 * which is also where the chip's text comes from, because a chip back to an
 * agenda card has to name the question on it.
 *
 * `parent_conversation_id` is still read, optionally and first, so that the day
 * the list route selects it the grouping uses it with no other change.
 */
import { describe, expect, it } from 'vitest'

import type { AgendaItem, DeskSession } from '@/api/desk'
import { childrenFromAgendas, railGroups, type RailConversation } from '@/lib/desk/rail'

function convo(id: string, over: Partial<RailConversation> = {}): RailConversation {
  return { conversation_id: id, title: `title ${id}`, ...over }
}

function session(over: Partial<DeskSession> = {}): DeskSession {
  return {
    session_id: 's_1',
    conversation_id: 'c_journal',
    continuous: true,
    question: null,
    tier: 'frontier',
    created_at: '2026-09-21T09:00:00Z',
    state: 'running',
    runs_started: 2,
    agenda_open: 3,
    ...over,
  }
}

function agendaItem(id: string, child: string | null): AgendaItem {
  return {
    item_id: id,
    question: `does ${id} survive costs?`,
    workflow: 'investigate',
    sources: ['charter_seed'],
    score: null,
    state: child ? 'running' : 'queued',
    child_conversation_id: child,
  }
}

const chips = () =>
  childrenFromAgendas([
    {
      session_id: 's_1',
      conversation_id: 'c_journal',
      items: [agendaItem('i_1', 'c_child'), agendaItem('i_2', null)],
    },
  ])

describe('the children hang under the journal', () => {
  it('finds the edge in the agenda, which is the end the platform serves', () => {
    const map = chips()
    expect(map.size).toBe(1)
    expect(map.get('c_child')).toEqual({
      session_id: 's_1',
      conversation_id: 'c_journal',
      item_id: 'i_1',
      question: 'does i_1 survive costs?',
    })
  })

  it('nests the child and marks the journal', () => {
    const groups = railGroups([convo('c_journal'), convo('c_child'), convo('c_chat')], [session()], chips())
    expect(groups.map((g) => g.conversation.conversation_id)).toEqual(['c_journal', 'c_chat'])
    expect(groups[0].journal).toBe(true)
    expect(groups[0].children.map((c) => c.conversation.conversation_id)).toEqual(['c_child'])
    expect(groups[1].journal).toBe(false)
  })

  it('gives the child a chip naming the agenda card that started it', () => {
    const groups = railGroups([convo('c_journal'), convo('c_child')], [session()], chips())
    expect(groups[0].children[0].chip?.question).toBe('does i_1 survive costs?')
    expect(groups[0].children[0].chip?.item_id).toBe('i_1')
  })

  it('uses parent_conversation_id the moment the route sends it', () => {
    // No agenda at all — only the column the list does not select today.
    const groups = railGroups(
      [convo('c_journal'), convo('c_other', { parent_conversation_id: 'c_journal' })],
      [session()],
      new Map(),
    )
    expect(groups.length).toBe(1)
    expect(groups[0].children.map((c) => c.conversation.conversation_id)).toEqual(['c_other'])
    // And with no agenda behind it there is no chip, which is said rather than
    // faked with the conversation's own title.
    expect(groups[0].children[0].chip).toBeNull()
  })

  it('keeps a child whose journal is not in the list rather than dropping the row', () => {
    const groups = railGroups([convo('c_child')], [session()], chips())
    expect(groups.map((g) => g.conversation.conversation_id)).toEqual(['c_child'])
  })

  it('groups nothing under a bounded session, which has no journal', () => {
    const groups = railGroups(
      [convo('c_journal'), convo('c_child')],
      [session({ continuous: false })],
      chips(),
    )
    // The child still nests — the agenda says so — but nothing is called a
    // journal, because a bounded session's one conversation is the run itself.
    expect(groups[0].journal).toBe(false)
    expect(groups[0].children.length).toBe(1)
  })

  it('leaves every conversation flat when nothing has been read yet', () => {
    const groups = railGroups([convo('c_1'), convo('c_2')], null, new Map())
    expect(groups.length).toBe(2)
    expect(groups.every((g) => !g.journal && g.children.length === 0)).toBe(true)
  })
})
