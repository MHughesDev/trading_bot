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
  | 'selection'
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

/** The board's five columns (AGENT-014 §9). */
export type AgendaState = 'proposed' | 'queued' | 'running' | 'done' | 'vetoed'

/**
 * An item's true status — nine of them, against the board's five columns.
 *
 * The card shows this on its badge and sits in the column `state` names. They
 * are not the same thing: `expired` and a verdict both land in `done`, and a
 * reader needs to be able to tell them apart.
 */
export type AgendaStatus =
  | 'proposed'
  | 'approved'
  | 'queued'
  | 'claimed'
  | 'running'
  | 'done'
  | 'vetoed'
  | 'superseded'
  | 'expired'

export interface AgendaItem {
  item_id: string
  question: string
  subject?: string
  workflow: Workflow
  sources: string[]
  /** Null until the scorer runs (Set S task S-11.3). */
  score: { value: number; novelty: number; feasibility: number; cost: number; total: number } | null
  rationale?: string
  exploration_pick?: boolean
  /** The column. */
  state: AgendaState
  /** The word on the badge. */
  status?: AgendaStatus
  /** True for a `user` item, which is claimed next whatever the score says. */
  bypasses_scoring?: boolean
  attempts?: number
  child_conversation_id?: string | null
  stage?: Stage | null
}

/**
 * A row of `GET /api/research/sessions`, as that route actually answers it.
 *
 * This type used to declare `title`, `mode`, `workflow`, `ring`, `stage` and a
 * `charter`, none of which the list has ever sent. Nothing read the type, so
 * nothing noticed — but the first surface to trust it would have rendered six
 * `undefined`s with no error anywhere, which is the failure that is hardest to
 * see. What the list sends is below; the charter is on the *detail* route, which
 * is why `sessionDetail` exists separately.
 */
export interface DeskSession {
  session_id: string
  conversation_id: string
  continuous: boolean
  /** A bounded session's one question. `null` for a continuous one. */
  question: string | null
  tier: string
  created_at: string
  /** The folded lifecycle word, or `null` when the log did not replay. */
  state: string | null
  runs_started: number | null
  /** Items neither finished nor vetoed. `null` when the fold failed. */
  agenda_open: number | null
}

/**
 * The session a conversation *is*, or `null` when it is an ordinary chat.
 *
 * A pure function over the list rather than a lookup route, because there is no
 * `GET /api/research/sessions?conversation_id=` and inventing one in the client
 * would be a call to a path that does not exist.
 */
export function sessionForConversation(
  sessions: DeskSession[] | null | undefined,
  conversationId: string,
): DeskSession | null {
  return (sessions ?? []).find((s) => s.conversation_id === conversationId) ?? null
}

// ── The session detail, which is a different shape from the charter form ─────
//
// `POST /api/research/sessions` takes the form's flat shape and `desk_charter::
// translate` converts it. `GET /api/research/sessions/{id}` answers the *stored*
// charter, which is the one AGENT-014 §2.2 defines: a nested `scope`, a tagged
// `objective`, and the budget, autonomy and schedule as siblings of the charter
// rather than fields of it. `Charter` above is the form. These are the wire.

export interface WireScope {
  instruments: string[]
  asset_classes: string[]
  families: string[]
  timeframes: string[]
  windows: string[]
}

/** `#[serde(tag = "kind")]` on the Rust side; the payload varies by tag. */
export type WireObjective = { kind: string } & Record<string, unknown>

export interface WireCharter {
  scope: WireScope
  objective: WireObjective | string
  seeds: Array<{ question: string; subject: string; workflow: string | null }>
  /** Empty means *all the tier supports*, not none (AGENT-014 §2.2). */
  permitted_workflows: string[]
  exclusions: string[]
  standing_constraints: string[]
  exploration_share: number
  stop_when: unknown[]
  delta_practical_default: number | null
  version: number
}

export interface WireWindow {
  /** ISO weekday numbers, Monday = 1. */
  days: number[]
  start: string
  end: string
  tz: string
}

export interface SessionDetail {
  session_id: string
  conversation_id: string
  continuous: boolean
  question: string | null
  charter: WireCharter | null
  budget: {
    usd_total: number
    usd_per_day: number
    trials_total: number
    max_runs: number
  } | null
  schedule: { windows: WireWindow[]; max_concurrent_runs: number } | null
  autonomy: string | null
  replenish: boolean | null
  review_every: { kind: 'runs'; runs: number } | { kind: 'window' } | null
  tier: string | null
  exploration_share: number | null
  state: unknown
  runs_started: number | null
  charter_version: number | null
  /** Events the fold could not replay. Reported rather than hidden. */
  anomalies?: unknown[]
}

/**
 * What the Charter tab renders, decoded from the wire with the empties read the
 * way the server reads them.
 *
 * Three of these decisions are the whole reason this is a function with a test
 * rather than a spread:
 *
 * * **An empty scope list is *everything*, not nothing.** `Scope::admits`
 *   returns `true` when every list is empty, so a charter that names no
 *   instruments has not excluded them all. The tab used to print `—` for an
 *   empty list, which reads as "nothing is in scope" — the opposite of what the
 *   session will do.
 * * **An empty `permitted_workflows` is *all of them*, for the same reason.**
 * * **`trials_total: 0` is "nobody said", not "no trials".** `translate` writes
 *   `form.max_trials.unwrap_or(0)` for a blank field, so the zero on the wire is
 *   an absent cap. Rendering "0 trials" would describe a session that may not
 *   run at all.
 */
export interface CharterView {
  objective: string
  scope: { instruments: string[]; asset_classes: string[]; families: string[]; timeframes: string[] }
  /** True when every scope list is empty, which the server reads as "any". */
  scope_is_any: boolean
  seeds: string[]
  permitted_workflows: string[]
  /** True when the list is empty, which the server reads as "all of them". */
  workflows_are_all: boolean
  exclusions: string[]
  standing_constraints: string[]
  exploration_share: number | null
  delta_practical: number | null
  version: number | null
  usd_per_day: number | null
  usd_total: number | null
  /** `null` when no cap was recorded — never `0`. */
  trials_total: number | null
  max_runs: number | null
  autonomy: string | null
  replenish: boolean | null
  review_every: { kind: 'runs'; runs: number } | { kind: 'window' } | null
  windows: WireWindow[]
  /** True when no window was set, which `Schedule::default` reads as always open. */
  always_open: boolean
  max_concurrent_runs: number | null
}

function objectiveWord(o: WireCharter['objective']): string {
  if (typeof o === 'string') return o
  if (o && typeof o === 'object' && typeof o.kind === 'string') return o.kind
  // The tag is how the server names it; without one the desk does not guess at
  // `find_deployable`, which is the reassuring answer rather than the true one.
  return 'not recorded'
}

export function charterView(s: SessionDetail | null | undefined): CharterView | null {
  if (!s?.charter) return null
  const c = s.charter
  const scope = {
    instruments: c.scope?.instruments ?? [],
    asset_classes: c.scope?.asset_classes ?? [],
    families: c.scope?.families ?? [],
    timeframes: c.scope?.timeframes ?? [],
  }
  const windows = s.schedule?.windows ?? []
  return {
    objective: objectiveWord(c.objective),
    scope,
    scope_is_any: Object.values(scope).every((l) => l.length === 0),
    seeds: (c.seeds ?? []).map((x) => x.question),
    permitted_workflows: c.permitted_workflows ?? [],
    workflows_are_all: (c.permitted_workflows ?? []).length === 0,
    exclusions: c.exclusions ?? [],
    standing_constraints: c.standing_constraints ?? [],
    exploration_share: typeof c.exploration_share === 'number' ? c.exploration_share : null,
    delta_practical: typeof c.delta_practical_default === 'number' ? c.delta_practical_default : null,
    version: s.charter_version ?? (typeof c.version === 'number' ? c.version : null),
    usd_per_day: s.budget?.usd_per_day ?? null,
    usd_total: s.budget?.usd_total ?? null,
    trials_total: s.budget && s.budget.trials_total > 0 ? s.budget.trials_total : null,
    max_runs: s.budget?.max_runs ?? null,
    autonomy: s.autonomy ?? null,
    replenish: s.replenish ?? null,
    review_every: s.review_every ?? null,
    windows,
    always_open: windows.length === 0,
    max_concurrent_runs: s.schedule?.max_concurrent_runs ?? null,
  }
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

export type DigestKind =
  | 'concluded'
  | 'finding'
  | 'refuted'
  | 'approval_defaulted'
  | 'monitor'
  | 'budget'
  | 'waiting'

/** One line of "Since you were away" — a verdict or a state change, never an event name. */
export interface DigestLine {
  id: string
  at: string
  kind: DigestKind
  text: string
  href: string | null
  mark?: ProvenanceMark | null
}

/**
 * A kind of line this deployment cannot see, and why.
 *
 * `/api/desk/digest` has sent these since it was built and this type did not
 * admit them, so the panel dropped them on the floor. That is the failure the
 * server went out of its way to prevent: three of the seven kinds have no source
 * here — `finding` and `refuted` need phase 4's memory migrations, `monitor`
 * needs a state-*change* log — and a digest that silently omits them says
 * "nothing was refuted" when it means "I cannot see refutations". Only one of
 * those two means the operator can stop looking.
 */
export interface DigestGap {
  kind: DigestKind
  why: string
}

export interface AwayDigest {
  since: string
  until: string
  lines: DigestLine[]
  /**
   * The server's own "do not show this at all" — it answers `absent` for a gap
   * under an hour rather than an empty list, because "nothing happened" and
   * "you have not been away" are different sentences.
   */
  absent?: boolean
  reason?: string | null
  kinds_not_covered?: DigestGap[]
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

/**
 * The wire shape `/api/trading/status` actually answers with today. COMP-007
 * (Set S phase 7) adds scope, who tripped it and why; until then the desk shows
 * the two fields that exist rather than inventing the other three.
 */
export interface KillSwitchState {
  trading_enabled: boolean
  kill_switch_active: boolean
  scope?: string | null
  tripped_by?: string | null
  reason?: string | null
  at?: string | null
}

/** A window in which the monitor's limits are deliberately not enforced (§4.3). */
export interface MaintenanceWindow {
  window_id: string
  opened_by: string
  reason: string
  expires_at: string
}

/** What `GET /api/monitor/kill-switch` answers. */
export interface MonitorKillSwitch {
  /** The lease word — `held`, `expired`, and so on, lowercased by the server. */
  lease: string
  orders_permitted: boolean
  /** The scopes that are halted. Empty means none are, which is measured. */
  halted_scopes: string[]
  maintenance_windows: MaintenanceWindow[]
}

/** One `(tier, workflow)` pair's share of runs that produced nothing. */
export interface BarrenRow {
  tier: string
  workflow: string
  runs: number
  barren: number
  /** `null` when there are no runs — never `0`, which reads as "never wasteful". */
  rate: number | null
}

export interface BarrenRates {
  days: number
  max_days: number
  runs: number
  rows: BarrenRow[]
  /** Runs whose profile the platform no longer ships, named rather than folded. */
  unresolved: Array<{ profile: string; runs: number }>
}

/** A request for a human decision (AGENT-013 section 7.5). */
export interface Proposal {
  proposal_id: string
  kind: string
  /** 1 = platform artifacts, 2 = paper. Derived from the kind, never sent. */
  ring: number
  envelope: string
  dossier_ref: string
  /** What is being promoted, armed or changed, as a person reads it. */
  subject: string
  /** The row the approval writes to. `null` on proposals predating 0131. */
  subject_id: string | null
  rationale: string
  /**
   * Whether approving it would actually do anything.
   *
   * Shown in the queue rather than discovered after deciding: two of the three
   * kinds have no approval-side writer yet, and an approval that changes
   * nothing is a fact the person deciding is owed *before* they decide.
   */
  actionable: boolean
}

/** What the platform did about a decision. */
export interface ProposalDecision {
  proposal_id: string
  decision: string
  decided_by: string
  /** Whether the registry write happened. Separate from the decision. */
  performed: boolean
  effect: Record<string, unknown>
}

export interface DossierComment {
  comment_id: string
  author: string
  claim: string | null
  body: string
  created_at: string
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

  /**
   * COMP-007's own kill-switch read, which `/api/trading/status` is not.
   *
   * `desk.ts` has said since it was written that scope, who tripped it and why
   * "come with COMP-007"; `/api/monitor/kill-switch` has been served since that
   * phase landed and nothing called it, so the Monitor page went on printing
   * `scope global` — a word the endpoint it did call has never sent.
   *
   * It carries the thing `/api/trading/status` cannot: which *scopes* are
   * halted, and the open maintenance windows. §4.3 says an open window must be
   * as visible in the desk chrome as a trip, because a window nobody can see is
   * a hole nobody can audit, and the desk showed it nowhere.
   */
  monitorKillSwitch: () => optional<MonitorKillSwitch>(client.get('/api/monitor/kill-switch')),

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
  // A reviewer's notes on a dossier, which are deliberately not part of it: the
  // document is a snapshot and a comment is a later reading of it (S-8.7).
  dossierComments: (ticket: string) =>
    optional<{ comments: DossierComment[] }>(client.get(`/api/dossiers/${ticket}/comments`)),
  addDossierComment: (ticket: string, body: string, claim?: string) =>
    client.post(`/api/dossiers/${ticket}/comments`, { body, claim: claim ?? null }),
  monitor: () => optional<{ strategies: Array<Record<string, unknown>> }>(client.get('/api/monitor/strategies')),

  // — Proposals (AGENT-013 section 7.5; S-10.9). The agent creates an inert row
  //   and a person decides here; the registry write happens at that moment, as
  //   that person. Distinct from `ApprovalsInbox`, which answers a *running*
  //   agent's `AskHuman` pause — one blocks a run and the other is decided
  //   later, and collapsing them would make a queue of waiting agents and
  //   pending decisions look like one list. —
  proposals: () => optional<{ pending: Proposal[] }>(client.get('/api/lifecycle/proposals')),

  /**
   * Barren rate per `(tier, workflow)` (AGENT-013 section 10).
   *
   * Rows, never one number: "12% barren" is something nobody can act on, and
   * "`survey` on the 7B is barren 60% of the time" names a tier and a workflow.
   */
  barren: (days = 30) =>
    // `params`, not a query string in the path: `the_desk_calls_routes_that_exist`
    // matches the path against the router, and a `?…` glued on makes a served
    // route read as unserved.
    optional<BarrenRates>(client.get('/api/lifecycle/barren', { params: { days } })),
  decideProposal: (id: string, decision: 'approved' | 'rejected', note?: string) =>
    optional<ProposalDecision>(
      client.post(`/api/lifecycle/proposals/${id}/decision`, { decision, note: note ?? '' }),
    ),
  awayDigest: (since: string) => optional<AwayDigest>(client.get('/api/desk/digest', { params: { since } })),
  sessions: () => optional<{ sessions: DeskSession[] }>(client.get('/api/research/sessions')),
  /**
   * One session, with the charter the list does not carry.
   *
   * Two calls rather than one because the list is the only place a
   * conversation id maps to a session id, and the charter is only on the
   * detail. Neither route takes the other's key.
   */
  session: (sessionId: string) =>
    optional<SessionDetail>(client.get(`/api/research/sessions/${sessionId}`)),
  /**
   * One door for both halves of the toggle (AGENT-014 §2.1).
   *
   * `model` is the profile the session runs on. It is sent because the
   * frontier-only rule is enforced on the server as well as in the composer —
   * a rule enforced only in the browser is a rule a `curl` walks around — and
   * the server answers with the profile named when it refuses.
   */
  createSession: (body: {
    mode: Mode
    continuous: boolean
    question?: string
    charter?: Charter
    workflow?: Workflow
    model?: string
  }) =>
    optional<{ session_id: string | null; conversation_id: string; continuous: boolean }>(
      client.post('/api/research/sessions', body),
    ),
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
