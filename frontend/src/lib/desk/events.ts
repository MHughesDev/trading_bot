// One mapper, two streams.
//
// `/agent` rendered sixteen `AgentMessage` kinds through `ActivityRow`; `/research`
// rendered `TimelineEvent` through an unrelated `ToolCard`. The two surfaces
// disagreed about what a tool call looks like, which is how a person ends up
// reading raw JSON to find out what happened. The desk maps both wire shapes into
// one `DeskCard` list, so there is exactly one place that decides what a kind
// means and exactly one renderer.
import type { CardKind, DeskCard, EvidenceClass, Provenance, ProvenanceMark, RiskClass } from '@/api/desk'

/** The union of what the two streams put on the wire. */
export interface WireEvent {
  seq: number
  kind: string
  content: Record<string, unknown>
  created_at: string
}

const KIND_MAP: Record<string, CardKind> = {
  user: 'you',
  assistant: 'agent',
  tool_call: 'tool',
  tool_result: 'tool',
  core_tool: 'tool',
  plan: 'plan',
  validation: 'correction',
  compaction: 'compaction',
  degradation: 'halt',
  error: 'halt',
  final: 'report',
  // The research timeline's names, which the desk treats as first-class cards.
  finding: 'finding',
  negative_result: 'negative',
  decision: 'decision',
  question: 'question',
  ask_user: 'question',
  approval: 'approval',
  job: 'job',
  gate: 'gate',
  escalation: 'escalation',
  stage: 'stage',
  preregistration: 'prereg',
  prereg: 'prereg',
  offer: 'offer',
  research_offer: 'offer',
  proposal: 'proposal',
  steer: 'steer',
  halt: 'halt',
  report: 'report',
  // The run's compiled dossier (S-8.7). Without this line the event fell to
  // `raw` and was folded away, so a dossier the platform had compiled and
  // stored had no way to be opened from the conversation that produced it —
  // and the Report card's "Open dossier" button, which reads `detail.ticket`,
  // could never appear.
  dossier: 'report',
}

/** Kinds folded behind the details disclosure (§3.2), off by default. */
const RAW_KINDS = new Set([
  'status',
  'canary',
  'exposure',
  'policy',
  'usage',
  'heartbeat',
  // What context assembly chose and what bound it (AGENT-005 §28.3). Behind the
  // disclosure rather than on the timeline: it is the answer to "why did the
  // model not know that?", which is a question asked *about* a step rather than
  // a thing that happened in one.
  'selection',
])

function str(v: unknown): string | null {
  return typeof v === 'string' && v.length > 0 ? v : null
}

function num(v: unknown): number | null {
  return typeof v === 'number' && Number.isFinite(v) ? v : null
}

/** A counter whose zero cannot be a measurement — see `costCalls`. */
function nonZero(v: unknown): number | null {
  const n = num(v)
  return n === null || n === 0 ? null : n
}

/** Ids the desk knows how to resolve, matched wherever they appear. */
const ID_PATTERN = /\b(?:exp|study|gate|f|run|bt|ds|job|tkt|sweep)[_-][A-Za-z0-9]{3,}\b/g

export function extractIds(content: Record<string, unknown>): string[] {
  const out = new Set<string>()
  for (const [k, v] of Object.entries(content)) {
    if (typeof v === 'string' && (k.endsWith('_id') || k === 'id' || k === 'handle')) out.add(v)
    if (Array.isArray(v) && (k === 'ids' || k === 'evidence' || k === 'evidence_ids')) {
      for (const e of v) if (typeof e === 'string') out.add(e)
    }
  }
  const text = [str(content.summary), str(content.text), str(content.claim), str(content.message)]
    .filter(Boolean)
    .join(' ')
  for (const m of text.matchAll(ID_PATTERN)) out.add(m[0])
  return [...out]
}

const PROVENANCES: Provenance[] = ['platform', 'user', 'untrusted']
const CLASSES: EvidenceClass[] = ['direct_stat', 'derived', 'model_summary', 'human']

/**
 * Reads the provenance fence off the wire.
 *
 * Defaults to `untrusted` for anything the stream marks as fetched and to
 * `platform` otherwise — and never the other way round. An unknown provenance
 * that rendered as trusted would be exactly the failure §6.3a describes.
 */
export function readMark(content: Record<string, unknown>): ProvenanceMark | null {
  const raw = content.provenance
  const cls = content.evidence_class
  const source = str(content.source_host) ?? str(content.source)
  const provenance: Provenance | null =
    typeof raw === 'string' && (PROVENANCES as string[]).includes(raw)
      ? (raw as Provenance)
      : content.fetched_at || content.source_host
        ? 'untrusted'
        : null
  const evidence_class: EvidenceClass | null =
    typeof cls === 'string' && (CLASSES as string[]).includes(cls) ? (cls as EvidenceClass) : null
  if (!provenance && !evidence_class) return null
  return {
    provenance: provenance ?? 'platform',
    source,
    fetched_at: str(content.fetched_at),
    fetched_by_run: str(content.fetched_by_run),
    evidence_class,
  }
}

const RISKS: RiskClass[] = ['read', 'write', 'destructive', 'outbound']

export function readRisk(content: Record<string, unknown>): RiskClass {
  const r = content.risk
  return typeof r === 'string' && (RISKS as string[]).includes(r) ? (r as RiskClass) : 'read'
}

function headlineFor(kind: CardKind, e: WireEvent): string {
  const c = e.content
  switch (kind) {
    case 'you':
    case 'agent':
    case 'steer':
      return str(c.text) ?? str(c.message) ?? ''
    case 'tool':
      return str(c.name) ?? str(c.tool) ?? 'tool'
    case 'plan':
      return str(c.goal) ?? 'plan'
    case 'finding':
      return str(c.claim) ?? str(c.headline) ?? 'finding'
    case 'negative':
      return str(c.what) ?? str(c.message) ?? 'negative result'
    case 'decision':
      return str(c.choice) ?? str(c.question) ?? 'decision'
    case 'question':
      return str(c.question) ?? str(c.text) ?? 'question'
    case 'approval':
    case 'proposal':
      return str(c.action) ?? str(c.kind) ?? 'awaiting a decision'
    case 'job':
      return str(c.kind) ?? 'job'
    case 'gate':
      return str(c.gate) ?? 'gate'
    case 'correction':
      return str(c.message) ?? str(c.code) ?? 'rejected'
    case 'halt':
      return str(c.message) ?? str(c.error) ?? str(c.reason) ?? 'halted'
    case 'report':
      // A dossier event has two shapes and they must not collapse: a ticket
      // means one was compiled, and `not_compiled` means the run produced a
      // report whose claims its evidence did not support. The second is a
      // finding about the run, not a missing button, so it gets said.
      if (str(c.not_compiled)) return `no dossier: ${str(c.not_compiled) as string}`
      if (str(c.ticket) && !str(c.summary)) return 'the dossier for this run is ready'
      return str(c.summary) ?? str(c.assessment) ?? 'report'
    case 'stage':
      return str(c.stage) ?? 'stage'
    case 'prereg':
      return str(c.objective) ?? 'pre-registration'
    case 'offer':
      return str(c.question) ?? 'this needs a research run'
    case 'compaction':
      return str(c.summary) ?? 'context compacted'
    case 'escalation':
      return `${str(c.from) ?? '?'} → ${str(c.to) ?? '?'}`
    default:
      return str(c.message) ?? e.kind
  }
}

/**
 * Turns a wire event into a card.
 *
 * `step` is threaded by the caller rather than read off the event, because
 * today's `agent_messages` do not all carry one. Inventing a step per event
 * would put every card under its own header and defeat the grouping.
 */
export function toCard(e: WireEvent, step: number): DeskCard {
  const raw = RAW_KINDS.has(e.kind)
  const kind: CardKind = raw ? 'raw' : (KIND_MAP[e.kind] ?? 'raw')
  return {
    seq: e.seq,
    step,
    kind,
    at: e.created_at,
    headline: raw ? e.kind : headlineFor(kind, e),
    body: str(e.content.body) ?? str(e.content.detail) ?? null,
    ids: extractIds(e.content),
    mark: readMark(e.content),
    detail: e.content,
    raw: raw || undefined,
  }
}

/**
 * Maps a whole stream, assigning step numbers.
 *
 * A step is one model call and the tools it made. `assistant` and `plan` open a
 * new one; everything else joins the open step.
 */
export function toCards(events: WireEvent[]): DeskCard[] {
  let step = 1
  let sawWork = false
  const out: DeskCard[] = []
  for (const e of events) {
    if ((e.kind === 'assistant' || e.kind === 'plan') && sawWork) {
      step += 1
      sawWork = false
    }
    if (e.kind === 'tool_call' || e.kind === 'tool_result' || e.kind === 'core_tool') sawWork = true
    const explicit = num(e.content.step)
    out.push(toCard(e, explicit ?? step))
  }
  return out
}

/**
 * Merges new cards into the stream by `seq`.
 *
 * A reconnect replays from `Last-Event-ID`, and a server that replays one frame
 * too many must not produce a duplicate card — the whole point of resuming from
 * a cursor is that the person does not see the seam.
 */
export function mergeCards(existing: DeskCard[], incoming: DeskCard[]): DeskCard[] {
  if (incoming.length === 0) return existing
  const bySeq = new Map<number, DeskCard>()
  for (const c of existing) bySeq.set(c.seq, c)
  for (const c of incoming) bySeq.set(c.seq, c)
  return [...bySeq.values()].sort((a, b) => a.seq - b.seq)
}

// ── The two notes the right panel needs, read off the same stream ────────────
//
// Both of these were already on the wire and both panels rendered a literal
// `null`/`[]` instead. `usage` and `exposure` are in `RAW_KINDS` above, so they
// were folded behind the details disclosure and nothing else ever looked at
// them — the Cost tab said "no model call has been accounted yet" over a stream
// carrying one `usage` note per call, and the Tools tab said the harness had
// reported no exposure over a stream carrying one per step. Neither surface was
// broken in a way anybody could see, which is why both survived.

/**
 * The prompt broken into the layers §3.5 asks for. The local driver writes it on
 * every `usage` note and `decision` row (estimated with the harness's own
 * bytes-per-token arithmetic; `output` is the reply cap, not what was generated).
 */
export interface CallLayers {
  charter: number
  schemas: number
  body: number
  output: number
}

/**
 * One model call's accounting, as the harness's `usage` note records it.
 *
 * Every field the note does not carry is `null` rather than `0`. Three of them
 * are always null today and each for its own reason, which is why they are three
 * fields and not one "incomplete" flag:
 *
 * * `layers` — the note records one prompt total, not a split. A four-column
 *   table of zeros would be a measurement nobody made.
 * * `usd_tenth_cents` — there is no price table in the platform (OQ-16), so a
 *   cumulative of `0` would read as a free run.
 * * `cache_hit_pct` — a share of a prompt total the provider did not report.
 */
export interface CallCost {
  step: number
  layers: CallLayers | null
  cache_hit_pct: number | null
  /** What the provider billed for the prompt. */
  provider_prompt_tokens: number | null
  /** What the harness predicted it would be, before sending (§3.5's last line). */
  estimated_prompt_tokens: number | null
  output_tokens: number | null
  cache_read_tokens: number | null
  latency_ms: number | null
  usd_tenth_cents: number | null
}

/**
 * Reads the per-call accounting out of a run's stream.
 *
 * Rows stay in stream order and a step may appear more than once: a step that
 * was rejected by a rung and retried is two calls, and collapsing them would
 * hide the retry, which is the call worth finding.
 */
export function costCalls(events: WireEvent[]): CallCost[] {
  const out: CallCost[] = []
  for (const e of events) {
    if (e.kind !== 'usage') continue
    const c = e.content
    // A prompt of zero tokens is not a measurement, it is a provider that
    // reported no usage.
    //
    // `Usage::prompt_tokens()` sums three counters that all default to `0` when
    // the provider omits them, so a local backend that sends no usage block
    // yields `prompt_tokens: 0` — and `0 ?? input_tokens` is `0`, not the
    // fallback, because `??` only fires on null. The table then printed a
    // measured-looking **0** in the provider column and the cumulative summed
    // those zeros, over calls whose `input_tokens` on this platform's own rows
    // run to eleven and a half thousand. No model call has an empty prompt;
    // every one of them has a system block. So zero here means unreported.
    //
    // Deliberately not applied to `output_tokens`: a call that returned nothing
    // is a real thing (a refusal, an immediate stop), so a zero there is a
    // number somebody could have measured and this must not erase it.
    // What the provider itself called the prompt: `input + cache_read +
    // cache_creation`. This and this alone can be the denominator of a cache
    // share.
    const providerPrompt = nonZero(c.prompt_tokens)
    const prompt = providerPrompt ?? nonZero(c.input_tokens)
    const cacheRead = num(c.cache_read_tokens)
    out.push({
      step: num(c.step) ?? 0,
      // Never `{0,0,0,0}`: a note without a split is `null`. Read only when the
      // emitter wrote all four, so a half-split is not drawn as a whole one.
      layers: readLayers(c.layers),
      // Deliberately `providerPrompt`, not `prompt`. `input_tokens` is the
      // prompt *minus* the cached part, so dividing the cache read by it
      // overstates the hit — 200/800 reads as 25% where the provider's own
      // total makes it 20%. A fallback that is good enough to print in a column
      // labelled "provider prompt" is not good enough to be the bottom of a
      // fraction, and a cache rate nobody can check is exactly the number that
      // gets quoted.
      cache_hit_pct:
        providerPrompt !== null && cacheRead !== null ? (cacheRead / providerPrompt) * 100 : null,
      provider_prompt_tokens: prompt,
      estimated_prompt_tokens: num(c.est_prompt_tokens),
      output_tokens: num(c.output_tokens),
      cache_read_tokens: cacheRead,
      latency_ms: num(c.latency_ms),
      // Priced only when the note says so (the call's own rate row, config/prices.yaml).
      // An absent figure is unpriced, not free.
      usd_tenth_cents: num(c.usd_tenth_cents),
    })
  }
  return out
}

/**
 * The harness's `Note::Exposure`, which says what a step actually saw.
 *
 * `exposed` is the list; `namespaces`, `budget` and `pinned` are the note's own
 * answer to "why that set and not another", and they are carried because an
 * exposure without them is unexplainable after the fact.
 */
export interface ExposureNote {
  exposed: string[]
  namespaces: string[]
  /** Tools held ahead of namespace filling because the model named them. */
  pinned: string[]
  budget: number | null
  plan_step: string | null
}

function strList(v: unknown): string[] {
  return Array.isArray(v) ? v.filter((x): x is string => typeof x === 'string') : []
}

/**
 * The most recent exposure note, or `null` when the run has emitted none.
 *
 * The *latest*, because the tab's heading is "exposed this step" — an exposure
 * from four steps ago answered under that heading would be a wrong answer rather
 * than a stale one.
 */
/**
 * One answered `search_tools` call, read off the stream.
 *
 * S-6.11's panel has a "searched for" section and `Conversation.tsx` handed it a
 * literal `[]`, so it never rendered — and the note it needed was on the wire
 * the whole time. `drive.rs` answers `search_tools` itself and pushes
 * `Note::CoreTool { name: "search_tools", detail }`, which `local_driver.rs`
 * appends under kind `core_tool`.
 *
 * Not the exposure note's `pinned`, which was the nearest-looking field: pinned
 * tools are ones the model *named* and that were held ahead of namespace
 * filling. A tool can be pinned without ever having been searched for, and a
 * search that matched nothing pins nothing — so labelling `pinned` as
 * "searched for" would have reported the opposite of the case the harness's own
 * comment calls out, "a tool the model had just searched for losing its slot to
 * an alphabetical sibling".
 */
export interface ToolSearch {
  step: number
  /** What was searched for, or `null` when the note's shape was not recognised. */
  query: string | null
  /**
   * What the search returned.
   *
   * `[]` is a search that matched nothing — a real and useful answer, and the
   * one that explains a model going round in circles. `null` is "this note did
   * not parse", which is a fact about the desk rather than about the run, and
   * the two must not collapse.
   */
  found: string[] | null
  /** The note verbatim, so an unrecognised shape is still readable. */
  detail: string
}

// `format!("{query:?} -> {}", …)` and `format!("no tool matches {query:?}")` in
// `drive.rs`. `{:?}` on a `&str` is a quoted, backslash-escaped string.
//
// **Three arrows, not one.** The note is a rendered string rather than a struct,
// so its spelling is whatever the harness build that wrote the row used — and
// this platform's `agent_messages` holds both. Two of the six `search_tools`
// notes stored on it are written with `→` (U+2192) by an older build and four
// with the ASCII `->` the current `drive.rs` emits. A parser that knows only
// today's spelling reports a third of the recorded searches as
// **"a search this panel could not read"** — which is a statement about the desk
// dressed as a statement about the run, on the one section that explains a model
// going round in circles. Old rows do not get rewritten, so the reader has to
// accept every spelling that was ever written.
const ARROW = String.raw`(?:->|=>|→|⟶|⇒)`
const SEARCH_HIT = new RegExp(String.raw`^"((?:[^"\\]|\\.)*)"\s*${ARROW}\s*(.*)$`)
const SEARCH_MISS = /^no tool matches "((?:[^"\\]|\\.)*)"$/

function unquote(s: string): string {
  return s.replace(/\\(.)/g, '$1')
}

/** Every `search_tools` note on the stream, oldest first. */
export function toolSearches(events: WireEvent[]): ToolSearch[] {
  const out: ToolSearch[] = []
  for (const e of events) {
    if (e.kind !== 'core_tool') continue
    const c = e.content
    // Two spellings, because the note is serialised one way into
    // `agent_messages` (`{note, name, detail}`) and another into the trajectory
    // (`{core_tool, detail}`), and this reads whichever arrives.
    const name = str(c.name) ?? str(c.core_tool)
    if (name !== 'search_tools') continue
    const detail = str(c.detail) ?? ''
    const step = num(c.step) ?? 0
    const miss = SEARCH_MISS.exec(detail)
    if (miss) {
      out.push({ step, query: unquote(miss[1]), found: [], detail })
      continue
    }
    const hit = SEARCH_HIT.exec(detail)
    if (hit) {
      out.push({
        step,
        query: unquote(hit[1]),
        found: hit[2]
          .split(',')
          .map((t) => t.trim())
          .filter((t) => t.length > 0),
        detail,
      })
      continue
    }
    out.push({ step, query: null, found: null, detail })
  }
  return out
}

export function latestExposure(events: WireEvent[]): ExposureNote | null {
  for (let i = events.length - 1; i >= 0; i -= 1) {
    const e = events[i]
    if (e.kind !== 'exposure') continue
    const c = e.content
    return {
      exposed: strList(c.exposed),
      namespaces: strList(c.namespaces),
      pinned: strList(c.pinned),
      budget: num(c.budget),
      plan_step: str(c.plan_step),
    }
  }
  return null
}

export function readLayers(v: unknown): CallLayers | null {
  if (!v || typeof v !== 'object') return null
  const o = v as Record<string, unknown>
  const [charter, schemas, body, output] = [o.charter, o.schemas, o.body, o.output].map(num)
  if (charter === null || schemas === null || body === null || output === null) return null
  return { charter, schemas, body, output }
}
