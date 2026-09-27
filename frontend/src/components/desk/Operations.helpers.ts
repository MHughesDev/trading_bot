import { useQuery } from '@tanstack/react-query'
import type { BadgeTone } from '@/components/primitives/Badge'
import { Button } from '@/components/primitives/Button'
import { Panel, PanelBody, PanelHeader } from '@/components/primitives/Panel'
import { EmptyState } from '@/components/primitives/States'
import { api } from '@/lib/api'
import { deskApi, type StrategyAction } from '@/api/desk'
import type { BuiltAction, BuiltGlobal, GlobalAction, MonitorRow, StrategyActionInput, UsageLine } from './Operations'

/** `null` and `0` are different answers, and only one of them is a measurement. */
export function formatRate(rate: number | null, runs: number): string {
  if (rate === null || runs === 0) return 'not measured'
  return `${Math.round(rate * 100)}%`
}

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
        // "no prompt has been billed yet" is a claim about billing. With an
        // empty table nobody knows whether a prompt was billed — only that no
        // row says so. This stays ahead of everything else.
        if (unrecorded) return { key, label, value: NOT_RECORDED }
        // The order of the next two is the fix, and it is the whole fix.
        //
        // This read `if (n === null) return 'not reported'` **first**, and the
        // better sentence below was keyed on `billablePrompt === 0`. That was
        // written when the server sent `0.0` for an unbilled desk. It does not
        // any more: `usage_body` in `crates/api/src/routes/agent_sessions.rs`
        // now sends `cache_hit_rate: null` in exactly the case
        // `billablePrompt === 0` describes, with its own comment saying "a rate
        // over nothing billable is not a rate of zero". So the null check
        // short-circuited the one branch that could explain the null, and
        // "not measured — no prompt has been billed yet" became unreachable on
        // every response the platform can now produce.
        //
        // `billablePrompt === 0` is still what licenses the sentence — it is
        // the evidence for it, read off `input_tokens` and `cache_read` in the
        // same payload — so it is the condition, not the null. A null with
        // billable tokens behind it is some other absence, and gets the vaguer
        // word.
        if (billablePrompt === 0) {
          return { key, label, value: 'not measured — no prompt has been billed yet' }
        }
        if (n === null) return { key, label, value: 'not reported' }
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
 * The global scope's confirmation word, which is not ours to choose either.
 *
 * `Scope::Global` displays as the bare word `global`, and the same
 * `ResetRequest::validate` compares it literally.
 */
export const GLOBAL_SCOPE = 'global'

export const USAGE_LABELS: Record<string, string> = {
  input_tokens: 'prompt tokens',
  output_tokens: 'output tokens',
  cache_read: 'cache reads',
  cache_write: 'cache writes',
  cost_usd: 'cost',
  cache_hit_rate: 'cache hit rate',
  verdicts: 'verdicts filed',
  dollars_per_verdict: 'cost per verdict',
}

export function numberOrNull(v: unknown): number | null {
  return typeof v === 'number' && Number.isFinite(v) ? v : null
}

/** The four counters `/api/agent/usage` sums straight out of `llm_usage`. */
export const LLM_USAGE_COUNTERS = ['input_tokens', 'output_tokens', 'cache_read', 'cache_write'] as const

/** What a counter is allowed to say when the table behind it recorded nothing. */
export const NOT_RECORDED =
  'not recorded — llm_usage holds nothing for this desk, and only the LLM proxy writes it. Not a measured zero.'
