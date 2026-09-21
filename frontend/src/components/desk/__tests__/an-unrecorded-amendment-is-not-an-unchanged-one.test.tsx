/**
 * The Charter tab reported that decisions had been taken and showed none of
 * them (AGENT-012 §2b; AGENT-014 §2.2).
 *
 * `PUT …/{id}/charter` overwrites the charter column in place, so for a long
 * time the only trace an amendment left was a version number — and a tab that
 * can say "version 4" and nothing else is telling the operator that three
 * decisions were made about what this session may do and none of them can be
 * read. The route now computes the diff where both documents are in hand,
 * writes it into the append-only event, and `GET /api/research/sessions/{id}`
 * reads the events back as `amendments[]`. Nothing on this side read them.
 *
 * The one distinction this file exists for: **`changed: null` is not `{}`.**
 *
 * * `{}` is a measurement — both charters were compared, nothing differed.
 * * `null` is an absence — the amendment was appended before the diff existed,
 *   and what it moved was never written down.
 *
 * Rendering the second as "no changes" would turn a missing record into a claim
 * about somebody's decision, which is the same defect as an empty budget bar
 * read as nothing spent. The route keeps the two apart deliberately
 * (`payload.changed.filter(is_object)`); so does this.
 */
import { render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import { charterView, sessionAmendments, type SessionDetail } from '@/api/desk'
import { CharterTab } from '../RightPanel'

function detail(over: Partial<SessionDetail> = {}): SessionDetail {
  return {
    session_id: 's_1',
    conversation_id: 'c_1',
    continuous: true,
    question: null,
    charter: {
      scope: { instruments: [], asset_classes: [], families: [], timeframes: [], windows: [] },
      objective: { kind: 'build_knowledge', about: ['momentum'] },
      seeds: [],
      permitted_workflows: [],
      exclusions: [],
      standing_constraints: [],
      exploration_share: 0.1,
      stop_when: [],
      delta_practical_default: null,
      version: 3,
    },
    budget: { usd_total: 750, usd_per_day: 25, trials_total: 0, max_runs: 60 },
    schedule: { windows: [], max_concurrent_runs: 1 },
    autonomy: 'notify',
    replenish: true,
    review_every: { kind: 'runs', runs: 5 },
    tier: 'frontier',
    exploration_share: 0.1,
    state: 'running',
    runs_started: 3,
    charter_version: 3,
    ...over,
  }
}

const withDiff = {
  seq: 42,
  version: 3,
  by: 'mason',
  changed: {
    exploration_share: { from: 0.1, to: 0.25 },
    standing_constraints: { from: [], to: ['no leverage'] },
  },
  at: '2026-09-21T09:00:00Z',
}

const withoutDiff = { seq: 7, version: 2, by: null, changed: null, at: '2026-09-20T09:00:00Z' }

const withEmptyDiff = { seq: 9, version: 2, by: 'mason', changed: {}, at: '2026-09-20T10:00:00Z' }

describe('the amendments behind the version number are read', () => {
  it('decodes each field of the diff into a from and a to', () => {
    const [a] = sessionAmendments(detail({ amendments: [withDiff] }))
    expect(a.version).toBe(3)
    expect(a.by).toBe('mason')
    expect(a.silence).toBeNull()
    expect(a.changes).toEqual([
      { field: 'exploration_share', from: '0.1', to: '0.25' },
      // An empty list is "nothing named", not a blank: the charter reads an
      // empty scope as everything, and a blank cell would say the opposite of
      // both readings.
      { field: 'standing_constraints', from: 'nothing named', to: 'no leverage' },
    ])
  })

  it('tells an amendment nobody wrote a diff for from one that changed nothing', () => {
    expect(sessionAmendments(detail({ amendments: [withoutDiff] }))[0].silence).toBe('unrecorded')
    expect(sessionAmendments(detail({ amendments: [withEmptyDiff] }))[0].silence).toBe('none')
  })

  it('is empty for a session read that carried no amendments field at all', () => {
    // Absent is not "never amended" — the tab's version line is what says that,
    // and it is checked below.
    expect(sessionAmendments(detail())).toEqual([])
    expect(sessionAmendments(null)).toEqual([])
  })
})

describe('the Charter tab shows what was decided, not only how often', () => {
  it('lists the fields an amendment moved, with both sides', () => {
    render(
      <CharterTab
        charter={charterView(detail())}
        amendments={sessionAmendments(detail({ amendments: [withDiff] }))}
      />,
    )
    const said = screen.getByTestId('amendments').textContent ?? ''
    expect(said).toContain('version 3')
    expect(said).toContain('mason')
    expect(said).toContain('exploration_share: 0.1 → 0.25')
    expect(said).toContain('standing_constraints: nothing named → no leverage')
  })

  it('says an old amendment was recorded before amendments were legible', () => {
    render(
      <CharterTab
        charter={charterView(detail())}
        amendments={sessionAmendments(detail({ amendments: [withoutDiff] }))}
      />,
    )
    const said = screen.getByTestId('amendment-unrecorded-7').textContent ?? ''
    expect(said).toContain('recorded before amendments were legible')
    // The words that would make the absence look like a finding.
    expect(said).not.toContain('no changes')
    expect(screen.queryByTestId('amendment-empty-7')).toBeNull()
  })

  it('says the other thing — that nothing differed — only when the diff is empty', () => {
    render(
      <CharterTab
        charter={charterView(detail())}
        amendments={sessionAmendments(detail({ amendments: [withEmptyDiff] }))}
      />,
    )
    expect(screen.getByTestId('amendment-empty-9').textContent).toContain('no field differed')
    expect(screen.queryByTestId('amendment-unrecorded-9')).toBeNull()
  })

  it('keeps the two apart on one board, which is how a session that ran for weeks reads', () => {
    render(
      <CharterTab
        charter={charterView(detail())}
        amendments={sessionAmendments(detail({ amendments: [withDiff, withoutDiff] }))}
      />,
    )
    expect(screen.getByTestId('amendments').textContent).toContain('exploration_share')
    expect(screen.getByTestId('amendment-unrecorded-7')).toBeInTheDocument()
    // The version-only fallback is for a read that listed none; with a list, it
    // must not also be on screen saying the amendments cannot be shown.
    expect(screen.queryByTestId('amendments-unlistable')).toBeNull()
  })
})
