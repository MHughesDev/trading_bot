// Usage, monitor and the strategy editor (AGENT-012 §8, §11, §12; task S-6.21).
//
// Three surfaces that share one property: they are the operator's, not the
// researcher's. Each is wired against the endpoint its owning phase will provide
// and says plainly when that phase has not landed, rather than rendering a
// placeholder number somebody might believe.
import { useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Badge, type BadgeTone } from '@/components/primitives/Badge'
import { Button } from '@/components/primitives/Button'
import { Modal } from '@/components/primitives/Modal'
import { Panel, PanelBody, PanelHeader } from '@/components/primitives/Panel'
import { Field, Input, TextArea } from '@/components/primitives/Field'
import { EmptyState } from '@/components/primitives/States'
import { api } from '@/lib/api'
import {
  deskApi,
  type HaltPolicy,
  type StrategyAction,
  type StrategyActionBody,
} from '@/api/desk'

// ── What the runs produced (AGENT-013 §10; S-10.13) ──────────────────────────
//
// The one number here that is not about whether the harness *worked*. A barren
// run finished perfectly: it spent from the trial budget — the quantity every
// selection-adjusted statistic on this platform depends on — and produced
// nothing the next run can use.
//
// Rendered as rows and never as a single rate. "12% barren" is something nobody
// can act on; "`survey` on the 7B is barren 60% of the time" names a tier and a
// workflow, so somebody can change one of them.

/** `null` and `0` are different answers, and only one of them is a measurement. */
export function formatRate(rate: number | null, runs: number): string {
  if (rate === null || runs === 0) return 'not measured'
  return `${Math.round(rate * 100)}%`
}

function BarrenPanel() {
  const q = useQuery({
    queryKey: ['desk', 'barren'],
    queryFn: () => deskApi.barren(30),
  })
  const d = q.data

  return (
    <Panel>
      <PanelHeader title="What the runs produced" />
      <PanelBody>
        {d === null && (
          <EmptyState
            message="Not available"
            detail="/api/lifecycle/barren is served, so this is the endpoint failing rather than the endpoint missing."
          />
        )}
        {d && d.rows.length === 0 ? (
          <EmptyState
            message="Nothing measured"
            detail={`No run finished in the last ${d.days} days, so there is no rate — which is not the same as a rate of zero.`}
          />
        ) : (
          d && (
            <table className="tbl">
              <thead>
                <tr>
                  <th style={{ textAlign: 'left' }}>Tier</th>
                  <th style={{ textAlign: 'left' }}>Workflow</th>
                  <th>Runs</th>
                  <th>Barren</th>
                  <th>Rate</th>
                </tr>
              </thead>
              <tbody>
                {d.rows.map((r) => (
                  <tr key={`${r.tier}|${r.workflow}`}>
                    <td style={{ textAlign: 'left' }}>{r.tier}</td>
                    <td style={{ textAlign: 'left' }}>{r.workflow}</td>
                    {/* The denominator travels with the rate: 60% over five runs
                        and over five hundred are different claims. */}
                    <td>{r.runs}</td>
                    <td>{r.barren}</td>
                    <td>{formatRate(r.rate, r.runs)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          )
        )}
        {d && d.unresolved.length > 0 && (
          <p className="sub">
            {/* Named rather than folded into a guessed tier or dropped: folding
                attributes one tier's waste to another, and dropping makes the
                platform look less wasteful the more configurations it retires. */}
            Not attributed to a tier — the platform no longer ships these profiles:{' '}
            {d.unresolved.map((u) => `${u.profile} (${u.runs})`).join(', ')}
          </p>
        )}
        <p className="sub">
          A barren run spent a trial and produced no knowledge unit. Over the last {d?.days ?? 30} days.
        </p>
      </PanelBody>
    </Panel>
  )
}

// ── Usage (§12) ──────────────────────────────────────────────────────────────

/** One line of the spend table, already decided and already in words. */
export interface UsageLine {
  key: string
  label: string
  /** What to print. Never a `0` standing in for a number nobody has. */
  value: string
}

const USAGE_LABELS: Record<string, string> = {
  input_tokens: 'prompt tokens',
  output_tokens: 'output tokens',
  cache_read: 'cache reads',
  cache_write: 'cache writes',
  cost_usd: 'cost',
  cache_hit_rate: 'cache hit rate',
  verdicts: 'verdicts filed',
  dollars_per_verdict: 'cost per verdict',
}

function numberOrNull(v: unknown): number | null {
  return typeof v === 'number' && Number.isFinite(v) ? v : null
}

/** The four counters `/api/agent/usage` sums straight out of `llm_usage`. */
const LLM_USAGE_COUNTERS = ['input_tokens', 'output_tokens', 'cache_read', 'cache_write'] as const

/**
 * Whether `llm_usage` recorded anything at all for this desk.
 *
 * `get_usage` is four `COALESCE(SUM(…),0)` over one table. Every one of them
 * coming back zero has exactly two readings — the desk spent nothing, or the
 * table holds no row for it — and the response cannot tell them apart, because
 * both produce the same four zeros.
 *
 * On this platform today it is the second reading: `llm_usage` holds **0 rows**
 * while `agent_runs` holds 55, 27 of which recorded a non-zero `tokens_in`, for
 * 2,526,695 prompt tokens and 27,512 output tokens in a column the endpoint does
 * not read. Only the LLM proxy writes `llm_usage`, and the desk driver does not
 * go through it.
 *
 * So this returning `true` does not license the surface to claim a number. It
 * licenses it to stop printing `0`, which is the one thing that is certainly
 * wrong: a zero here is *not recorded*, and the reader has no way to know that
 * from the digit.
 *
 * A single counter of zero beside a non-zero one is a real measured zero and is
 * printed as `0`, which is why this is all-four and not any-of-four.
 */
export function nothingRecordedInLlmUsage(raw: Record<string, unknown> | null | undefined): boolean {
  if (!raw) return false
  return LLM_USAGE_COUNTERS.every((k) => numberOrNull(raw[k]) === 0)
}

/** What a counter is allowed to say when the table behind it recorded nothing. */
const NOT_RECORDED =
  'not recorded — llm_usage holds nothing for this desk, and only the LLM proxy writes it. Not a measured zero.'

/**
 * The spend totals, read so that three of them stop lying.
 *
 * `Object.entries(u)` printed the response verbatim, and the response fills
 * three fields with a zero that means "nobody has this number":
 *
 * * `cost_usd` is `COALESCE(SUM(cost_usd),0)`, and the platform has **no price
 *   table** (OQ-16). The Cost tab six tabs away already says "not priced… this
 *   is not $0"; this page rendered a cost of 0 beside half a million tokens.
 * * `cache_hit_rate` is `0.0` when nothing billable has been sent — an
 *   unmeasured rate as a measured zero — and it is a fraction printed bare in a
 *   column of token counts, so `0.35` read as thirty-five of something.
 * * `dollars_per_verdict` is `null` with no verdicts, and `String(null)` put the
 *   literal word **null** in the table.
 *
 * And a fourth, added once the live platform was read: the four token counters
 * fell through to `n.toLocaleString()`, so an `llm_usage` with no rows rendered
 * as four honest-looking `0`s. See `nothingRecordedInLlmUsage`.
 *
 * Returns `null` when nothing answered, which the caller must not confuse with a
 * request still in flight.
 */
export function usageRows(raw: Record<string, unknown> | null | undefined): UsageLine[] | null {
  if (!raw) return null
  const input = numberOrNull(raw.input_tokens)
  const output = numberOrNull(raw.output_tokens)
  const cacheRead = numberOrNull(raw.cache_read)
  const verdicts = numberOrNull(raw.verdicts)
  const spentTokens = (input ?? 0) + (output ?? 0) > 0
  const billablePrompt = (input ?? 0) + (cacheRead ?? 0)
  const unrecorded = nothingRecordedInLlmUsage(raw)

  return Object.keys(raw).map((key): UsageLine => {
    const v = raw[key]
    const label = USAGE_LABELS[key] ?? key
    const n = numberOrNull(v)
    switch (key) {
      case 'input_tokens':
      case 'output_tokens':
      case 'cache_read':
      case 'cache_write':
        // A count is the one place a zero is invisible: `0` reads as a
        // measurement whichever it is. When the whole table came back empty
        // there is no measurement, and saying so is the only honest render.
        if (n === null) return { key, label, value: 'not reported' }
        return { key, label, value: unrecorded ? NOT_RECORDED : n.toLocaleString() }
      case 'cost_usd':
        if (n === null) return { key, label, value: 'not reported' }
        // Nothing to price, because nothing was recorded to price. `$0.0000`
        // here is the same claim the bare `0`s made.
        if (unrecorded) return { key, label, value: NOT_RECORDED }
        if (n === 0 && spentTokens) {
          return {
            key,
            label,
            value: 'not priced — the platform has no price table (OQ-16). This is not zero.',
          }
        }
        return { key, label, value: `$${n.toFixed(4)}` }
      case 'cache_hit_rate':
        if (n === null) return { key, label, value: 'not reported' }
        // "no prompt has been billed yet" is a claim about billing. With an
        // empty table nobody knows whether a prompt was billed — only that no
        // row says so.
        if (unrecorded) return { key, label, value: NOT_RECORDED }
        if (billablePrompt === 0) {
          return { key, label, value: 'not measured — no prompt has been billed yet' }
        }
        return { key, label, value: `${(n * 100).toFixed(1)}%` }
      case 'dollars_per_verdict':
        if (n !== null) return { key, label, value: `$${n.toFixed(4)}` }
        return {
          key,
          label,
          value: verdicts === 0 ? 'no verdict has been filed — not zero' : 'not reported',
        }
      default:
        return { key, label, value: n === null ? String(v) : n.toLocaleString() }
    }
  })
}

export function Usage() {
  const usage = useQuery({
    queryKey: ['desk', 'usage'],
    queryFn: () => deskApi.usage().then((r) => r.data),
  })
  const raw = usage.data as Record<string, unknown> | undefined
  const rows = usageRows(raw)
  const unrecorded = nothingRecordedInLlmUsage(raw)

  return (
    <div className="desk-scroll">
      <Panel>
        <PanelHeader title="Spend" />
        <PanelBody>
          {/* A request in flight is not an answer of nothing. The old copy
              asserted "/api/agent/usage answered with nothing" from the first
              render, before it had been asked. */}
          {usage.isPending ? (
            <p className="sub">Reading the spend totals…</p>
          ) : rows === null ? (
            <EmptyState
              message="The spend totals did not answer"
              detail="/api/agent/usage is served, so this is the request failing rather than the endpoint missing. What this desk has spent is unknown, not zero."
            />
          ) : (
            <table className="tbl">
              <tbody>
                {rows.map((r) => (
                  <tr key={r.key}>
                    <td style={{ textAlign: 'left' }}>{r.label}</td>
                    <td>{r.value}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
          {unrecorded && (
            // Stated, not left to be inferred from four zeros. The reader
            // cannot tell an empty table from an idle desk by looking at the
            // numbers, so the surface has to say which question is open.
            <p className="sub" data-testid="usage-unrecorded-note">
              Every total <code>/api/agent/usage</code> sums came back zero. That endpoint reads one table,{' '}
              <code>llm_usage</code>, which only the LLM proxy writes — and this desk's driver does not call
              the agent through it. So this desk may have spent nothing, or it may have spent a great deal
              that nothing recorded, and these totals cannot tell you which. Treat them as unread, not as
              nil. The run's own <code>tokens_in</code>/<code>tokens_out</code> are kept on{' '}
              <code>agent_runs</code>, which no endpoint exposes yet.
            </p>
          )}
          <p className="sub">
            Spend by day, by conversation, by model and by layer, cost per finding and per gate, and budget
            policies come from AGENT-009's published numbers and the per-layer accounting in AGENT-005. Neither
            is exposed by an endpoint yet, so the desk shows the totals that are. What the runs
            *produced* is below, from `/api/lifecycle/barren`.
          </p>
          <p className="sub">
            {/* Stated rather than left for somebody to deduce from a permanent
                zero: the count is over `agent_sessions` that filed a report, and
                the desk's conversations are `agent_runs`. Cost per verdict is
                therefore not a number this surface can produce today. */}
            `verdicts` counts sessions that filed a report. The desk's conversations are runs, not
            sessions, so this stays at zero however many dossiers the desk compiles — a gap in the
            endpoint, not a desk that has concluded nothing.
          </p>
          <Button
            size="xs"
            variant="secondary"
            disabled={rows === null}
            onClick={() => {
              if (rows === null) return
              // The rendered values, not the raw ones. A CSV carrying a cost of
              // 0 makes the same claim the table used to, somewhere nobody can
              // see the caveat beside it.
              const csv = [
                'metric,value',
                ...rows.map((r) => `${r.label},"${r.value.replace(/"/g, '""')}"`),
              ].join('\n')
              const url = URL.createObjectURL(new Blob([csv], { type: 'text/csv' }))
              const a = document.createElement('a')
              a.href = url
              a.download = 'desk-usage.csv'
              a.click()
              URL.revokeObjectURL(url)
            }}
          >
            Export CSV
          </Button>
        </PanelBody>
      </Panel>

      <BarrenPanel />
    </div>
  )
}

// ── Monitor (§11 — the UI of COMP-007) ───────────────────────────────────────

/**
 * What the kill-switch badge may claim, given what answered.
 *
 * `tripped` and `armed` are both claims about live trading, and the panel used
 * to derive them from `kill.data?.kill_switch_active === true` — so an endpoint
 * that did not answer rendered a green **armed**. That is the platform's one
 * non-negotiable pointed at the control that stops money moving: an unknown
 * state shown as the safe one. Absent is a third word here, not a default to
 * either of the other two.
 */
export function killSwitchBadge(
  status: { kill_switch_active?: boolean } | null | undefined,
): { word: string; tone: 'pos' | 'neg' | 'warn' } {
  if (!status) return { word: 'state unknown', tone: 'warn' }
  return status.kill_switch_active === true
    ? { word: 'tripped', tone: 'neg' }
    : { word: 'armed', tone: 'pos' }
}

/**
 * One row of `/api/monitor/strategies`, read under the names it is served with.
 *
 * The table read `name`, `status` and `breaching_metric`; the route sends
 * `strategy_id`, `state` and `scale_pct`. Every row therefore rendered as two
 * blanks and a dash — a monitor showing a table of empty rows for live
 * strategies, which looks like a monitor with nothing to report. The same class
 * of defect as `cited` against `cited_by_chain`: no error, no clue, and the
 * wrong reading is the reassuring one.
 */
export interface MonitorRow {
  strategy_id: string
  state: string | null
  /**
   * `null` when the row carries no scale.
   *
   * Never folded into `0`: a strategy scaled to 0% has been shut down by the
   * monitor, and one whose scale was not recorded has not. Those are opposite
   * facts and they were one `?? 0` apart.
   */
  scale_pct: number | null
  /**
   * Which environment this strategy's orders were executed in, from
   * `orders.execution_mode` — a recorded fact per order, never the strategy's
   * name read as an environment.
   *
   * `null` is **unknown, not paper**: the strategy has placed no order, so
   * nothing anywhere says where its orders would go. `mixed` is a strategy with
   * orders in more than one mode, which the route refuses to collapse to one
   * word because both answers are true.
   */
  environment: string | null
}

export function monitorRow(raw: Record<string, unknown>): MonitorRow {
  const scale = raw.scale_pct
  return {
    strategy_id:
      typeof raw.strategy_id === 'string' ? raw.strategy_id : String(raw.strategy_id ?? 'unnamed'),
    state: typeof raw.state === 'string' ? raw.state : null,
    scale_pct: typeof scale === 'number' && Number.isFinite(scale) ? scale : null,
    environment:
      typeof raw.environment === 'string' && raw.environment ? raw.environment : null,
  }
}

/**
 * The environment, as a badge, with the unknown one looking like neither.
 *
 * Three distinct readings, and the third is the one this exists for:
 *
 * * `paper` — recorded, and today the only mode the `orders_execution_mode_
 *   paper_only` CHECK admits. Quiet, because it is the safe answer.
 * * `live` / anything else the column starts holding — loud, because the day
 *   that CHECK widens this word means real money and no code changes here.
 * * `mixed` — orders in more than one mode; the route will not pick one.
 * * `null` — **no order has ever been placed.** Not paper. A strategy that has
 *   never traded rendered as paper is the platform answering a question nobody
 *   asked the database, and on this page it is the difference between "safe"
 *   and "nobody knows".
 */
export function environmentBadge(environment: string | null): { word: string; tone: BadgeTone } {
  if (environment === null) return { word: 'no orders — environment unknown', tone: 'outline' }
  if (environment === 'paper') return { word: 'paper', tone: 'neutral' }
  if (environment === 'mixed') return { word: 'mixed — orders in both', tone: 'warn' }
  if (environment === 'live') return { word: 'live', tone: 'neg' }
  // A word this build does not know is shown as it was sent rather than folded
  // into one of the four above: guessing is how a new mode reads as an old one.
  return { word: environment, tone: 'warn' }
}

// ── The per-row controls (§11; COMP-007 §6) ──────────────────────────────────
//
// `Scale down` was a `<Button>` with no `onClick`. It rendered, it took the
// hover, it took the click, and `POST /api/monitor/strategies/{id}/{action}` had
// been served since COMP-007 landed. A control that looks like a control and
// reaches nothing is worse on this page than no control at all: the operator
// walks away believing a losing strategy has been scaled back.

/**
 * The scope string the reset gate compares against, byte for byte.
 *
 * `Scope::Strategy(id)` displays as `strategy:<id>` and `ResetRequest::validate`
 * refuses on `self.typed_confirmation != expected`. The word to type is
 * therefore not ours to choose, and a friendlier one would produce a dialog that
 * always ends in a 409.
 */
export function strategyScope(strategyId: string): string {
  return `strategy:${strategyId}`
}

export interface StrategyActionInput {
  strategyId: string
  reason: string
  scalePct?: number | null
  haltPolicy?: HaltPolicy
  /** What the person typed into the confirmation box, for `resume`. */
  typed?: string
}

export type BuiltAction =
  | { ok: true; body: StrategyActionBody }
  | { ok: false; refusal: string }

/**
 * The body for one action, refusing here exactly what the route refuses there.
 *
 * Duplicated deliberately, and stated as a refusal the person reads *before*
 * the request rather than a 400 they read after it. The three actions need
 * three different subsets of the same optional-everywhere body, which is the
 * shape where a missing field becomes a server error nobody can act on —
 * `scale_pct is required`, `a halt requires a reason`, `resume_refused`.
 */
export function strategyActionBody(action: StrategyAction, input: StrategyActionInput): BuiltAction {
  const reason = input.reason.trim()
  switch (action) {
    case 'scale': {
      const pct = input.scalePct
      if (typeof pct !== 'number' || !Number.isInteger(pct)) {
        return { ok: false, refusal: 'Give a whole percentage between 1 and 100.' }
      }
      // The route's own bound. 0 is not "off" here — halting is a different
      // action with a different record, and `scale_pct must be in 1..=100`.
      if (pct < 1 || pct > 100) {
        return {
          ok: false,
          refusal: 'The monitor takes 1–100%. To stop a strategy entirely, halt it — that is a different record.',
        }
      }
      return { ok: true, body: reason ? { scale_pct: pct, reason } : { scale_pct: pct } }
    }
    case 'halt': {
      if (!reason) {
        return { ok: false, refusal: 'A halt needs a reason. The monitor refuses one without it.' }
      }
      return { ok: true, body: { reason, halt_policy: input.haltPolicy ?? 'hold' } }
    }
    case 'resume': {
      if (!reason) {
        return { ok: false, refusal: 'A resume is a reset, and a reset needs a reason.' }
      }
      const expected = strategyScope(input.strategyId)
      // Case-sensitive, because `ResetRequest::validate` is. `ConfirmDialog`
      // compares its own word case-insensitively, which would have let a
      // mismatch through to a 409 the person could not explain.
      if ((input.typed ?? '') !== expected) {
        return { ok: false, refusal: `Type ${expected} exactly — the monitor compares it literally.` }
      }
      return { ok: true, body: { reason, confirm_scope: expected } }
    }
  }
}

/**
 * What the monitor said when it refused, in words.
 *
 * Every branch of `strategy_action` can refuse for a reason the person can act
 * on — no monitor-control scope on this session, a reset inside the rate limit,
 * a reason left out — and a control that swallows those reports failure as
 * silence, which reads as success.
 */
export function strategyActionRefusal(e: unknown): string {
  const response = (e as { response?: { status?: number; data?: unknown } }).response
  const data = (response?.data ?? {}) as Record<string, unknown>
  const detail = typeof data.detail === 'string' ? data.detail : null
  const code = typeof data.error === 'string' ? data.error : null
  if (detail) return code ? `${code} — ${detail}` : detail
  if (code) return code
  if (typeof response?.status === 'number') {
    return `The monitor answered ${response.status} and said nothing else. Nothing was changed.`
  }
  return 'The request never reached the monitor. Nothing was changed.'
}

/**
 * The global scope's confirmation word, which is not ours to choose either.
 *
 * `Scope::Global` displays as the bare word `global`, and the same
 * `ResetRequest::validate` compares it literally.
 */
export const GLOBAL_SCOPE = 'global'

export type GlobalAction = 'halt' | 'resume'

export type BuiltGlobal =
  | { ok: true; action: 'halt'; reason: string }
  | { ok: true; action: 'resume'; body: { confirm_scope: string; reason: string } }
  | { ok: false; refusal: string }

/**
 * What the global halt and its reset need before the request is worth sending.
 *
 * `Halt all` used to be a `ConfirmDialog` typing `HALT` and then
 * `void api.post('/api/trading/kill')` — no reason, so the monitor's event log
 * recorded `manual kill from /api/trading/kill` against a human's name, and no
 * error handling, so a session without `permits_monitor_control` got a 403 that
 * reached nobody. `Reset (logged)` was worse: `reset_kill_switch` takes
 * `Json<ResumeBody>` by value and the call sent no body, so the extractor
 * rejected it every time. The button had never once reset anything.
 *
 * The word for the halt is the scope's, not a friendlier one, so the two
 * controls on this panel ask for the same thing.
 */
export function globalActionBody(
  action: GlobalAction,
  input: { reason: string; typed: string },
): BuiltGlobal {
  const reason = input.reason.trim()
  if (!reason) {
    return {
      ok: false,
      refusal:
        action === 'halt'
          ? 'A halt needs a reason. It is written into the monitor log against your name.'
          : 'A resume is a reset, and a reset needs a reason.',
    }
  }
  if (input.typed !== GLOBAL_SCOPE) {
    return {
      ok: false,
      refusal: `Type ${GLOBAL_SCOPE} exactly — the monitor compares it literally.`,
    }
  }
  return action === 'halt'
    ? { ok: true, action: 'halt', reason }
    : { ok: true, action: 'resume', body: { confirm_scope: GLOBAL_SCOPE, reason } }
}

/**
 * The two-step confirm for the global halt and the global reset (Meridian §5.5).
 *
 * Prop-driven for the same reason `StrategyActionDialog` is: the refusal rules
 * are the part worth testing and they should not need a query client to reach.
 */
export function GlobalActionDialog({
  action,
  halted,
  busy = false,
  error = null,
  onCancel,
  onConfirm,
}: {
  action: GlobalAction | null
  /** The scopes the monitor says are halted, or `null` when it did not answer. */
  halted: string[] | null
  busy?: boolean
  error?: string | null
  onCancel: () => void
  onConfirm: (built: BuiltGlobal & { ok: true }) => void
}) {
  const [reason, setReason] = useState('')
  const [typed, setTyped] = useState('')

  if (action === null) return null
  const built = globalActionBody(action, { reason, typed })

  // A global reset is refused outright while any venue is still halted
  // (`ResetRefusal::VenuesNotReconciled`). Said here rather than discovered as a
  // 409, and said as "unknown" when the scope read did not answer — the desk
  // does not get to promise this will go through on a list it has not seen.
  const venueNote =
    action === 'resume'
      ? halted === null
        ? 'The halted scopes could not be read, so whether a venue still blocks this reset is unknown.'
        : halted.some((s) => s.startsWith('venue:'))
          ? `The monitor will refuse while a venue is halted: ${halted
              .filter((s) => s.startsWith('venue:'))
              .join(', ')}.`
          : null
      : null

  return (
    <Modal
      open
      onClose={onCancel}
      dismissible={false}
      title={action === 'halt' ? 'Halt all trading?' : 'Reset the global kill switch?'}
      footer={
        <>
          <span className="spacer" />
          <Button onClick={onCancel}>Cancel</Button>
          <Button
            variant={action === 'halt' ? 'danger' : 'primary'}
            disabled={!built.ok || busy}
            loading={busy}
            data-testid="global-action-confirm"
            onClick={() => built.ok && onConfirm(built)}
          >
            {action === 'halt' ? 'Halt all' : 'Reset'}
          </Button>
        </>
      }
    >
      <div className="callout neg" style={{ marginBottom: 'var(--s-4)' }} data-testid="global-action-consequence">
        {action === 'halt'
          ? 'Every strategy stops placing orders immediately, on every venue. Open positions are held, not closed. Only a reset — which is rate-limited and typed — undoes it.'
          : 'Order flow resumes across every scope that is not separately halted. The reset is rate-limited, logged against your name, and refused if another reset was made too recently.'}
      </div>
      {venueNote && (
        <p className="sub" data-testid="global-venue-note">
          {venueNote}
        </p>
      )}
      <Field label="Reason (required)">
        <TextArea rows={2} aria-label="Reason" value={reason} onChange={(e) => setReason(e.target.value)} />
      </Field>
      <Field label={`Type ${GLOBAL_SCOPE} to confirm`}>
        <Input aria-label="Typed confirmation" value={typed} onChange={(e) => setTyped(e.target.value)} />
      </Field>
      {!built.ok && (
        <p className="sub" data-testid="global-action-refusal">
          {built.refusal}
        </p>
      )}
      {error && (
        <p className="sub" data-testid="global-action-error" role="alert">
          The monitor refused: {error}
        </p>
      )}
    </Modal>
  )
}

const ACTION_TITLE: Record<StrategyAction, string> = {
  scale: 'Scale down',
  halt: 'Halt',
  resume: 'Resume',
}

/**
 * The two-step confirm for one strategy action (Meridian §5.5).
 *
 * Presentational and prop-driven: it owns the fields and nothing else, so the
 * refusal rules can be exercised without a query client, a router or a network.
 */
export function StrategyActionDialog({
  action,
  strategyId,
  currentScale,
  busy = false,
  error = null,
  onCancel,
  onConfirm,
}: {
  action: StrategyAction | null
  strategyId: string
  /** What the row says the strategy runs at now, or `null` if nobody said. */
  currentScale: number | null
  busy?: boolean
  error?: string | null
  onCancel: () => void
  onConfirm: (body: StrategyActionBody) => void
}) {
  const [reason, setReason] = useState('')
  const [pct, setPct] = useState('50')
  const [typed, setTyped] = useState('')

  // The caller remounts this per action with a `key`, so the fields start empty
  // every time. An effect that cleared them on close would leave the last
  // reason typed into the next strategy's dialog for one render — and a reason
  // is written into the monitor's event log against a name.
  if (action === null) return null

  const parsed = /^\d+$/.test(pct.trim()) ? Number.parseInt(pct.trim(), 10) : null
  const built = strategyActionBody(action, {
    strategyId,
    reason,
    scalePct: parsed,
    typed,
  })

  const consequence =
    action === 'scale'
      ? `${strategyId} will be recorded as scaled down and run at ${
          parsed === null ? 'the percentage you give' : `${parsed}%`
        } of full size${
          currentScale === null
            ? ' — the monitor has not reported its current scale, so this is not a change from a known number'
            : `, down from ${currentScale}%`
        }. The monitor writes a ScaledDown event against your name.`
      : action === 'halt'
        ? `${strategyId} stops placing orders immediately. Open positions are held, not closed. This trips the kill switch for ${strategyScope(
            strategyId,
          )} and only a resume — which is a reset, rate-limited — undoes it.`
        : `${strategyId} returns to nominal at 100% of full size and may place orders again. This is a reset: it is rate-limited, logged against your name, and refused if another reset was made too recently.`

  return (
    <Modal
      open
      onClose={onCancel}
      dismissible={false}
      title={`${ACTION_TITLE[action]} ${strategyId}?`}
      footer={
        <>
          <span className="spacer" />
          <Button onClick={onCancel}>Cancel</Button>
          <Button
            variant={action === 'resume' ? 'primary' : 'danger'}
            disabled={!built.ok || busy}
            loading={busy}
            data-testid="strategy-action-confirm"
            onClick={() => built.ok && onConfirm(built.body)}
          >
            {ACTION_TITLE[action]}
          </Button>
        </>
      }
    >
      <div className="callout neg" style={{ marginBottom: 'var(--s-4)' }} data-testid="strategy-action-consequence">
        {consequence}
      </div>
      {action === 'scale' && (
        <Field label="Percentage of full size (1–100)">
          <Input
            aria-label="Percentage of full size"
            inputMode="numeric"
            value={pct}
            onChange={(e) => setPct(e.target.value)}
          />
        </Field>
      )}
      <Field
        label={action === 'scale' ? 'Reason (recorded with the event)' : 'Reason (required)'}
      >
        <TextArea
          rows={2}
          aria-label="Reason"
          value={reason}
          onChange={(e) => setReason(e.target.value)}
        />
      </Field>
      {action === 'resume' && (
        <Field label={`Type ${strategyScope(strategyId)} to confirm`}>
          <Input
            aria-label="Typed confirmation"
            value={typed}
            onChange={(e) => setTyped(e.target.value)}
          />
        </Field>
      )}
      {/* The refusal the monitor would give, given before the request rather
          than after it. A disabled button with no reason beside it is a dead
          end the person cannot debug. */}
      {!built.ok && (
        <p className="sub" data-testid="strategy-action-refusal">
          {built.refusal}
        </p>
      )}
      {/* …and the refusal it actually gave, if it did. Never swallowed: a
          failed control that says nothing reads exactly like one that worked. */}
      {error && (
        <p className="sub" data-testid="strategy-action-error" role="alert">
          The monitor refused: {error}
        </p>
      )}
    </Modal>
  )
}

export function Monitor() {
  const [global, setGlobal] = useState<GlobalAction | null>(null)
  const [pending, setPending] = useState<{ action: StrategyAction; row: MonitorRow } | null>(null)
  const qc = useQueryClient()
  const strategies = useQuery({ queryKey: ['desk', 'monitor'], queryFn: () => deskApi.monitor() })
  const kill = useQuery({
    queryKey: ['desk', 'kill-switch'],
    queryFn: () => deskApi.killSwitch(),
    refetchInterval: 3_000,
  })
  const scopes = useQuery({
    queryKey: ['desk', 'monitor', 'kill-switch'],
    queryFn: () => deskApi.monitorKillSwitch(),
    refetchInterval: 3_000,
  })
  const tripped = kill.data?.kill_switch_active === true
  const badge = killSwitchBadge(kill.data)
  const windows = scopes.data?.maintenance_windows ?? []

  const act = useMutation({
    mutationFn: async ({ body }: { body: StrategyActionBody }) => {
      if (!pending) throw new Error('no action is pending')
      return (await deskApi.strategyAction(pending.row.strategy_id, pending.action, body)).data
    },
    onSuccess: () => {
      setPending(null)
      // Re-read rather than patching the row locally: the monitor decides what
      // the state and the scale now are, and a table showing what we *asked*
      // for would show a change a refusal may have undone.
      void qc.invalidateQueries({ queryKey: ['desk', 'monitor'] })
      void qc.invalidateQueries({ queryKey: ['desk', 'kill-switch'] })
    },
  })

  const globalAct = useMutation({
    mutationFn: async (built: BuiltGlobal & { ok: true }) =>
      built.action === 'halt'
        ? (await deskApi.haltAll(built.reason)).data
        : (await deskApi.resumeAll(built.body)).data,
    onSuccess: () => {
      setGlobal(null)
      void qc.invalidateQueries({ queryKey: ['desk', 'kill-switch'] })
      void qc.invalidateQueries({ queryKey: ['desk', 'monitor'] })
      void qc.invalidateQueries({ queryKey: ['desk', 'monitor', 'kill-switch'] })
    },
  })

  return (
    <div className="desk-scroll" style={{ display: 'flex', flexDirection: 'column', gap: 'var(--s-3)' }}>
      <Panel>
        <PanelHeader
          title="Kill switch"
          badges={
            <Badge tone={badge.tone} data-testid="kill-switch-badge">
              {badge.word}
            </Badge>
          }
        />
        <PanelBody>
          <p className="sub">
            {/* `scope` is not on `/api/trading/status` and never has been, so
                the old `?? 'global'` printed a word the endpoint never sent —
                and "global" is the widest possible claim to invent. The halted
                scopes below come from the route that does know. */}
            scope {kill.data?.scope ?? 'not recorded on this endpoint'} · tripped by{' '}
            {kill.data?.tripped_by ?? 'not recorded'} · {kill.data?.reason ?? 'no reason recorded'}
          </p>
          <p className="sub" data-testid="halted-scopes">
            {scopes.data === null || scopes.data === undefined
              ? 'halted scopes unknown — /api/monitor/kill-switch did not answer'
              : scopes.data.halted_scopes.length === 0
                ? 'no scope is halted · orders ' +
                  (scopes.data.orders_permitted ? 'permitted' : 'not permitted') +
                  ' · lease ' +
                  scopes.data.lease
                : `halted: ${scopes.data.halted_scopes.join(', ')} · lease ${scopes.data.lease}`}
          </p>
          {/* §4.3: an open maintenance window is a hole in the limits, and it is
              as visible here as a trip. The desk showed it nowhere. */}
          {windows.length > 0 && (
            <p className="sub" data-testid="maintenance-windows">
              {windows.length} open maintenance{' '}
              {windows.length === 1 ? 'window' : 'windows'} — limits are not enforced:{' '}
              {windows
                .map((w) => `${w.reason} (${w.opened_by}, until ${w.expires_at})`)
                .join('; ')}
            </p>
          )}
          {/* Both controls, always — the badge above may say `state unknown`,
              and an operator who cannot tell whether trading is halted still
              needs to be able to halt it. Gating the halt on a read that did not
              answer would make the failure of the read disable the response to
              it. */}
          <div className="desk-actions">
            <Button
              size="xs"
              variant="danger"
              disabled={tripped}
              onClick={() => {
                globalAct.reset()
                setGlobal('halt')
              }}
            >
              {tripped ? 'Halted' : 'Halt all'}
            </Button>
            <Button
              size="xs"
              variant="secondary"
              onClick={() => {
                globalAct.reset()
                setGlobal('resume')
              }}
            >
              Reset (logged)
            </Button>
          </div>
          <GlobalActionDialog
            key={global ?? 'closed'}
            action={global}
            halted={scopes.data ? scopes.data.halted_scopes : null}
            busy={globalAct.isPending}
            error={globalAct.isError ? strategyActionRefusal(globalAct.error) : null}
            onCancel={() => setGlobal(null)}
            onConfirm={(built) => globalAct.mutate(built)}
          />
        </PanelBody>
      </Panel>

      <Panel>
        <PanelHeader title="Strategies" />
        <PanelBody>
          {strategies.data === null || strategies.data === undefined ? (
            <EmptyState
              message="The monitor did not answer"
              detail="/api/monitor/strategies is served (COMP-007), so this is the request failing rather than the phase being unbuilt. The live strategies are unknown, not none."
            />
          ) : strategies.data.strategies.length === 0 ? (
            <EmptyState
              message="No strategy is under the monitor"
              detail="The endpoint answered with an empty list, which is a fact about the monitor rather than about this page."
            />
          ) : (
            <table className="tbl">
              <thead>
                <tr>
                  <th style={{ textAlign: 'left' }}>strategy</th>
                  <th>state</th>
                  <th>scale</th>
                  <th>environment</th>
                  <th />
                </tr>
              </thead>
              <tbody>
                {strategies.data.strategies.map((s) => {
                  const row = monitorRow(s as Record<string, unknown>)
                  return (
                    <tr key={row.strategy_id}>
                      <td style={{ textAlign: 'left' }}>{row.strategy_id}</td>
                      <td>{row.state ?? 'not reported'}</td>
                      {/* 0% is "the monitor shut this down"; an absent scale is
                          "nobody said". They must not look the same. */}
                      <td>{row.scale_pct === null ? 'not reported' : `${row.scale_pct}%`}</td>
                      {/* Read from the order journal, and `null` there means no
                          order has ever been placed — unknown, not paper. */}
                      <td>
                        {(() => {
                          const env = environmentBadge(row.environment)
                          return (
                            <Badge tone={env.tone} data-testid={`env-${row.strategy_id}`}>
                              {env.word}
                            </Badge>
                          )
                        })()}
                      </td>
                      <td>
                        <Button
                          size="xs"
                          variant="ghost"
                          onClick={() => {
                            act.reset()
                            setPending({ action: 'scale', row })
                          }}
                        >
                          Scale down
                        </Button>
                        {/* Halt and resume are offered on the state the route
                            reports. When it reports none, both are offered and
                            the dialog says the current state is unknown —
                            guessing which one applies would be the monitor
                            answering a question nobody asked it. */}
                        {row.state !== 'halted' && (
                          <Button
                            size="xs"
                            variant="ghost"
                            onClick={() => {
                              act.reset()
                              setPending({ action: 'halt', row })
                            }}
                          >
                            Halt
                          </Button>
                        )}
                        {row.state !== 'nominal' && (
                          <Button
                            size="xs"
                            variant="ghost"
                            onClick={() => {
                              act.reset()
                              setPending({ action: 'resume', row })
                            }}
                          >
                            Resume
                          </Button>
                        )}
                      </td>
                    </tr>
                  )
                })}
              </tbody>
            </table>
          )}
          <p className="sub">
            {/* The column that was here read `breaching_metric`, which the route
                does not send. An always-blank column is worse than a missing
                one: it reads as "nothing is breaching". */}
            Which limit a strategy is breaching is not on this route. `/api/monitor/events` carries
            the breaches and is not joined to these rows yet, so this table says what each strategy
            is, not what it did wrong.
          </p>
        </PanelBody>
      </Panel>

      <StrategyActionDialog
        key={pending ? `${pending.action}:${pending.row.strategy_id}` : 'none'}
        action={pending?.action ?? null}
        strategyId={pending?.row.strategy_id ?? ''}
        currentScale={pending?.row.scale_pct ?? null}
        busy={act.isPending}
        error={act.isError ? strategyActionRefusal(act.error) : null}
        onCancel={() => {
          act.reset()
          setPending(null)
        }}
        onConfirm={(body) => act.mutate({ body })}
      />
    </div>
  )
}

// ── Strategy editor (§8) ─────────────────────────────────────────────────────

export interface ValidatorError {
  line: number
  message: string
  /** The edit that fixes it, applied verbatim. */
  fix: string | null
}

export function StrategyEditor({ initial = '' }: { initial?: string }) {
  const [text, setText] = useState(initial)
  const [errors, setErrors] = useState<ValidatorError[] | null>(null)
  const [preview, setPreview] = useState<Record<string, unknown> | null>(null)

  async function validate() {
    try {
      const { data } = await api.post<{ errors: ValidatorError[] }>('/api/strategies/compile', { source: text })
      setErrors(data.errors ?? [])
    } catch {
      setErrors(null)
    }
  }

  async function runPreview() {
    try {
      const { data } = await api.post<Record<string, unknown>>('/api/strategies/preview', { source: text })
      setPreview(data)
    } catch {
      setPreview(null)
    }
  }

  return (
    <div className="desk-scroll" style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 'var(--s-3)' }}>
      <Panel>
        <PanelHeader
          title="SLv2"
          actions={
            <>
              <Button size="xs" onClick={() => void validate()}>
                Validate
              </Button>
              <Button size="xs" variant="secondary" onClick={() => void runPreview()}>
                Preview
              </Button>
            </>
          }
        />
        <PanelBody>
          <TextArea
            rows={18}
            aria-label="Strategy source"
            value={text}
            onChange={(e) => setText(e.target.value)}
            className="mono"
          />
          {errors === null ? (
            <p className="sub">
              The compiler endpoint (/api/strategies/compile) belongs to AGENT-011, Set S phase 5. Until it
              answers, this editor does not claim your strategy is valid.
            </p>
          ) : errors.length === 0 ? (
            <p className="sub">No validator errors.</p>
          ) : (
            <ul style={{ fontSize: 'var(--t-12)', paddingLeft: 'var(--s-4)' }}>
              {errors.map((e, i) => (
                <li key={i}>
                  line {e.line}: {e.message}{' '}
                  {e.fix && (
                    <Button size="xs" variant="ghost" onClick={() => setText(e.fix as string)}>
                      Apply fix
                    </Button>
                  )}
                </li>
              ))}
            </ul>
          )}
        </PanelBody>
      </Panel>
      <Panel>
        <PanelHeader title="preview_signals" />
        <PanelBody>
          {preview === null ? (
            <EmptyState message="No preview" detail="Preview needs the feature engine's preview_signals (AGENT-011)." />
          ) : (
            <pre className="mono well" style={{ fontSize: 'var(--t-11)', overflow: 'auto', maxHeight: '50vh' }}>
              {JSON.stringify(preview, null, 2)}
            </pre>
          )}
        </PanelBody>
      </Panel>
    </div>
  )
}
