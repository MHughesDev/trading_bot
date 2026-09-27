/**
 * What the desk announces, and the two things it refuses to announce
 * (AGENT-012 §15; Set S task S-6.22's third clause).
 *
 * The acceptance names three announcements — new approval, new question, run
 * finished — and the third is the one with a trap in it. The conversation
 * screen holds `running = convo.data?.conversation.running === true`, which is
 * `false` for a run that is idle **and** `false` for a conversation the platform
 * would not return. Wiring that boolean straight into a finish announcement
 * would shout *"run finished"* every time the desk lost sight of a run: the same
 * defect as the header badge that read **paused** over a run still spending
 * trials, except said out loud to somebody who cannot see the screen.
 *
 * So `running` is three-valued here and only `true → false` announces.
 *
 * The other refusal is history. A conversation opened with nine pending
 * approvals must announce nothing at all — see the hook's test.
 *
 * WHICH OF THESE WOULD HAVE FAILED BEFORE: all of them. `lib/desk/announce.ts`
 * did not exist and no `aria-live` region on this platform announced an
 * approval, a question or a finished run; only the kill-switch banner had one.
 */
import { describe, expect, it } from 'vitest'

import type { DeskCard } from '@/api/desk'
import { cardAnnouncements, isDecisionCard, runFinishedAnnouncement } from '../announce'
import { pendingApprovals } from '@/components/desk/RightPanel.helpers'

function card(over: Partial<DeskCard> & { seq: number; kind: DeskCard['kind'] }): DeskCard {
  return { step: 1, at: '2026-09-22T10:00:00Z', headline: 'headline', ...over }
}

const stream: DeskCard[] = [
  card({ seq: 1, kind: 'plan', headline: 'sweep the parameter grid' }),
  card({ seq: 2, kind: 'approval', headline: 'spend a holdout look', body: 'two trials left' }),
  card({ seq: 3, kind: 'tool', headline: 'get_bars' }),
  card({ seq: 4, kind: 'question', headline: 'which venue should this assume?' }),
  card({ seq: 5, kind: 'proposal', headline: 'promote to paper' }),
]

describe('the announcements are exactly the decisions and the questions', () => {
  it('announces an approval, a proposal and a question and nothing else', () => {
    const a = cardAnnouncements(stream)
    expect(a.map((x) => x.kind)).toEqual(['approval', 'question', 'approval'])
    expect(a.map((x) => x.key)).toEqual(['approval:2', 'question:4', 'approval:5'])
  })

  it('reads as a sentence about what is waiting, not as a card kind', () => {
    const [approval, question] = cardAnnouncements(stream)
    expect(approval.text).toBe('Waiting on you: spend a holdout look')
    expect(question.text).toBe('The agent asked: which venue should this assume?')
  })

  it('cannot disagree with the queue the right panel draws', () => {
    // Two predicates over the same cards, in two modules that must not import
    // each other — the announcement layer is a lib and `pendingApprovals` lives
    // beside a component. A divergence would announce something the queue never
    // shows, or show something it never announced.
    expect(stream.filter(isDecisionCard)).toEqual(pendingApprovals(stream))
  })

  it('never announces an empty string when a card carried no headline', () => {
    const a = cardAnnouncements([card({ seq: 9, kind: 'approval', headline: '   ' })])
    expect(a[0].text).toBe('Waiting on you: an approval with no headline')
  })

  it('gives every card a key of its own, so two approvals in a step are two announcements', () => {
    const a = cardAnnouncements([
      card({ seq: 7, kind: 'approval', headline: 'one' }),
      card({ seq: 8, kind: 'approval', headline: 'two' }),
    ])
    expect(new Set(a.map((x) => x.key)).size).toBe(2)
  })
})

describe('a run finished only when the desk watched it finish', () => {
  it('announces a genuine stop', () => {
    const a = runFinishedAnnouncement(true, false, 'funding carry, BTC-USD')
    expect(a?.text).toBe('Run finished: funding carry, BTC-USD')
  })

  it('says nothing when the desk lost sight of the run', () => {
    // `null` is "cannot be read". This is the case the whole three-valued shape
    // exists for: a refused or unloaded conversation answering `false` would
    // announce a finish for a run that is still working.
    expect(runFinishedAnnouncement(true, null, 'x')).toBeNull()
  })

  it('says nothing when the desk never saw it running', () => {
    expect(runFinishedAnnouncement(null, false, 'x')).toBeNull()
    expect(runFinishedAnnouncement(false, false, 'x')).toBeNull()
  })

  it('says nothing about a run that started', () => {
    expect(runFinishedAnnouncement(false, true, 'x')).toBeNull()
    expect(runFinishedAnnouncement(null, true, 'x')).toBeNull()
  })

  it('names no title rather than announcing an empty one', () => {
    expect(runFinishedAnnouncement(true, false, null)?.text).toBe('Run finished')
    expect(runFinishedAnnouncement(true, false, '  ')?.text).toBe('Run finished')
  })
})
