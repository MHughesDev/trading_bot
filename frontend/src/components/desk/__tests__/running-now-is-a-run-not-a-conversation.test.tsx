/**
 * Desk home's *Running now* panel, against the route built for it (AGENT-012
 * §2.2).
 *
 * `GET /api/desk/running` answers one row per working **run** with every column
 * §2.2 asks for — status, mode, stage, model, workflow, tier, profile,
 * iterations, prompt and cache-read tokens, trials, outcome, and the last entry
 * in the run's own transcript — and nothing called it. The panel filtered the
 * conversation list on `running`, which answers a different question ("is any
 * turn active in this conversation") and none of the other columns: a
 * conversation with three finished turns and one working one has one tier and
 * one token count that matter, and the list cannot say which.
 *
 * Three properties, and each of them is a way a number gets invented:
 *
 * 1. **`not_recorded` is rendered.** The route sends an array naming the columns
 *    §2.2 asks for that nothing here records — per-run dollars (`llm_usage.
 *    cost_usd` keys on an auth session, so asking it for a run returns zero) and
 *    the working-state step (its route is unbuilt). Dropping that array turns a
 *    stated absence into a silent one, and a blank cost column reads as free.
 * 2. **No cache-hit percentage.** Both token counts are shown; the ratio is not
 *    computed, because `prompt_tokens` is nullable and a percentage over a
 *    missing denominator is invented rather than measured.
 * 3. **`null` is never zero.** Migration 0062 made the telemetry columns
 *    nullable so a run that crashed before filling them is still a run. "0
 *    trials spent" is a run that searched nothing; absent is a run that did not
 *    record what it searched.
 */
import { render as rtlRender, screen } from '@testing-library/react'
import { MemoryRouter } from 'react-router-dom'
import type { ReactElement } from 'react'
import { describe, expect, it, vi } from 'vitest'

import type { Conversation } from '@/api/agent'
import type { DeskRunningRun } from '@/api/desk'
import { RunningNow, runCount } from '../DeskHome'

vi.mock('@/api/desk', () => ({
  deskApi: { cancel: vi.fn(() => Promise.resolve({ data: {} })), haltAll: vi.fn() },
}))

function render(ui: ReactElement) {
  return rtlRender(<MemoryRouter>{ui}</MemoryRouter>)
}

function run(over: Partial<DeskRunningRun> = {}): DeskRunningRun {
  return {
    run_id: 'r_1',
    conversation_id: 'c_1',
    title: 'does momentum survive costs?',
    status: 'running',
    mode: 'research',
    stage: 'working',
    model: 'claude-opus-5',
    workflow: 'investigate',
    tier: 'frontier',
    profile_name: 'frontier-default',
    iterations: 12,
    prompt_tokens: 48_000,
    cache_read_tokens: 120_000,
    trials_spent: 3,
    outcome: null,
    created_at: '2026-09-21T09:00:00Z',
    started_at: '2026-09-21T09:01:00Z',
    last_transcript_entry: { seq: 88, kind: 'tool_result', at: '2026-09-21T09:20:00Z' },
    ...over,
  }
}

const notRecorded = [
  { field: 'cost_usd', why: 'llm_usage.cost_usd keys on an agent_sessions id, not on a run.' },
  { field: 'working_state_step', why: 'the per-run working-state snapshot route is unbuilt.' },
]

const convo: Conversation = {
  conversation_id: 'c_1',
  title: 'the momentum session',
  provider: 'anthropic',
  model: 'claude-opus-5',
  created_at: '2026-09-21T08:00:00Z',
  last_activity_at: '2026-09-21T09:20:00Z',
  running: true,
  turns: 4,
}

function panel(over: {
  runs?: DeskRunningRun[] | null
  conversations?: Conversation[] | null
  loading?: boolean
} = {}) {
  return render(
    <RunningNow
      runs={over.runs === undefined ? [run()] : over.runs}
      notRecorded={notRecorded}
      loading={over.loading ?? false}
      conversations={over.conversations === undefined ? [convo] : over.conversations}
    />,
  )
}

describe('a count nobody recorded is not a count of zero', () => {
  it('says so in words rather than printing a number', () => {
    expect(runCount(null, 'trials spent')).toBe('trials spent not recorded')
    expect(runCount(0, 'trials spent')).toBe('0 trials spent')
    expect(runCount(48_000, 'prompt tokens')).toContain('48,000')
  })

  it('keeps a run that recorded nothing legible', () => {
    panel({
      runs: [
        run({
          iterations: null,
          prompt_tokens: null,
          cache_read_tokens: null,
          trials_spent: null,
          tier: null,
          profile_name: null,
          started_at: null,
          last_transcript_entry: null,
        }),
      ],
    })
    const card = screen.getByTestId('run-r_1').textContent ?? ''
    expect(card).toContain('prompt tokens not recorded')
    expect(card).toContain('trials spent not recorded')
    expect(card).toContain('tier not recorded')
    expect(card).toContain('no transcript entry yet')
    expect(card).toContain('not started yet')
    // The reading that would have been invented for every one of them.
    expect(card).not.toContain('0 trials spent')
    expect(card).not.toContain('0 prompt tokens')
  })
})

describe('the two token counts are shown and never divided', () => {
  it('prints both, with no percentage anywhere on the row', () => {
    panel()
    const tokens = screen.getByTestId('tokens-r_1').textContent ?? ''
    expect(tokens).toContain('48,000 prompt tokens')
    expect(tokens).toContain('120,000 cache read tokens')
    expect(tokens).not.toContain('%')
    expect(screen.getByTestId('run-r_1').textContent).not.toContain('%')
  })

  it('shows the cache reads even when the prompt size is missing, without a ratio', () => {
    panel({ runs: [run({ prompt_tokens: null })] })
    const tokens = screen.getByTestId('tokens-r_1').textContent ?? ''
    expect(tokens).toContain('prompt tokens not recorded')
    expect(tokens).toContain('120,000 cache read tokens')
    expect(tokens).not.toContain('%')
  })
})

describe('what the route says it cannot measure is on the screen', () => {
  it('renders every not_recorded entry with its reason', () => {
    panel()
    const said = screen.getByTestId('running-not-recorded').textContent ?? ''
    expect(said).toContain('cost_usd')
    expect(said).toContain('keys on an agent_sessions id')
    expect(said).toContain('working_state_step')
    expect(said).toContain('unbuilt')
  })

  it('does not print a dollar figure of its own anywhere', () => {
    panel()
    expect(screen.getByTestId('run-r_1').textContent).not.toContain('$')
  })
})

describe('the rows are runs, joined to the conversation list', () => {
  it('opens the conversation the run belongs to', () => {
    panel()
    const link = screen.getByRole('link', { name: 'does momentum survive costs?' })
    expect(link.getAttribute('href')).toBe('/desk/c/c_1')
  })

  it('falls back to the conversation title when the run carries none', () => {
    panel({ runs: [run({ title: null })] })
    expect(screen.getByRole('link', { name: 'the momentum session' })).toBeInTheDocument()
  })

  it('renders a run with no conversation as text, not as a link that opens nothing', () => {
    panel({ runs: [run({ conversation_id: null, title: 'an orphan run' })] })
    expect(screen.queryByRole('link', { name: 'an orphan run' })).toBeNull()
    expect(screen.getByText('an orphan run')).toBeInTheDocument()
    // Nothing to cancel, so no control that would reach nothing.
    expect(screen.queryByRole('button', { name: 'Stop' })).toBeNull()
  })

  it('says a chat turn has no workflow rather than leaving the badge blank', () => {
    panel({ runs: [run({ workflow: null, mode: 'chat' })] })
    expect(screen.getByTestId('run-r_1').textContent).toContain('no workflow')
  })
})

describe('a failed read is not an idle desk', () => {
  it('says what is working is unknown, and keeps what the conversation list still knows', () => {
    panel({ runs: null })
    expect(screen.getByText(/The run list could not be read/)).toBeInTheDocument()
    const detail = screen.getByText(/is served/).textContent ?? ''
    expect(detail).toContain('1 conversation is working')
    expect(screen.queryByText('Nothing is running')).toBeNull()
  })

  it('says unknown, not none, when neither read answered', () => {
    panel({ runs: null, conversations: null })
    expect(screen.getByText(/unknown, not none/)).toBeInTheDocument()
  })

  it('tells an empty answer from a failed one', () => {
    panel({ runs: [] })
    expect(screen.getByText('Nothing is running')).toBeInTheDocument()
    expect(screen.queryByText(/could not be read/)).toBeNull()
  })

  it('has a reading state that claims nothing at all', () => {
    panel({ loading: true })
    expect(screen.queryByText('Nothing is running')).toBeNull()
    expect(screen.queryByTestId('running-runs')).toBeNull()
  })
})
