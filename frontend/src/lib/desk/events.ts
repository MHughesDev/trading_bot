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
