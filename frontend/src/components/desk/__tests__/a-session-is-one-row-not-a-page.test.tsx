/**
 * S-11.13 (the desk half), S-10.15 (proposals in *Needs you*) and S-6.14 (the
 * Start panel sends the mode with the turn).
 *
 * What was missing, and what each block below would have failed on:
 *
 *  - desk home had no continuous session anywhere: not in *Running now*, and a
 *    stalled one was not in *Needs you* — so a session waiting on a human for
 *    its diagnosis was a page nobody visited;
 *  - the agenda's **Add a question** was a button with no handler at its only
 *    call site, and its running cards drew no stage;
 *  - the charter form had no schedule grid, and offered `replenish` and
 *    `autonomous`, both of which the server refuses with a 400;
 *  - the journal's `/pause`, `/resume`, `/agenda add` and `/charter` did not
 *    exist; a charter could be read but never amended;
 *  - the Start panel wrote `mode=research` into the message *text*, which the
 *    conversation route (`{text}` only) never parses — and swallowed every
 *    refusal into a dropped promise.
 */
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { MemoryRouter } from 'react-router-dom'
import type { ReactElement } from 'react'
import { beforeEach, describe, expect, it, vi } from 'vitest'

const api = vi.hoisted(() => ({
  sessionAction: vi.fn(),
  createSession: vi.fn(),
  decideProposal: vi.fn(),
  proposals: vi.fn(),
  sessions: vi.fn(),
  session: vi.fn(),
  agenda: vi.fn(),
  approvals: vi.fn(),
  awayDigest: vi.fn(),
  running: vi.fn(),
  monitor: vi.fn(),
  killSwitch: vi.fn(),
}))
const convo = vi.hoisted(() => ({
  list: vi.fn(),
  listProfiles: vi.fn(),
  create: vi.fn(),
  send: vi.fn(),
}))
vi.mock('@/api/desk', async (orig) => ({ ...(await orig<object>()), deskApi: api }))
vi.mock('@/api/agent', async (orig) => ({ ...(await orig<object>()), conversationsApi: convo }))
// The Start panel opens conversations on a provider the user has a key for
// (PROVIDERS track): here, an Anthropic key, the default.
const keys = vi.hoisted(() => ({ status: vi.fn(), models: vi.fn(() => Promise.resolve([])) }))
vi.mock('@/api/llm', async (orig) => ({ ...(await orig<object>()), llmKeysApi: keys }))

import type { AgendaItem, DeskSession, Proposal, SessionDetail, WireCharter } from '@/api/desk'
import { emptyCharter } from '@/api/desk'
import { agendaDepth, nextItem, sessionState, slashAction } from '@/lib/desk/sessions'
import { CharterForm, ScheduleGrid } from '../CharterForm'
import { Composer } from '../Composer'
import { DeskHome } from '../DeskHome'
import { SessionRunningRow, StalledSessionRow } from '../DeskHomeSessions'
import { ProposalCard } from '../ProposalsQueue'
import { AgendaTab, CharterTab, StateTab } from '../RightPanel'
import { PreregCard } from '../Cards'
import { CHARTER_REFUSED } from '../Composer.helpers'

function withAll(ui: ReactElement) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: 0 } } })
  return render(
    <QueryClientProvider client={qc}>
      <MemoryRouter>{ui}</MemoryRouter>
    </QueryClientProvider>,
  )
}

const listRow = (over: Partial<DeskSession> = {}): DeskSession => ({
  session_id: 's_1',
  conversation_id: 'c_1',
  continuous: true,
  question: null,
  tier: 'frontier',
  created_at: '2026-10-01T00:00:00Z',
  state: 'running',
  runs_started: 4,
  agenda_open: 3,
  ...over,
})

const wireCharter = (): WireCharter => ({
  scope: { instruments: ['BTC-USD'], asset_classes: [], families: [], timeframes: ['1h'], windows: [] },
  objective: { kind: 'build_knowledge' },
  seeds: [{ question: 'does the overnight gap fade?', subject: 'gap', workflow: 'falsify' }],
  permitted_workflows: [],
  exclusions: [],
  standing_constraints: [],
  exploration_share: 0.1,
  stop_when: [],
  delta_practical_default: null,
  version: 2,
})

const detail = (over: Partial<SessionDetail> = {}): SessionDetail => ({
  session_id: 's_1',
  conversation_id: 'c_1',
  continuous: true,
  question: null,
  charter: wireCharter(),
  budget: { usd_total: 0, usd_per_day: 25, trials_total: 0, max_runs: 0 },
  schedule: null,
  autonomy: 'notify',
  replenish: false,
  review_every: null,
  tier: 'frontier',
  exploration_share: 0.1,
  state: { kind: 'running' },
  runs_started: 4,
  charter_version: 2,
  ...over,
})

const item = (over: Partial<AgendaItem> = {}): AgendaItem => ({
  item_id: 'i_1',
  question: 'does momentum survive costs?',
  workflow: 'investigate',
  sources: ['charter_seed'],
  score: null,
  state: 'queued',
  ...over,
})

beforeEach(() => {
  for (const f of [...Object.values(api), ...Object.values(convo)]) f.mockReset()
})

// ── reading a session ─────────────────────────────────────────────────────────

describe('a session is read, not assumed', () => {
  it('reads the stalled diagnosis off the tagged state, and an unreadable state as unknown', () => {
    expect(sessionState({ kind: 'stalled', diagnosis: 'six barren runs in a row' })).toEqual({
      kind: 'stalled',
      diagnosis: 'six barren runs in a row',
    })
    expect(sessionState('paused')).toEqual({ kind: 'paused', why: null })
    // A log the fold could not replay is not a running session.
    expect(sessionState(null)).toEqual({ kind: 'unknown' })
  })

  it('names the next item only when the board can: yours first, then the scorer', () => {
    const scored = (id: string, total: number) =>
      item({ item_id: id, question: id, score: { value: 1, novelty: 1, feasibility: 1, cost: 1, total } })
    expect(nextItem([scored('a', 0.2), scored('b', 0.9)]).item?.item_id).toBe('b')
    expect(nextItem([scored('b', 0.9), item({ item_id: 'u', bypasses_scoring: true })]).item?.item_id).toBe('u')
    // Nothing scored: the board does not invent an order the scorer has not given.
    expect(nextItem([item(), item({ item_id: 'i_2' })])).toEqual({ item: null, why: 'unscored' })
    expect(nextItem(null)).toEqual({ item: null, why: 'unknown' })
    expect(agendaDepth([item(), item({ state: 'done' }), item({ state: 'running' })])).toBe(2)
  })
})

// ── Running now and Needs you ─────────────────────────────────────────────────

describe('desk home: a continuous session is one row in Running now', () => {
  it('shows state, next item, agenda depth and day budget', () => {
    withAll(
      <SessionRunningRow
        r={{
          session: listRow(),
          detail: detail(),
          agenda: [item({ bypasses_scoring: true, question: 'my question' }), item({ item_id: 'i_2' })],
        }}
      />,
    )
    const row = screen.getByTestId('session-row-s_1')
    expect(row.textContent).toMatch(/continuous/)
    expect(row.textContent).toMatch(/running/)
    expect(screen.getByTestId('session-next-s_1').textContent).toMatch(/my question/)
    expect(row.textContent).toMatch(/agenda depth 2/)
    // The budget is stated and the spend is not invented.
    expect(screen.getByTestId('day-budget-no-spend').textContent).toMatch(/\$25.00\/day/)
  })

  it('an agenda that did not answer is an unknown next item, not an empty one', () => {
    withAll(<SessionRunningRow r={{ session: listRow({ agenda_open: null }), detail: detail(), agenda: null }} />)
    expect(screen.getByTestId('session-next-s_1').textContent).toMatch(/unknown/)
    expect(screen.getByTestId('session-row-s_1').textContent).toMatch(/agenda depth \(unknown\)/)
  })
})

describe('desk home: a stalled session is in Needs you with resume · amend · stop', () => {
  const stalled = {
    session: listRow({ state: 'stalled' }),
    detail: detail({ state: { kind: 'stalled', diagnosis: 'six barren runs; narrow the families' } }),
    agenda: [],
  }

  it('shows the diagnosis and the three actions, and Amend lands on the charter', () => {
    withAll(<StalledSessionRow r={stalled} />)
    expect(screen.getByTestId('stalled-diagnosis-s_1').textContent).toMatch(/six barren runs/)
    expect(screen.getByRole('button', { name: 'Resume' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Stop' })).toBeInTheDocument()
    expect(screen.getByRole('link', { name: 'Amend' }).getAttribute('href')).toBe('/desk/c/c_1?tab=charter')
  })

  it('resume reaches the session route, and a refusal is printed in its words', async () => {
    api.sessionAction.mockRejectedValue({
      response: { status: 409, data: { error: 'transition_refused', message: 'cannot move from stopped to running' } },
    })
    withAll(<StalledSessionRow r={stalled} />)
    fireEvent.click(screen.getByRole('button', { name: 'Resume' }))
    expect(api.sessionAction).toHaveBeenCalledWith('s_1', 'resume')
    await waitFor(() =>
      expect(screen.getByTestId('stalled-error-s_1').textContent).toMatch(/cannot move from stopped to running/),
    )
  })
})

// ── the whole home page ───────────────────────────────────────────────────────

const proposal: Proposal = {
  proposal_id: 'p_1',
  kind: 'deploy_paper',
  ring: 2,
  envelope: 'Promotion',
  subject: 'gap strategy v3',
  subject_id: 'st_3',
  rationale: 'clears the funnel',
  dossier_ref: 'd_9',
  actionable: true,
}

function homeReads(over: { sessions?: DeskSession[]; detail?: SessionDetail; proposals?: Proposal[] } = {}) {
  convo.list.mockResolvedValue({ data: { conversations: [] } })
  convo.create.mockResolvedValue({ data: { conversation_id: 'c_open', provider: 'anthropic', model: 'claude-sonnet-5' } })
  keys.status.mockResolvedValue({
    providers: [
      {
        provider: 'anthropic', configured: true, key_last4: 'abcd', base_url: null, updated_at: null,
        verified_at: null, last_checked_at: null, last_error: null, default_model: null, is_default: true,
      },
    ],
    encryption_available: true,
    encryption_problem: null,
  })
  convo.listProfiles.mockResolvedValue({ data: { profiles: [] } })
  api.approvals.mockResolvedValue({ data: { approvals: [], kinds: [] } })
  api.awayDigest.mockResolvedValue({ absent: true, lines: [] })
  api.running.mockResolvedValue({ runs: [], not_recorded: [] })
  api.monitor.mockResolvedValue({ strategies: [] })
  api.killSwitch.mockResolvedValue({ trading_enabled: true, kill_switch_active: false })
  api.proposals.mockResolvedValue({ pending: over.proposals ?? [] })
  api.sessions.mockResolvedValue({ sessions: over.sessions ?? [] })
  api.session.mockResolvedValue(over.detail ?? detail())
  api.agenda.mockResolvedValue({ items: [] })
}

describe('desk home, assembled', () => {
  it('puts a pending proposal and a stalled session in Needs you, and a running session in Running now', async () => {
    homeReads({
      proposals: [proposal],
      sessions: [listRow(), listRow({ session_id: 's_2', conversation_id: 'c_2', state: 'stalled' })],
    })
    api.session.mockImplementation((id: string) =>
      Promise.resolve(
        id === 's_2'
          ? detail({ session_id: 's_2', conversation_id: 'c_2', state: { kind: 'stalled', diagnosis: 'no yield' } })
          : detail(),
      ),
    )
    withAll(<DeskHome />)
    await waitFor(() => expect(screen.getByTestId('needs-you-proposals')).toBeInTheDocument())
    expect(screen.getByTestId('proposal-p_1').textContent).toMatch(/Promotion/)
    expect(screen.getByTestId('proposal-p_1').textContent).toMatch(/d_9/)
    await waitFor(() => expect(screen.getByTestId('stalled-s_2')).toBeInTheDocument())
    expect(screen.getByTestId('session-row-s_1')).toBeInTheDocument()
    // The stalled one is not also a running row.
    expect(screen.queryByTestId('session-row-s_2')).toBeNull()
    expect(screen.queryByText('Nothing is waiting on you')).toBeNull()
  })

  it('does not claim nothing is waiting when the proposals queue could not be read', async () => {
    homeReads()
    api.proposals.mockRejectedValue(new Error('500'))
    withAll(<DeskHome />)
    await waitFor(() => expect(screen.getByTestId('needs-you-proposals-unknown')).toBeInTheDocument())
  })
})

// ── the Start panel ───────────────────────────────────────────────────────────

describe('the Start panel sends the mode as a field, not as prose', () => {
  it('a Research start goes through the session door with mode, workflow and question', async () => {
    homeReads()
    api.createSession.mockResolvedValue({ session_id: 's_9', conversation_id: 'c_9', continuous: false })
    withAll(<DeskHome />)
    const starter = await screen.findByRole('button', { name: 'Research an edge' })
    // Enabled once the user's providers are read (PROVIDERS track).
    await waitFor(() => expect(starter).toBeEnabled())
    fireEvent.click(starter)
    await waitFor(() => expect(api.createSession).toHaveBeenCalled())
    expect(api.createSession.mock.calls[0][0]).toMatchObject({
      mode: 'research',
      continuous: false,
      workflow: 'falsify',
      question: expect.stringMatching(/overnight gap/),
      // Opened on the user's own provider first, not the server's default.
      conversation_id: 'c_open',
    })
    expect(convo.create).toHaveBeenCalledWith({ provider: 'anthropic', model: 'claude-sonnet-5' })
    // Not the text door with the mode written into the message.
    expect(convo.send).not.toHaveBeenCalled()
  })

  it('a refusal is shown rather than dropped', async () => {
    homeReads()
    api.createSession.mockRejectedValue({
      response: { status: 400, data: { error: 'refused', message: '`local-8b` is a local profile', fix: 'run it bounded' } },
    })
    withAll(<DeskHome />)
    const starter = await screen.findByRole('button', { name: 'Research an edge' })
    // Enabled once the user's providers are read (PROVIDERS track).
    await waitFor(() => expect(starter).toBeEnabled())
    fireEvent.click(starter)
    await waitFor(() => expect(screen.getByTestId('start-error').textContent).toMatch(/local profile/))
  })

  it('a Chat start sends the question alone, with no mode prefix in the text', async () => {
    homeReads()
    convo.create.mockResolvedValue({ data: { conversation_id: 'c_7' } })
    convo.send.mockResolvedValue({ data: {} })
    withAll(<DeskHome />)
    const starter = await screen.findByRole('button', { name: 'Ask a question' })
    // Enabled once the user's providers are read (PROVIDERS track).
    await waitFor(() => expect(starter).toBeEnabled())
    fireEvent.click(starter)
    await waitFor(() => expect(convo.send).toHaveBeenCalled())
    expect(convo.send.mock.calls[0][1]).not.toMatch(/mode=/)
    expect(convo.create).toHaveBeenCalledWith({ provider: 'anthropic', model: 'claude-sonnet-5' })
  })
})

// ── the composer and the charter form ─────────────────────────────────────────

describe('the charter form', () => {
  it('toggling Continuous swaps the question for the charter and back, losing nothing typed', () => {
    render(
      <Composer running={false} profiles={[{ id: 'f', label: 'F', tier: 'frontier', measured: true, frontier: true }]} model="f" onModelChange={() => {}} onSubmit={() => {}} />,
    )
    fireEvent.click(screen.getByRole('radio', { name: 'Research' }))
    fireEvent.change(screen.getByLabelText('instrument'), { target: { value: 'BTC-USD' } })
    fireEvent.change(screen.getByLabelText('Ask, or describe what to research'), { target: { value: 'kept question' } })
    fireEvent.click(screen.getByRole('switch', { name: 'Continuous' }))
    expect(screen.getByTestId('charter-form')).toBeInTheDocument()
    fireEvent.change(screen.getByLabelText('Seeds'), { target: { value: 'a seed' } })
    fireEvent.click(screen.getByRole('switch', { name: 'Continuous' }))
    expect((screen.getByLabelText('Ask, or describe what to research') as HTMLTextAreaElement).value).toBe('kept question')
    expect((screen.getByLabelText('instrument') as HTMLInputElement).value).toBe('BTC-USD')
    fireEvent.click(screen.getByRole('switch', { name: 'Continuous' }))
    expect((screen.getByLabelText('Seeds') as HTMLInputElement).value).toBe('a seed')
  })

  it('an empty schedule is any time, and the first click makes the full week', () => {
    const onChange = vi.fn()
    render(<ScheduleGrid value={[]} onChange={onChange} />)
    expect(screen.getByTestId('schedule-reading').textContent).toMatch(/any time/)
    fireEvent.click(screen.getByRole('gridcell', { name: 'Mon 09:00' }))
    const grid = onChange.mock.calls[0][0] as boolean[][]
    expect(grid).toHaveLength(7)
    expect(grid.every((d) => d.length === 24)).toBe(true)
    expect(grid[0][9]).toBe(true)
    expect(grid.flat().filter(Boolean)).toHaveLength(1)
  })

  it('does not offer what the server refuses: replenish is disabled with the reason', () => {
    render(
      <CharterForm
        value={emptyCharter()}
        onChange={() => {}}
        replenishDisabledReason={CHARTER_REFUSED.replenish}
        autonomousDisabledReason={CHARTER_REFUSED.autonomous}
      />,
    )
    expect(screen.getByRole('switch', { name: 'Keep generating questions' })).toBeDisabled()
    expect(screen.getByTestId('replenish-hint').textContent).toMatch(/S-11.11/)
  })
})

// ── the board and the charter tab ─────────────────────────────────────────────

describe('the agenda board', () => {
  it('Add a question reaches the route and prints a refusal', async () => {
    const onAdd = vi.fn().mockRejectedValueOnce({
      response: { status: 400, data: { message: '`sweep` is not a workflow' } },
    })
    render(<AgendaTab items={[]} onAdd={onAdd} />)
    fireEvent.click(screen.getByRole('button', { name: 'Add a question' }))
    fireEvent.change(screen.getByLabelText('Question to add'), { target: { value: 'is ETH different?' } })
    fireEvent.click(screen.getByRole('button', { name: 'Add' }))
    expect(onAdd).toHaveBeenCalledWith('is ETH different?')
    await waitFor(() => expect(screen.getByTestId('agenda-add-error').textContent).toMatch(/not a workflow/))
  })

  it('draws no Add button when nothing can add', () => {
    render(<AgendaTab items={[]} />)
    expect(screen.queryByRole('button', { name: 'Add a question' })).toBeNull()
  })

  it("a running card shows its child's stage, and says so when it is not reported", () => {
    render(
      <AgendaTab
        items={[
          item({ item_id: 'r1', state: 'running', child_conversation_id: 'cc_1' }),
          item({ item_id: 'r2', state: 'running', child_conversation_id: 'cc_2' }),
        ]}
        childStages={{ cc_1: 'design' }}
      />,
    )
    expect(screen.getByLabelText('lifecycle stage: design')).toBeInTheDocument()
    expect(screen.getByTestId('stage-unknown-r2')).toBeInTheDocument()
  })
})

describe('the charter tab amends', () => {
  it('sends the stored charter back with only the amended field changed', async () => {
    const onAmend = vi.fn().mockResolvedValue({})
    const raw = wireCharter()
    render(
      <CharterTab
        charter={{
          objective: 'build knowledge',
          scope: raw.scope,
          scope_is_any: false,
          permitted_workflows: [],
          workflows_are_all: true,
          exclusions: [],
          standing_constraints: [],
          seeds: ['does the overnight gap fade?'],
          version: 2,
          usd_per_day: 25,
          usd_total: null,
          max_runs: null,
          trials_total: null,
          autonomy: 'notify',
          max_concurrent_runs: null,
          exploration_share: 0.1,
          replenish: false,
          review_every: null,
          always_open: true,
          windows: [],
          delta_practical: null,
        } as never}
        amendments={[]}
        raw={raw}
        onAmend={onAmend}
      />,
    )
    fireEvent.click(screen.getByRole('button', { name: 'Amend' }))
    fireEvent.change(screen.getByLabelText('Exclusions'), { target: { value: 'meme coins' } })
    fireEvent.click(screen.getByRole('button', { name: 'Save amendment' }))
    await waitFor(() => expect(onAmend).toHaveBeenCalled())
    const sent = onAmend.mock.calls[0][0] as WireCharter
    expect(sent.exclusions).toEqual(['meme coins'])
    // Everything else as read, so an amendment cannot reset the objective.
    expect(sent.objective).toEqual({ kind: 'build_knowledge' })
    expect(sent.exploration_share).toBe(0.1)
    expect(sent.seeds[0]).toEqual(raw.seeds[0])
  })
})

// ── the journal's commands ────────────────────────────────────────────────────

describe('the journal answers its commands', () => {
  const journal = { continuous: true, hasRun: true }
  it('maps each to the session, never to a steer', () => {
    expect(slashAction('/pause', '', journal)).toEqual({ kind: 'session', action: 'pause' })
    expect(slashAction('/resume', '', journal)).toEqual({ kind: 'session', action: 'resume' })
    expect(slashAction('/agenda', 'add is ETH different?', journal)).toEqual({
      kind: 'agenda_add',
      question: 'is ETH different?',
    })
    expect(slashAction('/charter', '', journal)).toEqual({ kind: 'tab', tab: 'charter' })
  })
  it('in an ordinary conversation, pause is the run and the session commands refuse', () => {
    const chat = { continuous: false, hasRun: true }
    expect(slashAction('/pause', '', chat)).toEqual({ kind: 'run', action: 'pause' })
    expect(slashAction('/agenda', 'add x', chat)?.kind).toBe('refuse')
    expect(slashAction('/charter', '', chat)?.kind).toBe('refuse')
  })
})

// ── proposals from the keyboard ───────────────────────────────────────────────

describe('a proposal decides from the keyboard', () => {
  it('a approves and x rejects the focused card, and typing a reason is not a decision', async () => {
    api.decideProposal.mockResolvedValue({ proposal_id: 'p_1', decision: 'approved', decided_by: 'm', performed: true, effect: {} })
    withAll(<ProposalCard p={proposal} onDecided={() => {}} />)
    const card = screen.getByTestId('proposal-p_1')
    fireEvent.keyDown(screen.getByPlaceholderText('on what basis'), { key: 'a' })
    expect(api.decideProposal).not.toHaveBeenCalled()
    fireEvent.keyDown(card, { key: 'a' })
    await waitFor(() => expect(api.decideProposal).toHaveBeenCalledWith('p_1', 'approved', ''))
    api.decideProposal.mockClear()
    fireEvent.keyDown(card, { key: 'x' })
    await waitFor(() => expect(api.decideProposal).toHaveBeenCalledWith('p_1', 'rejected', ''))
  })
})

// ── S-10.15: the lifecycle reaches the screen without the working state ──────

describe('a running Research turn says what it is from its run record', () => {
  it('the State tab shows mode, workflow, ring and stage when the snapshot is not served', () => {
    render(
      <StateTab
        state={null}
        absence="not_served"
        recorded={{ mode: 'research', workflow: 'falsify', ring: 'platform_artifacts', stage: 'design' }}
      />,
    )
    const r = screen.getByTestId('recorded-lifecycle')
    expect(r.textContent).toMatch(/mode: research/)
    expect(r.textContent).toMatch(/falsify/)
    expect(r.textContent).toMatch(/ring: platform_artifacts/)
    expect(screen.getByLabelText('lifecycle stage: design')).toBeInTheDocument()
    // Labelled as the record, not as the agent's own state.
    expect(r.textContent).toMatch(/from the run record/)
  })

  it('a ring the server did not send reads "not reported", never ring 0', () => {
    render(<StateTab state={null} recorded={{ mode: 'research', workflow: null, ring: null, stage: 'explore' }} />)
    expect(screen.getByTestId('recorded-lifecycle').textContent).toMatch(/ring: not reported/)
  })
})

describe('the pre-registration card: steer opens an amendment', () => {
  const card = {
    seq: 9,
    step: 2,
    kind: 'prereg' as const,
    at: '2026-10-03T10:00:00Z',
    headline: 'gap fades within 2h',
    detail: { hash: 'ph_ab12cd', falsifier: 'IC < 0.01 over the window', trial_budget: 40 },
  }

  it('shows the lineage warning before anything is sent, and steers with the superseded hash', () => {
    const onSteer = vi.fn()
    render(<PreregCard card={card} onSteer={onSteer} />)
    expect(screen.getByText('Sealed before any trial was spent.')).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: 'Steer: amend' }))
    expect(screen.getByTestId('prereg-lineage-warning').textContent).toMatch(/ph_ab12cd/)
    expect(onSteer).not.toHaveBeenCalled()
    fireEvent.change(screen.getByLabelText('What to amend'), { target: { value: 'widen the window to 4h' } })
    fireEvent.click(screen.getByRole('button', { name: 'Send amendment request' }))
    expect(onSteer).toHaveBeenCalledWith('Amend the pre-registration (supersedes ph_ab12cd): widen the window to 4h')
  })

  it('draws no amend control when nothing can steer', () => {
    render(<PreregCard card={card} />)
    expect(screen.queryByRole('button', { name: 'Steer: amend' })).toBeNull()
  })
})
