/**
 * The Charter tab read a shape the server has never sent, and read every empty
 * list backwards (AGENT-012 §2b; AGENT-014 §2.2; Set S tasks S-11.13, S-11.14).
 *
 * Three separate defects, all of which rendered without an error:
 *
 * 1. **`continuous` was a hard-coded `false`.** The Agenda and Charter tabs are
 *    only listed when a session is continuous, so on a continuous session — the
 *    only kind that has an agenda — neither tab could be opened at all.
 *    `/api/research/sessions` and `/api/research/sessions/{id}` were both served
 *    and `deskApi.sessions` had no caller anywhere in the app.
 *
 * 2. **`CharterTab` took the *form's* flat shape.** `POST …/sessions` accepts
 *    that shape and `desk_charter::translate` converts it; the GET answers the
 *    stored charter, which is nested — a `scope` object, a tagged `objective`,
 *    and the budget, autonomy and schedule as siblings of the charter rather
 *    than fields of it. Handed the response directly, every line would have read
 *    `undefined`, and `$undefined/day` throws no error.
 *
 * 3. **Empty meant "nothing" in the panel and "everything" on the server.**
 *    `Scope::admits` returns true for every subject when the scope names none,
 *    and `Charter::permits` answers yes to every workflow when the list is
 *    empty, and `Schedule::default` with no windows is always open. The tab
 *    printed `—` for all three. A charter that will research anything, at any
 *    hour, with any workflow, rendered as one that will do nothing — and the
 *    person reading it is deciding whether to leave it running for a week.
 *
 * The budget half is the platform's non-negotiable in a fourth place:
 * `translate` writes `trials_total: 0` for a blank field, so a literal render
 * says "0 trials" about a session nobody capped.
 */
import { render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import {
  charterView,
  sessionForConversation,
  type DeskSession,
  type SessionDetail,
} from '@/api/desk'
import { AgendaTab, CharterTab } from '../RightPanel'

function row(over: Partial<DeskSession> = {}): DeskSession {
  return {
    session_id: 's_1',
    conversation_id: 'c_1',
    continuous: true,
    question: null,
    tier: 'frontier',
    created_at: '2026-09-21T09:00:00Z',
    state: 'running',
    runs_started: 3,
    agenda_open: 4,
    ...over,
  }
}

function detail(over: Partial<SessionDetail> = {}): SessionDetail {
  return {
    session_id: 's_1',
    conversation_id: 'c_1',
    continuous: true,
    question: null,
    charter: {
      scope: { instruments: [], asset_classes: [], families: [], timeframes: [], windows: [] },
      objective: { kind: 'build_knowledge', about: ['momentum'] },
      seeds: [{ question: 'does momentum survive costs?', subject: '', workflow: null }],
      permitted_workflows: [],
      exclusions: [],
      standing_constraints: [],
      exploration_share: 0.1,
      stop_when: [],
      delta_practical_default: null,
      version: 1,
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
    charter_version: 1,
    ...over,
  }
}

describe('a conversation finds its session through the list, which is the only map', () => {
  it('matches on the conversation id the list actually carries', () => {
    expect(sessionForConversation([row()], 'c_1')?.session_id).toBe('s_1')
  })

  it('is null for an ordinary chat, and for a list that did not answer', () => {
    expect(sessionForConversation([row()], 'c_other')).toBeNull()
    expect(sessionForConversation(null, 'c_1')).toBeNull()
  })
})

describe('the charter is decoded from the shape the endpoint answers', () => {
  it('takes the objective off the serde tag rather than the whole object', () => {
    expect(charterView(detail())?.objective).toBe('build_knowledge')
  })

  it('names the objective as not recorded rather than guessing the reassuring one', () => {
    const c = charterView(detail({ charter: { ...detail().charter!, objective: {} as never } }))
    expect(c?.objective).toBe('not recorded')
  })

  it('reads the budget off the session, where it is, not off the charter', () => {
    const c = charterView(detail())
    expect(c?.usd_per_day).toBe(25)
    expect(c?.usd_total).toBe(750)
    expect(c?.max_runs).toBe(60)
  })

  it('treats a trials_total of zero as no cap recorded, never as a cap of zero', () => {
    expect(charterView(detail())?.trials_total).toBeNull()
    expect(
      charterView(detail({ budget: { usd_total: 750, usd_per_day: 25, trials_total: 40, max_runs: 60 } }))
        ?.trials_total,
    ).toBe(40)
  })

  it('reads an empty scope as "any", the way Scope::admits does', () => {
    expect(charterView(detail())?.scope_is_any).toBe(true)
    const named = detail()
    named.charter!.scope.instruments = ['BTC-USD']
    expect(charterView(named)?.scope_is_any).toBe(false)
    expect(charterView(named)?.scope.instruments).toEqual(['BTC-USD'])
  })

  it('reads an empty workflow list as all of them, the way Charter::permits does', () => {
    expect(charterView(detail())?.workflows_are_all).toBe(true)
    const named = detail()
    named.charter!.permitted_workflows = ['falsify']
    expect(charterView(named)?.workflows_are_all).toBe(false)
  })

  it('reads no schedule window as always open, the way Schedule::default does', () => {
    expect(charterView(detail())?.always_open).toBe(true)
  })

  it('is null for a session with no charter, which a bounded one has', () => {
    expect(charterView(detail({ charter: null }))).toBeNull()
    expect(charterView(null)).toBeNull()
  })
})

describe('the Charter tab says what the session will do, not the opposite of it', () => {
  it('says an unnamed scope admits everything instead of printing a dash', () => {
    render(<CharterTab charter={charterView(detail())} amendments={[]} />)
    expect(screen.getByTestId('scope-any').textContent).toContain('in scope for everything')
    // The old render, explicitly: four empty lists joined by dashes.
    expect(screen.queryByText('— · — · — · —')).toBeNull()
  })

  it('says an unnamed workflow list is all of them', () => {
    render(<CharterTab charter={charterView(detail())} amendments={[]} />)
    expect(screen.getByTestId('workflows-all').textContent).toContain('all the workflows')
  })

  it('says an unset schedule is any time', () => {
    render(<CharterTab charter={charterView(detail())} amendments={[]} />)
    expect(screen.getByTestId('schedule-any').textContent).toContain('any time')
  })

  it('says there is no trial cap rather than printing zero trials', () => {
    render(<CharterTab charter={charterView(detail())} amendments={[]} />)
    expect(screen.getByText(/no trial cap recorded — not a cap of zero/)).toBeInTheDocument()
    expect(screen.queryByText(/^0 trials$/)).toBeNull()
  })

  it('renders the budget the session carries, so the assertions above are on a real card', () => {
    render(<CharterTab charter={charterView(detail())} amendments={[]} />)
    expect(screen.getByText(/\$25\/day/)).toBeInTheDocument()
    expect(screen.getByText(/keeps generating questions/)).toBeInTheDocument()
  })

  it('does not contradict its own version number when it cannot list the amendments', () => {
    // `PUT …/{id}/charter` bumps the version and no route reads the amendments
    // back, so "amendments: none" under a version 3 charter is the panel
    // disagreeing with the line above it.
    render(<CharterTab charter={charterView(detail({ charter_version: 3 }))} amendments={[]} />)
    const said = screen.getByTestId('amendments-unlistable').textContent ?? ''
    expect(said).toContain('amended 2 times')
    expect(said).toContain('no route reads the amendments back')
  })

  it('still says none when the charter has never been amended', () => {
    render(<CharterTab charter={charterView(detail())} amendments={[]} />)
    expect(screen.queryByTestId('amendments-unlistable')).toBeNull()
    expect(screen.getByText('none')).toBeInTheDocument()
  })

  it('tells "no charter" apart from "not read yet"', () => {
    const { unmount } = render(<CharterTab charter={null} amendments={[]} loading />)
    expect(screen.getByText(/Reading the charter/)).toBeInTheDocument()
    unmount()
    render(<CharterTab charter={null} amendments={[]} />)
    expect(screen.getByText(/has no charter/)).toBeInTheDocument()
  })
})

describe('the Agenda tab tells an unknown board from an empty one', () => {
  it('says the request did not answer, not that there is no board', () => {
    render(<AgendaTab items={null} />)
    const said = screen.getByText(/unknown, not empty/)
    expect(said).toBeInTheDocument()
  })

  it('has a loading state of its own', () => {
    render(<AgendaTab items={[]} loading />)
    expect(screen.getByText(/Reading the agenda/)).toBeInTheDocument()
  })

  it('draws the five empty columns when the board answered with nothing', () => {
    render(<AgendaTab items={[]} />)
    // A genuinely empty board is the five columns at zero, which looks nothing
    // like the sentence above — that difference is the whole point.
    for (const col of ['proposed', 'queued', 'running', 'done', 'vetoed']) {
      expect(screen.getByText(`${col} · 0`)).toBeInTheDocument()
    }
  })
})
