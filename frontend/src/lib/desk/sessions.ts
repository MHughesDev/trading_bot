// Reading a continuous session for desk home and the journal (AGENT-014 §9;
// AGENT-012 §2, §2b; task S-11.13).
//
// Pure functions over the two served reads — `GET /api/research/sessions/{id}`
// and `…/{id}/agenda` — so the rows on desk home and the header in the journal
// say the same thing about the same session.
import { sessionDigestView, type AgendaItem, type SessionDetail, type SessionDigestRow } from '@/api/desk'

/** The folded lifecycle state, as the detail route serialises `SessionState`. */
export type SessionStateView =
  | { kind: 'draft' | 'running' | 'complete' | 'stopped' }
  | { kind: 'paused'; why: string | null }
  | { kind: 'stalled'; diagnosis: string | null }
  | { kind: 'unknown' }

/**
 * `SessionState` is `#[serde(tag = "kind")]`, so a stalled session arrives as
 * `{kind: "stalled", diagnosis: "…"}`. The list route sends the bare word. Both
 * are read; anything else — including `null`, a log the fold could not replay —
 * is `unknown`, never `running`.
 */
export function sessionState(raw: unknown): SessionStateView {
  const word =
    typeof raw === 'string'
      ? raw
      : raw && typeof raw === 'object' && typeof (raw as { kind?: unknown }).kind === 'string'
        ? (raw as { kind: string }).kind
        : null
  const obj = (raw && typeof raw === 'object' ? raw : {}) as Record<string, unknown>
  switch (word) {
    case 'draft':
    case 'running':
    case 'complete':
    case 'stopped':
      return { kind: word }
    case 'paused':
      return { kind: 'paused', why: typeof obj.why === 'string' ? obj.why : null }
    case 'stalled':
      return { kind: 'stalled', diagnosis: typeof obj.diagnosis === 'string' ? obj.diagnosis : null }
    default:
      return { kind: 'unknown' }
  }
}

/**
 * The item the session is most likely to claim next, and how sure that is.
 *
 * The scorer decides (AGENT-014 §5) and nothing serves its pick ahead of time,
 * so this is a reading of the board rather than a claim about the driver: a
 * `user` item is claimed first whatever the score says (§4.2); otherwise the
 * highest scored queued item; otherwise, with nothing scored, the board cannot
 * say — and saying "the first one" would be inventing an order the scorer has
 * not given.
 */
export function nextItem(
  items: AgendaItem[] | null | undefined,
): { item: AgendaItem; why: 'yours' | 'highest score' } | { item: null; why: 'nothing queued' | 'unscored' | 'unknown' } {
  if (!items) return { item: null, why: 'unknown' }
  const queued = items.filter((i) => i.state === 'queued')
  if (queued.length === 0) return { item: null, why: 'nothing queued' }
  const mine = queued.find((i) => i.bypasses_scoring)
  if (mine) return { item: mine, why: 'yours' }
  const scored = queued.filter((i) => i.score !== null)
  if (scored.length === 0) return { item: null, why: 'unscored' }
  const best = scored.reduce((a, b) => ((b.score?.total ?? -Infinity) > (a.score?.total ?? -Infinity) ? b : a))
  return { item: best, why: 'highest score' }
}

/** Items neither finished nor vetoed, from the board itself. */
export function agendaDepth(items: AgendaItem[] | null | undefined): number | null {
  if (!items) return null
  return items.filter((i) => i.state === 'proposed' || i.state === 'queued' || i.state === 'running').length
}

/** The day budget the charter set, or `null` when the detail did not carry one. */
export function dayBudget(detail: SessionDetail | null | undefined): number | null {
  const v = detail?.budget?.usd_per_day
  return typeof v === 'number' && Number.isFinite(v) ? v : null
}

/**
 * Dollars spent today, as far as the session's own digests say (S-11.13).
 *
 * No route counts dollars against a day, so the one figure that exists is each
 * digest's `budget.spent` over its period. This sums the digests **written on
 * `now`'s UTC day whose recorded unit is `usd`** and returns `null` otherwise:
 * a digest in trials is not money, one with no unit is not known to be money,
 * and a day with no digest is "not recorded" rather than "nothing spent". The
 * figure is a floor, not a meter: spend since the last digest is not in it,
 * which is why the bar that shows it says it is read from digests.
 */
export function daySpendFromDigests(
  digests: ReadonlyArray<SessionDigestRow> | null | undefined,
  now: Date = new Date(),
): number | null {
  if (!digests) return null
  const today = now.toISOString().slice(0, 10)
  let total: number | null = null
  for (const row of digests) {
    if (typeof row.created_at !== 'string' || row.created_at.slice(0, 10) !== today) continue
    const view = sessionDigestView(row)
    if (!view || view.spend_unit !== 'usd' || !view.budget || !Number.isFinite(view.budget.spent)) continue
    total = (total ?? 0) + view.budget.spent
  }
  return total
}

/**
 * What a session route said when it refused, in its own words.
 *
 * Every refusal on `/api/research/sessions…` is `{error, message, fix}` — the
 * transition trigger's 409 names both states, and a 400 says what to do
 * instead. A control that dropped them would report a refusal as silence.
 */
export function sessionRefusal(e: unknown): string {
  const response = (e as { response?: { status?: number; data?: unknown } }).response
  const data = (response?.data ?? {}) as Record<string, unknown>
  const message = typeof data.message === 'string' ? data.message : null
  const fix = typeof data.fix === 'string' ? data.fix : null
  if (message) return fix ? `${message} — ${fix}` : message
  if (typeof response?.status === 'number') return `The session route answered ${response.status} and said nothing else.`
  return 'The request never reached the platform. Nothing was changed.'
}

/**
 * The journal's slash commands (§2b): `/agenda add <question>`, `/pause`,
 * `/resume`, `/charter`. Parsed here so the composer and the journal agree
 * about what each one means; `null` is a command this is not.
 */
export type JournalCommand =
  | { kind: 'agenda_add'; question: string }
  | { kind: 'agenda_add_empty' }
  | { kind: 'pause' }
  | { kind: 'resume' }
  | { kind: 'charter' }

export function journalCommand(cmd: string, rest: string): JournalCommand | null {
  switch (cmd) {
    case '/agenda': {
      const [sub, ...words] = rest.trim().split(/\s+/)
      if (sub !== 'add') return null
      const question = words.join(' ').trim()
      return question ? { kind: 'agenda_add', question } : { kind: 'agenda_add_empty' }
    }
    case '/pause':
      return { kind: 'pause' }
    case '/resume':
      return { kind: 'resume' }
    case '/charter':
      return { kind: 'charter' }
    default:
      return null
  }
}

/** What a slash command typed in a conversation does, decided in one place. */
export type SlashAction =
  | { kind: 'steer'; text: string }
  | { kind: 'tab'; tab: 'cost' | 'charter' }
  | { kind: 'session'; action: 'pause' | 'resume' }
  | { kind: 'run'; action: 'pause' | 'resume' }
  | { kind: 'agenda_add'; question: string }
  | { kind: 'refuse'; why: string }

/**
 * Every command `SLASH_COMMANDS` lists, mapped to what it does here.
 *
 * `continuous` is whether this conversation is a continuous session's journal;
 * `hasRun` whether it has a run a pause could reach. A command that cannot act
 * in this conversation is **refused with the reason**, never sent somewhere
 * else: `/agenda add` in an ordinary chat has no agenda to add to, and quietly
 * steering the run with the text instead would be the composer deciding what
 * the person meant.
 */
export function slashAction(
  cmd: string,
  rest: string,
  ctx: { continuous: boolean; hasRun: boolean },
): SlashAction | null {
  switch (cmd) {
    case '/steer':
      return rest.trim() ? { kind: 'steer', text: rest.trim() } : { kind: 'refuse', why: '/steer needs the text to steer with.' }
    case '/why':
      return { kind: 'steer', text: 'why did you do that?' }
    case '/budget':
      return { kind: 'tab', tab: 'cost' }
  }
  const j = journalCommand(cmd, rest)
  if (!j) return null
  switch (j.kind) {
    case 'pause':
    case 'resume':
      if (ctx.continuous) return { kind: 'session', action: j.kind }
      if (ctx.hasRun) return { kind: 'run', action: j.kind }
      return { kind: 'refuse', why: `${cmd} has nothing to act on: this conversation has no run and is not a session.` }
    case 'charter':
      return ctx.continuous
        ? { kind: 'tab', tab: 'charter' }
        : { kind: 'refuse', why: '/charter belongs to a continuous session; this conversation has no charter.' }
    case 'agenda_add':
      return ctx.continuous
        ? { kind: 'agenda_add', question: j.question }
        : { kind: 'refuse', why: '/agenda add belongs to a continuous session; this conversation has no agenda.' }
    case 'agenda_add_empty':
      return { kind: 'refuse', why: '/agenda add needs the question to add, e.g. /agenda add does the gap fade on ETH?' }
  }
}
