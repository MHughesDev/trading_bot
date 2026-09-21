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
  /**
   * The `charter_amended` events behind `charter_version`, newest first, last
   * 100. Optional on the type because a build of the route older than the read
   * omits it — and an absent list is not an unamended charter.
   */
  amendments?: WireAmendment[]
  /** Events the fold could not replay. Reported rather than hidden. */
  anomalies?: unknown[]
}

/**
 * One amendment, as `GET /api/research/sessions/{id}` serves it.
 *
 * `changed` is a field-level diff of the charter as it was against the charter
 * the amendment wrote — and it is `null`, not `{}`, for an amendment appended
 * before the route computed one. The two are different sentences: `{}` says the
 * amendment moved nothing, `null` says nobody wrote down what it moved. The
 * route keeps them apart (`payload.changed.filter(is_object)`), so this does
 * too.
 */
export interface WireAmendment {
  seq: number
  version: number | null
  by: string | null
  changed: Record<string, unknown> | null
  at: string
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

/**
 * One amendment, decoded for the Charter tab.
 *
 * `silence` is the whole point of this type. A charter that has been amended
 * three times and can show nothing about any of them is the tab reporting that
 * decisions were taken and none of them can be read — and there are two ways to
 * arrive at that screen, which must not print the same words:
 *
 * * `unrecorded` — the route sent `changed: null`. The amendment predates the
 *   diff; what changed was never written down and cannot be recovered.
 * * `none` — the route sent `{}`. Both charters were compared and nothing
 *   differed, which is a measurement.
 *
 * Reading the first as the second would turn an absent record into a claim
 * about what somebody decided.
 */
export interface AmendmentView {
  seq: number
  version: number | null
  by: string | null
  at: string
  /** The fields this amendment moved, `from` → `to`, already in words. */
  changes: Array<{ field: string; from: string; to: string }>
  /** Why `changes` is empty, or `null` when it is not. */
  silence: 'unrecorded' | 'none' | null
}

/**
 * One side of one field of a diff, in words.
 *
 * `null`/absent reads as "not recorded" rather than as an empty string, and an
 * empty list reads as "nothing named" rather than as a blank — the charter's own
 * empty-is-everything reading is stated where the field is rendered, not
 * guessed at here.
 */
export function amendmentValueWords(v: unknown): string {
  if (v === null || v === undefined) return 'not recorded'
  if (Array.isArray(v)) return v.length === 0 ? 'nothing named' : v.map((x) => amendmentValueWords(x)).join(', ')
  if (typeof v === 'string') return v === '' ? 'empty' : v
  if (typeof v === 'number' || typeof v === 'boolean') return String(v)
  return JSON.stringify(v)
}

/** The amendments a session read carries, newest first, decoded for the tab. */
export function sessionAmendments(s: SessionDetail | null | undefined): AmendmentView[] {
  const rows = Array.isArray(s?.amendments) ? (s as SessionDetail).amendments ?? [] : []
  return rows.map((a) => {
    const diff = a?.changed
    // `null` and `{}` are told apart here and nowhere else. `typeof null` is
    // `'object'`, so the null check comes first or every unrecorded amendment
    // renders as one that changed nothing.
    const legible = diff !== null && diff !== undefined && typeof diff === 'object' && !Array.isArray(diff)
    const changes = legible
      ? Object.entries(diff as Record<string, unknown>).map(([field, move]) => {
          const m = (move ?? {}) as Record<string, unknown>
          const hasSides = typeof move === 'object' && move !== null && ('from' in m || 'to' in m)
          return {
            field,
            from: hasSides ? amendmentValueWords(m.from) : 'not recorded',
            to: hasSides ? amendmentValueWords(m.to) : amendmentValueWords(move),
          }
        })
      : []
    return {
      seq: typeof a?.seq === 'number' ? a.seq : 0,
      version: typeof a?.version === 'number' ? a.version : null,
      by: typeof a?.by === 'string' && a.by ? a.by : null,
      at: typeof a?.at === 'string' ? a.at : '',
      changes,
      silence: changes.length > 0 ? null : legible ? 'none' : 'unrecorded',
    }
  })
}

// ── The digest, which renders as a card in the journal (AGENT-014 §2.4) ──────
//
// `GET …/{id}/digests` answers rows whose `body` is `research_session::digest::
// Digest` verbatim. Two things about that body decide how it is decoded here.
//
// **Every line carries a reference and the card has to open it.** §2.4's "each
// line opens the thing it names" and §12.11's "every line resolves on click"
// are the acceptance, and a line whose reference has an empty id cannot meet
// it. `Digest::unresolvable` is the Rust reader for exactly that hole; this is
// its counterpart, so the card can say a line opens nothing rather than render
// a control that does nothing.
//
// **The spend has a unit and the unit is not dollars.** Migration 0139 adds
// `session_digests.spend_unit`, NOT NULL and CHECKed to ('trials','usd'),
// because the number this platform can measure per run is trials —
// `agent_runs.trials_spent` — and there is no per-run USD anywhere. The digest
// body's own budget sentence reads "3.00 spent over 2 runs" with **no unit in
// it**, and that sentence lands on a card beside a charter that says $25/day.
// So the unit is read here, optionally, and the card refuses a currency symbol
// without one. A number that looks like dollars and is not is worse than a
// number with no unit, because only the second one makes the reader check.

/** The units a digest's spend can be counted in (migration 0139's CHECK). */
export type SpendUnit = 'trials' | 'usd'

/** What a digest line opens. `kind` mirrors `digest::RefKind`. */
export interface SessionDigestRef {
  kind: string
  id: string
}

export interface SessionDigestLine {
  at: SessionDigestRef
  text: string
}

export interface SessionDigestSection {
  heading: string
  lines: SessionDigestLine[]
}

export interface SessionDigestPeriod {
  from: string
  to: string
  from_seq: number
  to_seq: number
}

export interface SessionDigestBudget {
  runs_finished: number
  spent: number
  knowledge_units: number
  /** `null` for a barren period: no knowledge means no cost *per* unit. */
  cost_per_knowledge_unit: number | null
}

/** `digest::Digest`, as the route sends it inside `body`. */
export interface SessionDigestBody {
  session?: string
  period?: SessionDigestPeriod
  budget?: SessionDigestBudget
  sections?: SessionDigestSection[]
  /**
   * Optional, and read as well as the row's column: the driver writes the unit
   * onto the `digest_written` event too, so a writer that folded it into the
   * body would be honest rather than wrong. Neither place is assumed.
   */
  spend_unit?: string | null
}

/** One row of `GET …/{id}/digests`. */
export interface SessionDigestRow {
  digest_id: string
  from_seq: number
  to_seq: number
  body: SessionDigestBody | null
  created_at: string
  /**
   * `session_digests.spend_unit` (migration 0139), NOT NULL in the table and
   * CHECKed to `('trials','usd')`. `GET …/{id}/digests` selects it, so a row
   * that arrives without one came from a build that did not — and absent is
   * still not `usd`, which is why this stays optional and `readSpendUnit`
   * refuses anything it does not recognise.
   */
  spend_unit?: string | null
}

/** A digest decoded for the card, with the unit settled one way or the other. */
export interface SessionDigestView {
  digest_id: string
  created_at: string
  from_seq: number
  to_seq: number
  period: SessionDigestPeriod | null
  budget: SessionDigestBudget | null
  /** In `SESSION_DIGEST_HEADINGS` order; a heading this build does not know goes last. */
  sections: SessionDigestSection[]
  /** `null` when nothing said. Never defaulted to `usd`. */
  spend_unit: SpendUnit | null
  /** Lines whose reference opens nothing — the §12.11 hole, counted. */
  unresolvable: number
}

/** `digest::Heading::ALL`, in §2.4's order, with `Heading::title`'s words. */
export const SESSION_DIGEST_HEADINGS: Array<{ key: string; title: string }> = [
  { key: 'runs_finished', title: 'Runs finished' },
  { key: 'findings', title: 'Findings' },
  { key: 'negatives', title: 'Negative results' },
  { key: 'questions', title: 'Questions' },
  { key: 'contradictions', title: 'Contradictions' },
  { key: 'proposals_awaiting', title: 'Awaiting you' },
  { key: 'budget', title: 'Budget' },
  { key: 'next_up', title: 'Next up' },
  { key: 'paused', title: 'Stopped or paused' },
]

export function sessionDigestHeading(key: string): string {
  return SESSION_DIGEST_HEADINGS.find((h) => h.key === key)?.title ?? key
}

function readSpendUnit(raw: unknown): SpendUnit | null {
  return raw === 'trials' || raw === 'usd' ? raw : null
}

/** Whether a reference names something the card can open at all. */
export function sessionDigestRefResolves(r: SessionDigestRef | null | undefined): boolean {
  return !!r && typeof r.id === 'string' && r.id.trim() !== ''
}

export function sessionDigestView(row: SessionDigestRow | null | undefined): SessionDigestView | null {
  if (!row) return null
  const body = row.body ?? {}
  const known = new Map(SESSION_DIGEST_HEADINGS.map((h, i) => [h.key, i]))
  const sections = [...(body.sections ?? [])]
    .filter((s) => !!s && Array.isArray(s.lines))
    // An unrecognised heading sorts last rather than being dropped: a section
    // whose name this build does not know still holds lines the session wrote.
    .sort((a, b) => (known.get(a.heading) ?? 99) - (known.get(b.heading) ?? 99))
  return {
    digest_id: row.digest_id,
    created_at: row.created_at,
    from_seq: row.from_seq,
    to_seq: row.to_seq,
    period: body.period ?? null,
    budget: body.budget ?? null,
    sections,
    // The row first, because that is where 0139's column is; the body second,
    // because the driver also writes the unit onto its event payload.
    spend_unit: readSpendUnit(row.spend_unit) ?? readSpendUnit(body.spend_unit),
    unresolvable: sections.flatMap((s) => s.lines).filter((l) => !sessionDigestRefResolves(l?.at)).length,
  }
}

/**
 * Where a digest line goes when it is clicked (§2.4's "each line opens the
 * thing it names"; §12.11's "every line resolves on click").
 *
 * A function rather than a `switch` inside the card, because the interesting
 * half is the `none` arm: nine `RefKind`s exist and the desk does not have a
 * surface for all of them. The ones it cannot open have to say so on the card
 * rather than render as a control that swallows the click, and the reason has
 * to be specific enough to act on — which is also what makes the gap countable
 * from a test instead of discovered by clicking.
 */
export type SessionDigestTarget =
  | { kind: 'run'; runId: string }
  /** A tab of the session's own right panel. */
  | { kind: 'panel'; tab: 'agenda' | 'charter' }
  | { kind: 'route'; to: string }
  | { kind: 'none'; why: string }

export function sessionDigestTarget(ref: SessionDigestRef | null | undefined): SessionDigestTarget {
  if (!sessionDigestRefResolves(ref)) {
    return { kind: 'none', why: 'this line carries no id, so there is nothing to open' }
  }
  const id = encodeURIComponent((ref as SessionDigestRef).id)
  switch ((ref as SessionDigestRef).kind) {
    case 'run':
      return { kind: 'run', runId: (ref as SessionDigestRef).id }
    case 'item':
      return { kind: 'panel', tab: 'agenda' }
    case 'session':
      return { kind: 'panel', tab: 'charter' }
    case 'proposal':
      // No `?find=`: the queue has no filter to honour one, and a query
      // parameter nothing reads is a claim the surface does not keep. Every
      // proposal a digest lists is one still awaiting a decision, which is
      // exactly the list this page shows.
      return { kind: 'route', to: '/desk/proposals' }
    case 'finding':
    case 'contradiction':
      // A contradiction is a pair of findings, and the memory browser's
      // conflict view is where a pair is read.
      return { kind: 'route', to: `/desk/memory?bucket=findings&find=${id}` }
    case 'negative':
      return { kind: 'route', to: `/desk/memory?bucket=negative&find=${id}` }
    case 'question':
      return { kind: 'route', to: `/desk/memory?bucket=questions&find=${id}` }
    case 'event':
      // Honest, and a gap worth naming: `session_events` is served as SSE and
      // as nothing a person can read a row of. The line keeps its id so it
      // starts opening the moment a log view exists.
      return {
        kind: 'none',
        why: 'nothing in the desk renders a row of the session log yet, so this event cannot be opened from here',
      }
    default:
      return { kind: 'none', why: `the desk has no surface for a ${(ref as SessionDigestRef).kind}` }
  }
}

/**
 * The budget sentence, written here rather than taken from the server's line.
 *
 * The server's own text is "3.00 spent over 2 runs — 1.50 per knowledge unit"
 * with **no unit in it**, and that writer is outside this surface. Rendered
 * verbatim beside a charter that says $25/day, it is a trial count a reader
 * will read as money. So the card builds the sentence from the numbers and the
 * unit, and where there is no unit it says so rather than choosing one.
 */
export function sessionDigestBudgetLine(view: Pick<SessionDigestView, 'budget' | 'spend_unit'>): string {
  const b = view.budget
  if (!b) return 'This digest carries no budget summary, so nothing here says what the period cost.'
  const runs = `${b.runs_finished} run${b.runs_finished === 1 ? '' : 's'}`
  const amount =
    view.spend_unit === 'usd'
      ? `$${b.spent.toFixed(2)}`
      : view.spend_unit === 'trials'
        ? `${b.spent.toFixed(2)} trials`
        : `${b.spent.toFixed(2)}, in a unit nothing recorded — not dollars`
  const per =
    b.cost_per_knowledge_unit === null
      ? ' and no knowledge units, so there is no cost per unit to report'
      : view.spend_unit === 'usd'
        ? ` — $${b.cost_per_knowledge_unit.toFixed(2)} per knowledge unit`
        : view.spend_unit === 'trials'
          ? ` — ${b.cost_per_knowledge_unit.toFixed(2)} trials per knowledge unit`
          : ` — ${b.cost_per_knowledge_unit.toFixed(2)} per knowledge unit, in that same unrecorded unit`
  return `${amount} spent over ${runs}${per}`
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

/**
 * §2.2's Running-now row as the spec sketched it — **not** a served shape.
 *
 * Nothing calls this. It is kept as the spec's own list of columns, and three of
 * them are why the served route is a different type: `usd_tenth_cents` and
 * `cache_hit_pct` are numbers this platform cannot produce (there is no per-run
 * USD anywhere, and a hit rate over a nullable `prompt_tokens` has no
 * denominator), and `step` is the working state, whose route is unbuilt. What
 * `GET /api/desk/running` answers is `DeskRunningRun`, and what it cannot answer
 * it names in `not_recorded` rather than sending a zero.
 */
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

/**
 * One working run, as `GET /api/desk/running` answers it (AGENT-012 §2.2).
 *
 * One row per **run**, not per conversation: every column here is a fact about
 * a run, and a conversation with three finished turns and one working one has
 * one tier and one token count that matter. The conversation is carried along
 * as the thing the row opens.
 *
 * **Every nullable field here is a real state.** Migration 0062 made the ten
 * telemetry columns nullable so a run that crashed before filling them is still
 * a run. `trials_spent: null` is "not recorded"; `0` is a run that searched
 * nothing, and they are different facts about whether the money was spent.
 */
export interface DeskRunningRun {
  run_id: string
  conversation_id: string | null
  title: string | null
  status: string
  mode: string
  stage: string
  model: string
  /** `null` for a Chat turn, which is not doing one of the seven workflows. */
  workflow: string | null
  /** `null` for a run started before the tier was recorded; never guessed from the model string. */
  tier: string | null
  profile_name: string | null
  iterations: number | null
  /** Prompt size, not what was billed. `null` when the run recorded none. */
  prompt_tokens: number | null
  cache_read_tokens: number | null
  trials_spent: number | null
  outcome: string | null
  created_at: string
  started_at: string | null
  /** The last entry in the run's own transcript — `null` for a run that has written none. */
  last_transcript_entry: { seq: number | null; kind: string | null; at: string | null } | null
}

/**
 * A column §2.2 asks for that nothing in this deployment records, and why.
 *
 * Rendered, never dropped. This array is the route saying what it cannot know;
 * hiding it turns a stated absence back into a silent one, which is how a blank
 * dollar column comes to be read as free.
 */
export interface NotRecordedNote {
  field: string
  why: string
}

export interface DeskRunning {
  runs: DeskRunningRun[]
  not_recorded: NotRecordedNote[]
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
 * What a read of an unbuilt endpoint answered — the same rule as `optional`,
 * keeping the two apart.
 *
 * `optional` folds "the route is not mounted" and "the route answered with
 * nothing" into one `null`, and for most panels that is the same sentence. For
 * the State tab it is two opposite ones. `/api/agent/runs/{id}/state` is **not
 * mounted on this platform** — it is one of AGENT-012 §17's ✗ rows and
 * `crates/api/src/routes/mod.rs` registers `/api/agent/runs/{id}`,
 * `…/messages` and `…/cancel` and no `…/state`; an unauthenticated GET answers
 * 404 where every mounted route answers 401. So every conversation's default tab
 * rendered *"No working state yet — it appears as soon as the run's first step
 * lands"*, which is a promise about a step, on a platform where no step will
 * ever make it appear.
 */
export type ServedRead<T> = { served: true; data: T } | { served: false }

async function served<T>(p: Promise<{ data: T }>): Promise<ServedRead<T>> {
  try {
    return { served: true, data: (await p).data }
  } catch (e) {
    const status = (e as { response?: { status?: number } }).response?.status
    if (status === 404 || status === 501 || status === 405) return { served: false }
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

/**
 * The three words `POST /api/monitor/strategies/{id}/{action}` accepts.
 *
 * A union and not a `string`, because the route's fourth branch is
 * `other => bad_request("unknown action …")` — a typo here would be a 400 that
 * looks exactly like a control that quietly did nothing.
 */
export type StrategyAction = 'scale' | 'halt' | 'resume'

/** What to do with the open position when a strategy is halted. */
export type HaltPolicy = 'hold' | 'flatten' | 'reduce'

/**
 * The body each of the three actions needs, as the route reads it.
 *
 * Every field is optional on the wire and each action requires a different
 * subset, which is what [`strategyActionBody`] exists to get right in one place:
 *
 * * `scale` refuses without `scale_pct`, and refuses it outside `1..=100`.
 * * `halt` refuses without a `reason`.
 * * `resume` is a reset by another name — it needs a `reason` **and** a
 *   `confirm_scope` equal to `strategy:<id>` exactly, compared byte-for-byte on
 *   the server.
 */
export interface StrategyActionBody {
  reason?: string
  scale_pct?: number
  confirm_scope?: string
  halt_policy?: HaltPolicy
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
  /**
   * The working state, and whether anything answered for it.
   *
   * `ServedRead` rather than `WorkingState | null`: the difference between a run
   * with no snapshot and a platform with no endpoint is the difference between
   * "wait" and "stop waiting", and the tab has to say which.
   */
  state: (runId: string, step?: number) =>
    served<WorkingState>(client.get(`/api/agent/runs/${runId}/state`, { params: { step } })),
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

  /**
   * The runs that are working right now (AGENT-012 §2.2).
   *
   * Desk home read `running` off the conversation list, which answers "is any
   * turn active in this conversation" and none of §2.2's other columns. This
   * route answers them per run — and names, in `not_recorded`, the two it
   * deliberately does not send. **Do not compute a cache-hit percentage from
   * these two token counts:** `prompt_tokens` is nullable, and a ratio with a
   * missing denominator is an invented number.
   */
  running: () => optional<DeskRunning>(client.get('/api/desk/running')),

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
  /**
   * The other half of the pair the board already drew a button for.
   *
   * `AgendaTab` has rendered **Approve** beside **Veto** since it was written
   * and `Conversation` passed no `onApprove`, so under `supervised` — the
   * autonomy in which every item waits for a human before it starts — the one
   * control that releases an item reached nothing. The route has been served
   * since S-11.14.
   */
  approveAgendaItem: (sessionId: string, itemId: string) =>
    optional<{ ok: boolean }>(
      client.post(`/api/research/sessions/${sessionId}/agenda/${itemId}/approve`),
    ),
  /**
   * A drag on the board (AGENT-014 §9).
   *
   * **`order` is the board's whole order, every id once, in the arrangement the
   * board shows.** A `user` source bypasses scoring entirely (§4.2), so which
   * cards earn one cannot be left to a client: the route compares the body
   * against the board's own order, subtracts the longest run that is already in
   * it and marks only the remainder — a card dragged down prioritises nothing.
   * It refuses duplicates, and it refuses a body of one id, naming
   * `POST …/{item}/prioritise` as the route for that intent. Sending the
   * moved-up cards instead is both redundant (the route already subtracts the
   * rest) and, for the single-card drag, a 400.
   *
   * `prioritised` is how many cards the route read as promoted and `considered`
   * how many of the body were live agenda items, so a board posted with nothing
   * moved is legible as such rather than as a request that was thrown away.
   */
  reorderAgenda: (sessionId: string, order: string[]) =>
    optional<{ ok: boolean; prioritised?: number; considered?: number }>(
      client.post(`/api/research/sessions/${sessionId}/agenda/order`, { order }),
    ),
  /**
   * The session's digests, newest first (AGENT-014 §2.4).
   *
   * Served since S-11.14 and called by nothing until the journal had a card to
   * put one on.
   */
  digests: (sessionId: string) =>
    optional<{ digests: SessionDigestRow[] }>(
      client.get(`/api/research/sessions/${sessionId}/digests`),
    ),

  /**
   * Scale, halt or resume one strategy under the monitor (COMP-007 §6).
   *
   * `POST /api/monitor/strategies/{strategy_id}/{action}` has been served since
   * COMP-007 landed and the desk's **Scale down** button called nothing at all
   * — a control that looks like a control and does not reach the platform,
   * which on this page means an operator believing they have scaled a losing
   * strategy back when they have not. Not a missing endpoint: a missing caller.
   *
   * The three words are the route's own (`scale`, `halt`, `resume`) and it
   * refuses anything else by name, so they are a union here rather than a
   * string.
   *
   * Every refusal this can draw is one a person has to read, so nothing here is
   * routed through `optional()`: a 403 from `require_control` (an agent or
   * service token never carries monitor control), a 409 `resume_refused` from
   * the reset rate limit, a 400 for a missing reason. Turning those into `null`
   * would put "it did not work" and "it worked" on the same screen.
   */
  strategyAction: (strategyId: string, action: StrategyAction, body: StrategyActionBody) =>
    client.post<Record<string, unknown>>(
      `/api/monitor/strategies/${strategyId}/${action}`,
      body,
    ),

  /**
   * The global halt and its reset (COMP-007 §5; AGENT-012 §2.4, §11).
   *
   * Both were posted from the Monitor panel with **no body at all**. `Halt all`
   * survived it — `trip_kill_switch` takes `Option<Json<KillBody>>` and invents
   * the reason "manual kill from /api/trading/kill" — but `reset_kill_switch`
   * takes `Json<ResumeBody>` by value, so a bodiless POST is rejected by the
   * extractor before the handler runs. `Reset (logged)` could not ever have
   * worked, and because the call was a bare `void api.post(…)` it said nothing
   * when it did not: the operator clicks the control that lets trading start
   * again, sees no change, and has no way to tell a refusal from a switch that
   * is simply slow to clear.
   *
   * Deliberately outside `optional()`, like `strategyAction` and for the same
   * reason: the 403 from `permits_monitor_control` and the 409 `reset_refused`
   * from the rate limit are answers a person has to read, and folding them into
   * `null` puts "it worked" and "it did not" on the same screen.
   */
  haltAll: (reason: string) =>
    client.post<Record<string, unknown>>('/api/trading/kill', { reason }),

  /**
   * `confirm_scope` is compared byte for byte against `Scope::Global`'s
   * `Display`, which is the literal word `global`, and `reason` must be
   * non-empty. Both refusals are stated in the dialog before the request.
   */
  resumeAll: (body: { confirm_scope: string; reason: string }) =>
    client.post<Record<string, unknown>>('/api/trading/resume', body),

  // — Controls. Steer and cancel exist; **pause, resume and escalate are gone.**
  //
  //   They were three `optional()` posts to `POST /api/agent/runs/{id}/pause`,
  //   `…/resume` and `…/escalate`, all three of which answer 404 on this
  //   platform where every mounted route answers 401. `optional()` turns a 404
  //   into `null`, so each call was a request that failed and reported nothing,
  //   and `ControlsRow` drew a button for each. Keeping them here as "wired the
  //   day they land" is what let the buttons stay: the next caller finds a
  //   method on `deskApi`, and a method on `deskApi` reads as a route.
  //
  //   The unbuilt reads above are a different case and stay: `state()` and
  //   `trajectory()` return data the panel renders an honest absence for.
  //   A *control* has no honest absence — a click either reaches the platform
  //   or it does not happen. —
  steer: (sessionId: string, body: { kind: 'steer' | 'interrupt' | 'stop' | 'answer'; text?: string }) =>
    client.post(`/api/agent/sessions/${sessionId}/steer`, body),
  cancel: (conversationId: string) => client.post(`/api/agent/conversations/${conversationId}/cancel`),
}
