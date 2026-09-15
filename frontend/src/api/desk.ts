// The Research Desk's data layer (AGENT-012 §16, §17).
//
// One surface replaces `/agent` (polling) and `/research` (SSE), so this module
// replaces two clients with one. It is deliberately honest about what the backend
// can answer today: §17 of the spec lists a dozen endpoints that do not exist yet
// and belong to other Set S phases. Rather than pretend, every optional read goes
// through `optional()`, which turns a 404 into `null` so the surface can render
// "not built yet" instead of an error toast that says nothing.
import { api as client } from '@/lib/api'

// ── The lifecycle vocabulary (mirrors crates/harness/src/state.rs) ────────────
//
// These are the serde `snake_case` spellings of the Rust enums, so a snapshot
// deserialises straight into the panel. Keep them in step with `state.rs`; the
// desk renders the state the harness owns, it does not define it.

export type GoalKind = 'lookup' | 'research' | 'diagnose' | 'compare'

/** Two modes, and only two (ADR-0037 §2). The toggle below is not a third. */
export type Mode = 'chat' | 'research'

export type Workflow =
  | 'investigate'
  | 'falsify'
  | 'replicate'
  | 'compare'
  | 'diagnose'
  | 'survey'
  | 'optimise'

/** How far from capital a run may reach (AGENT-013 §4). */
export type Ring = 'research' | 'platform_artifacts' | 'paper'

export type Stage =
  | 'explore'
  | 'hypothesise'
  | 'preregister'
  | 'design'
  | 'run'
  | 'refine'
  | 'compare'
  | 'reproduce'
  | 'propose'
  | 'closed'

export const STAGE_ORDER: Stage[] = [
  'explore',
  'hypothesise',
  'preregister',
  'design',
  'run',
  'refine',
  'compare',
  'reproduce',
  'propose',
  'closed',
]

export const WORKFLOWS: Workflow[] = [
  'investigate',
  'falsify',
  'replicate',
  'compare',
  'diagnose',
  'survey',
  'optimise',
]

export type StepStatusKind = 'pending' | 'active' | 'done' | 'skipped' | 'failed'

export interface StepStatus {
  status: StepStatusKind
  /** Present on `skipped` and `failed`; the harness tags it `why`. */
  why?: string | null
}

export interface StepKind {
  kind: 'tool' | 'workflow' | 'reflect' | 'ask_user'
  name?: string
}

export interface PlannedStep {
  goal: string
  kind: StepKind
  status: StepStatus
}

export interface EvidenceRef {
  id: string
  tool: string
  step: number
}

export interface FindingRef {
  id: string
  headline: string
  step: number
}

export interface Correction {
  code: string
  message: string
  fix: string
  step: number
}

export interface Decision {
  question: string
  choice: string
  reason: string
  step: number
}

export type HypothesisStatus = 'open' | 'supported' | 'refuted' | 'inconclusive'

export interface Hypothesis {
  family_id: string
  statement: string
  falsifier: string
  role: string
  status: HypothesisStatus
}

export interface PreregRef {
  hash: string
  sealed_at?: string | null
  [k: string]: unknown
}

export interface BudgetView {
  steps_remaining: number
  steps_total: number
  tokens_remaining: number
  /** Tenths of a cent. Money is integer arithmetic here too (SC-1). */
  usd_tenth_cents_remaining: number
  trials_spent: number
  n_eff: number | null
}

/** AGENT-008 §2, as the harness serialises it. */
export interface WorkingState {
  goal: string
  kind: GoalKind
  mode: Mode
  workflow: Workflow
  ring: Ring
  stage: Stage
  prereg: PreregRef | null
  acceptance: string[]
  hypothesis: Hypothesis | null
  plan: PlannedStep[]
  evidence: EvidenceRef[]
  findings: FindingRef[]
  open_questions: string[]
  last_error: Correction | null
  budget: BudgetView
  decided: Decision[]
}

/** The narrowest reading of an absent field, matching `state.rs`'s defaults. */
export function emptyWorkingState(goal = ''): WorkingState {
  return {
    goal,
    kind: 'lookup',
    mode: 'chat',
    workflow: 'investigate',
    ring: 'research',
    stage: 'explore',
    prereg: null,
    acceptance: [],
    hypothesis: null,
    plan: [],
    evidence: [],
    findings: [],
    open_questions: [],
    last_error: null,
    budget: {
      steps_remaining: 0,
      steps_total: 0,
      tokens_remaining: 0,
      usd_tenth_cents_remaining: 0,
      trials_spent: 0,
      n_eff: null,
    },
    decided: [],
  }
}

// ── Provenance: the fence has to reach the human (AGENT-012 §6.3a) ───────────

/** Where a piece of content came from. `untrusted` is anything fetched. */
export type Provenance = 'platform' | 'user' | 'untrusted'

/** What kind of thing a number is (AGENT-007 §2.2). */
export type EvidenceClass = 'direct_stat' | 'derived' | 'model_summary' | 'human'

export interface ProvenanceMark {
  provenance: Provenance
  /** Host the content was fetched from, when untrusted. */
  source?: string | null
  fetched_at?: string | null
  fetched_by_run?: string | null
  evidence_class?: EvidenceClass | null
}

// ── Cards (AGENT-012 §3.2) ───────────────────────────────────────────────────

export type CardKind =
  | 'you'
  /**
   * The agent's own prose between tool calls. AGENT-012 §3.2's table does not
   * list it, because the spec was written against a harness whose every step is
   * a card; today's `agent_messages` still carry free assistant text and hiding
   * it would be the transcript-shaped lie the desk is supposed to remove. A
   * deliberate deviation, recorded here rather than silently.
   */
  | 'agent'
  | 'plan'
  | 'tool'
  | 'finding'
  | 'negative'
  | 'decision'
  | 'question'
  | 'approval'
  | 'job'
  | 'gate'
  | 'escalation'
  | 'correction'
  | 'compaction'
  | 'report'
  | 'stage'
  | 'prereg'
  | 'offer'
  | 'proposal'
  | 'halt'
  | 'steer'
  | 'raw'

export type RiskClass = 'read' | 'write' | 'destructive' | 'outbound'
export type Confidence = 'established' | 'suggestive' | 'refuted' | 'stale'

export interface DeskCard {
  /** Monotonic within a conversation; the stream cursor. */
  seq: number
  step: number
  kind: CardKind
  at: string
  /** The one-line thing a human reads first. Never a raw table. */
  headline: string
  body?: string | null
  /** Ids this card asserts. Each renders as a chip that resolves or is struck. */
  ids?: string[]
  mark?: ProvenanceMark | null
  /** Card-shaped extras, narrowed by `kind` at the render site. */
  detail?: Record<string, unknown> | null
  /** `true` for the raw events folded behind the details disclosure. */
  raw?: boolean
}

export interface StepGroup {
  step: number
  label: string
  cards: DeskCard[]
  raw: DeskCard[]
  tools: number
  seconds: number | null
}

/** Groups a flat card list into the stream's step headers (§6.6). */
export function groupByStep(cards: DeskCard[]): StepGroup[] {
  const groups = new Map<number, StepGroup>()
  for (const c of cards) {
    let g = groups.get(c.step)
    if (!g) {
      g = { step: c.step, label: '', cards: [], raw: [], tools: 0, seconds: null }
      groups.set(c.step, g)
    }
    if (c.raw) g.raw.push(c)
    else g.cards.push(c)
    if (c.kind === 'tool') g.tools += 1
    if (!g.label && (c.kind === 'plan' || c.kind === 'tool')) g.label = c.headline
  }
  const out = [...groups.values()].sort((a, b) => a.step - b.step)
  for (const g of out) {
    if (!g.label) g.label = g.cards[0]?.headline ?? 'step'
    const all = [...g.cards, ...g.raw]
    if (all.length > 1) {
      const first = Date.parse(all[0].at)
      const last = Date.parse(all[all.length - 1].at)
      g.seconds = Number.isFinite(first) && Number.isFinite(last) ? (last - first) / 1000 : null
    }
  }
  return out
}

// ── The catalogue, as the desk needs it (AGENT-006, track 23 owns the source) ─

export interface ToolCard {
  name: string
  namespace: string
  risk: RiskClass
  /** Whether calling it spends a trial. A Chat turn may never see one. */
  spends_trial: boolean
  /** The lowest ring this tool reaches. */
  ring: Ring
  output_budget_tokens?: number | null
  notes?: string | null
  tiers?: string[]
}

// ── Sessions: a Research run is the one-item case of a continuous one ─────────

export type Autonomy = 'supervised' | 'notify' | 'autonomous'
export type Objective = 'find_deployable' | 'build_knowledge' | 'maintain' | 'mixed'

/**
 * The charter the Continuous toggle asks for (AGENT-014 §2.1).
 *
 * It is a field of the session, not a different object: `continuous: false` means
 * the charter is unused and the question field is shown instead. Toggling does
 * not discard it, which is why it lives here and not in the toggle's own state.
 */
export interface Charter {
  instruments: string[]
  asset_classes: string[]
  families: string[]
  timeframes: string[]
  objective: Objective
  seeds: string[]
  workflows: Workflow[]
  exclusions: string[]
  standing_constraints: string[]
  budget_usd_day: number
  budget_usd_total: number | null
  max_trials: number | null
  max_runs: number | null
  /** days × hours grid, 7 × 24 booleans; empty means "any time". */
  schedule: boolean[][]
  timezone: string
  concurrency: number
  autonomy: Autonomy
  /** "Keep generating questions" — AGENT-014's `replenish`. */
  replenish: boolean
  review_every: number | null
  delta_practical: string
}

/** The platform default day budget (ADR-0037 §8). Never required of the user. */
export const DEFAULT_DAY_BUDGET_USD = 25

export function emptyCharter(): Charter {
  return {
    instruments: [],
    asset_classes: [],
    families: [],
    timeframes: [],
    objective: 'find_deployable',
    seeds: [],
    workflows: [...WORKFLOWS],
    exclusions: [],
    standing_constraints: [],
    budget_usd_day: DEFAULT_DAY_BUDGET_USD,
    budget_usd_total: null,
    max_trials: null,
    max_runs: null,
    schedule: [],
    timezone: Intl.DateTimeFormat().resolvedOptions().timeZone || 'UTC',
    concurrency: 1,
    autonomy: 'notify',
    replenish: false,
    review_every: null,
    delta_practical: '',
  }
}

export type AgendaState = 'proposed' | 'queued' | 'running' | 'done' | 'vetoed'

export interface AgendaItem {
  item_id: string
  question: string
  workflow: Workflow
  sources: string[]
  score: { value: number; novelty: number; feasibility: number; cost: number; total: number }
  rationale: string
  exploration_pick: boolean
  state: AgendaState
  child_conversation_id?: string | null
  stage?: Stage | null
}

export interface DeskSession {
  session_id: string
  conversation_id: string
  title: string
  continuous: boolean
  mode: Mode
  workflow: Workflow
  ring: Ring
  stage: Stage
  charter: Charter | null
  /** Set when the stall detector fired; the three actions hang off it. */
  stalled_reason?: string | null
  agenda_depth?: number
  spent_usd_today?: number
}

// ── Desk home ────────────────────────────────────────────────────────────────

export interface NeedsYouRow {
  id: string
  kind: 'approval' | 'question' | 'escalation' | 'proposal' | 'stalled'
  what: string
  conversation_id: string | null
  waiting_since: string
  /** The two obvious actions, inline. */
  actions: Array<{ id: string; label: string; danger?: boolean }>
}

export interface RunningRow {
  run_id: string
  conversation_id: string
  title: string
  tier: string
  measured: boolean
  step: string
  waiting_on: string
  tokens: number
  usd_tenth_cents: number
  cache_hit_pct: number | null
  continuous: boolean
  next_item?: string | null
  agenda_depth?: number | null
  day_budget_used_pct?: number | null
}

/** One line of "Since you were away" — a verdict or a state change, never an event name. */
export interface DigestLine {
  id: string
  at: string
  kind: 'concluded' | 'finding' | 'refuted' | 'approval_defaulted' | 'monitor' | 'budget' | 'waiting'
  text: string
  href: string | null
  mark?: ProvenanceMark | null
}

export interface AwayDigest {
  since: string
  until: string
  lines: DigestLine[]
}

// ── Replay (AGENT-012 §4) ────────────────────────────────────────────────────

export type PromptLayer = 'charter' | 'brief' | 'working_state' | 'schemas' | 'body' | 'output'

export interface PromptSection {
  layer: PromptLayer
  /** Exactly the text the model saw. Never re-wrapped, never re-indented. */
  text: string
  tokens: number
}

export interface TrajectoryStep {
  step: number
  at: string
  sections: PromptSection[]
  /**
   * The prompt as the harness sent it. Kept beside the sections so the scrubber
   * can prove it reassembles them byte-for-byte rather than approximately.
   */
  prompt: string
  reply: string
  state: WorkingState | null
}

/**
 * Reassembles the prompt from its layers, with no separator, no trimming and no
 * normalisation. AGENT-012 §19.4 asks for byte-for-byte; the only way to be sure
 * is for the renderer to do nothing at all to the stored text.
 */
export function assemblePrompt(sections: PromptSection[]): string {
  return sections.map((s) => s.text).join('')
}

/** True when the reassembly is byte-identical to what was sent. */
export function promptIsFaithful(step: TrajectoryStep): boolean {
  return assemblePrompt(step.sections) === step.prompt
}

// ── Client ───────────────────────────────────────────────────────────────────

/**
 * Reads an endpoint the spec marks ✗ (not built). A 404/501 becomes `null` so the
 * panel can say "this phase has not landed" instead of showing an error that
 * blames the network.
 */
async function optional<T>(p: Promise<{ data: T }>): Promise<T | null> {
  try {
    return (await p).data
  } catch (e) {
    const status = (e as { response?: { status?: number } }).response?.status
    if (status === 404 || status === 501 || status === 405) return null
    throw e
  }
}

export interface KillSwitchState {
  tripped: boolean
  scope: string | null
  tripped_by: string | null
  reason: string | null
  at: string | null
}

export const deskApi = {
  /** Existing: the conversation list the desk rail renders. */
  conversations: () =>
    client.get<{ conversations: Array<Record<string, unknown>> }>('/api/agent/conversations'),

  /** Existing: `/api/agent/runs/{id}/messages`, the fallback when SSE is closed. */
  messages: (runId: string, afterSeq?: number) =>
    client.get<{ messages: Array<{ seq: number; kind: string; content: Record<string, unknown>; created_at: string }>; last_seq: number }>(
      `/api/agent/runs/${runId}/messages`,
      { params: { after_seq: afterSeq } },
    ),

  approvals: (params: { state?: string } = {}) =>
    client.get<{ approvals: Array<Record<string, unknown>>; kinds: string[] }>('/api/approvals', { params }),

  answerApproval: (id: string, option: string, note?: string) =>
    client.post(`/api/approvals/${id}/answer`, { option, note }),

  usage: (params: { session_id?: string } = {}) =>
    client.get<Record<string, number>>('/api/agent/usage', { params }),

  killSwitch: () => optional<KillSwitchState>(client.get('/api/trading/status')),

  // — AGENT-012 §17's ✗ rows. Present so the desk is wired the day they land. —
  state: (runId: string, step?: number) =>
    optional<WorkingState>(client.get(`/api/agent/runs/${runId}/state`, { params: { step } })),
  trajectory: (runId: string) =>
    optional<{ steps: TrajectoryStep[] }>(client.get(`/api/agent/runs/${runId}/trajectory`)),
  lifecycle: (runId: string) =>
    optional<{ stage: Stage; blocking: string | null }>(client.get(`/api/agent/runs/${runId}/lifecycle`)),
  catalogue: () => optional<{ tools: ToolCard[] }>(client.get('/api/catalogue')),
  exposure: (profile: string, goal: string) =>
    optional<{ tools: ToolCard[]; reasons: Record<string, string> }>(
      client.get('/api/catalogue/exposure', { params: { profile, goal } }),
    ),
  memory: (bucket: string) =>
    optional<{ rows: Array<Record<string, unknown>> }>(client.get(`/api/memory/${bucket}`)),
  forget: (bucket: string, id: string) => client.delete(`/api/memory/${bucket}/${id}`),
  dossier: (ticket: string) => optional<Record<string, unknown>>(client.get(`/api/dossiers/${ticket}`)),
  verifyClaim: (ticket: string, claim: string) =>
    optional<{ ok: boolean; note: string }>(client.post(`/api/dossiers/${ticket}/verify/${claim}`)),
  monitor: () => optional<{ strategies: Array<Record<string, unknown>> }>(client.get('/api/monitor/strategies')),
  awayDigest: (since: string) => optional<AwayDigest>(client.get('/api/desk/digest', { params: { since } })),
  sessions: () => optional<{ sessions: DeskSession[] }>(client.get('/api/research/sessions')),
  createSession: (body: { mode: Mode; continuous: boolean; question?: string; charter?: Charter; workflow?: Workflow }) =>
    optional<{ session_id: string; conversation_id: string }>(client.post('/api/research/sessions', body)),
  agenda: (sessionId: string) =>
    optional<{ items: AgendaItem[] }>(client.get(`/api/research/sessions/${sessionId}/agenda`)),
  vetoAgendaItem: (sessionId: string, itemId: string) =>
    optional<{ ok: boolean }>(client.post(`/api/research/sessions/${sessionId}/agenda/${itemId}/veto`)),
  reorderAgenda: (sessionId: string, order: string[]) =>
    optional<{ ok: boolean }>(client.post(`/api/research/sessions/${sessionId}/agenda/order`, { order })),

  // — Controls. Pause/resume/escalate are ✗; steer and cancel exist. —
  steer: (sessionId: string, body: { kind: 'steer' | 'interrupt' | 'stop' | 'answer'; text?: string }) =>
    client.post(`/api/agent/sessions/${sessionId}/steer`, body),
  pause: (runId: string) => optional<{ ok: boolean }>(client.post(`/api/agent/runs/${runId}/pause`)),
  resume: (runId: string) => optional<{ ok: boolean }>(client.post(`/api/agent/runs/${runId}/resume`)),
  escalate: (runId: string, tier: string) =>
    optional<{ ok: boolean }>(client.post(`/api/agent/runs/${runId}/escalate`, { tier })),
  cancel: (conversationId: string) => client.post(`/api/agent/conversations/${conversationId}/cancel`),
}
