/**
 * The operator pages' pure rules: reconciliation, coverage, alerts, execution
 * quality and governance (Set T T-0.3 / T-2.2, Set P P-0.2, FB-RX-003..005).
 *
 * One property runs through all of them and it is this repository's favourite
 * one: **a value nobody measured is not zero, and a list that failed to load is
 * not an empty one.** Each describe block names the place it could have been
 * flattened.
 */
import { describe, expect, it } from 'vitest'
import { classify, serverWords } from '@/api/ops'
import type { CoverageSeries, DivergenceHalt, ExecQualityRow, FleetReport, InboxItem } from '@/api/ops'
import { clearReadiness, fleetView, haltStanding, bpsWords } from '@/lib/ops/reconciliation'
import { depthFraction, depthStanding, depthWords, lastRowWords, stalestFirst, summarise, thinnestFirst } from '@/lib/ops/coverage'
import { dismissControl, hasUnreadCritical, inboxOrder, severityView, unreadLabel } from '@/lib/ops/alerts'
import { fmtBps, measuredText, reasonOf, slippageReason, totalFills, unproducedStatistics } from '@/lib/ops/execQuality'
import { gateStackWords, preventedRows, rollbackConsequence } from '@/lib/ops/governance'

describe('a read that failed is one of four named outcomes, never an empty list', () => {
  it('classifies the statuses', () => {
    expect(classify({ response: { status: 404 } })).toEqual({ state: 'not_served' })
    expect(classify({ response: { status: 405 } })).toEqual({ state: 'not_served' })
    expect(classify({ response: { status: 403, data: { detail: 'desk only' } } })).toEqual({
      state: 'forbidden',
      detail: 'desk only',
    })
    expect(classify({ response: { status: 500, data: { error: 'monitor_unavailable' } } })).toEqual({
      state: 'error',
      status: 500,
      message: 'monitor_unavailable',
    })
    expect(classify(new Error('Network Error'))).toMatchObject({ state: 'error', status: null })
  })

  it('takes the server sentence from detail, then message, then error', () => {
    expect(serverWords({ detail: 'a', message: 'b', error: 'c' })).toBe('a')
    expect(serverWords({ message: 'b', error: 'c' })).toBe('b')
    expect(serverWords({ error: 'c' })).toBe('c')
    expect(serverWords({ error: '  ' })).toBeNull()
    expect(serverWords('nope')).toBeNull()
  })
})

const halt = (over: Partial<DivergenceHalt> = {}): DivergenceHalt => ({
  halt_id: 'h1',
  strategy_id: 'btc-trend-v2',
  kind: 'position',
  reason: 'internal position 3 vs broker 2',
  raised_at: '2026-10-03T09:00:00Z',
  in_force: true,
  persisted: true,
  ...over,
})

describe('a halt is cleared only by the exact strategy id and a note', () => {
  it('is not ready on an empty or near-miss id', () => {
    expect(clearReadiness(halt(), '', 'checked').ready).toBe(false)
    // Case folded, trimmed and prefixed versions are all different strategies.
    for (const near of ['BTC-TREND-V2', ' btc-trend-v2', 'btc-trend-v2 ', 'btc-trend-v']) {
      const r = clearReadiness(halt(), near, 'checked')
      expect(r.ready).toBe(false)
      expect(r.why).toMatch(/character for character/)
    }
  })

  it('is not ready without a note, even with the right id', () => {
    const r = clearReadiness(halt(), 'btc-trend-v2', '   ')
    expect(r.ready).toBe(false)
    expect(r.why).toMatch(/note/)
  })

  it('is ready with both', () => {
    expect(clearReadiness(halt(), 'btc-trend-v2', 'broker confirmed 3')).toEqual({ ready: true, why: null })
  })

  it('distinguishes a halt in force from one only held in memory', () => {
    expect(haltStanding(halt()).word).toBe('in force')
    expect(haltStanding(halt({ persisted: false })).tone).toBe('warn')
    expect(haltStanding(halt({ in_force: false })).word).toBe('persisted, not in force')
  })
})

const fleet = (verdict: FleetReport['verdict'], detail: unknown): FleetReport => ({
  window: { from: 'a', to: 'b', days: 30 },
  policy: { tolerance_per_bar: 0.0005, quorum: 3, min_bars: 30 },
  deployments: [],
  aggregate: null,
  verdict,
  detail,
  defect_id: null,
})

describe('the fleet verdict is never the comfortable reading of nothing', () => {
  it('not computable is not "no divergence"', () => {
    const v = fleetView(fleet('not_computable', 'no deployment has forward observations'))
    expect(v.tone).toBe('warn')
    expect(v.headline).toBe('Not computable')
    expect(v.sentence).toMatch(/Nothing can be said/)
  })

  it('common mode carries the server observation and is the red one', () => {
    const v = fleetView(fleet('common_mode', { observed: 'five deployments lost 9 bps per bar to slippage' }))
    expect(v.tone).toBe('neg')
    expect(v.sentence).toBe('five deployments lost 9 bps per bar to slippage')
  })

  it('reads a detail it does not recognise without throwing', () => {
    expect(() => fleetView(fleet('idiosyncratic', null))).not.toThrow()
    expect(fleetView(fleet('no_divergence', undefined)).tone).toBe('pos')
  })

  it('prints divergence in bps and an unmeasured one as such', () => {
    expect(bpsWords(0.0005, 0)).toBe('5 bps')
    expect(bpsWords(null)).toBe('not measured')
  })
})

const series = (over: Partial<CoverageSeries>): CoverageSeries => ({
  instrument: 'BTC-USD',
  venue: 'coinbase',
  timeframe: '1m',
  series_kind: 'ohlcv',
  as_of: '2026-10-03T00:00:00Z',
  first_bar: null,
  last_bar: null,
  expected_bars: 100,
  present_bars: 100,
  missing_bps: 0,
  continuous_days: 400,
  derived_knowledge_bars: null,
  observed_knowledge_bars: null,
  last_row_age_secs: 60,
  accruing: true,
  strict_v1: { satisfied: false, shortfall_days: 1400, permits: 'x' },
  paper_v1: { satisfied: true, shortfall_days: null, permits: 'y' },
  ...over,
})

describe('the census is thinnest first and says depth in words', () => {
  it('orders by continuous days ascending whatever order it arrived in', () => {
    const rows = thinnestFirst([
      series({ instrument: 'A', continuous_days: 900 }),
      series({ instrument: 'B', continuous_days: 12 }),
      series({ instrument: 'C', continuous_days: 400 }),
    ])
    expect(rows.map((r) => r.instrument)).toEqual(['B', 'C', 'A'])
  })

  it('states depth from the continuous run', () => {
    expect(depthWords({ continuous_days: 41, present_bars: 9 })).toBe('41 days of continuous history')
    expect(depthWords({ continuous_days: 0, present_bars: 0 })).toBe('no rows')
    expect(depthWords({ continuous_days: 1900, present_bars: 9 })).toMatch(/^5 years/)
    expect(depthWords({ continuous_days: 200, present_bars: 9 })).toBe('7 months of continuous history')
  })

  it('classifies against both floors', () => {
    expect(depthStanding(series({ strict_v1: { satisfied: true, shortfall_days: null, permits: '' } }))).toBe('strict')
    expect(depthStanding(series({}))).toBe('paper')
    expect(depthStanding(series({ paper_v1: { satisfied: false, shortfall_days: 5, permits: '' } }))).toBe('short')
    expect(depthStanding(series({ present_bars: 0 }))).toBe('empty')
  })

  it('clamps the depth bar to the floor', () => {
    expect(depthFraction({ continuous_days: 3650 }, 1825)).toBe(1)
    expect(depthFraction({ continuous_days: 365 }, 1825)).toBeCloseTo(0.2)
    expect(depthFraction({ continuous_days: 10 }, 0)).toBe(0)
  })

  it('counts what is not accruing', () => {
    const s = summarise({
      census_as_of: null,
      census_taken: true,
      floors: { strict_v1_days: 1825, paper_v1_days: 365 },
      series: [series({}), series({ accruing: false }), series({ present_bars: 0, accruing: false })],
      forward: { measured: true, series: [] },
    })
    expect(s).toMatchObject({ total: 3, paper: 2, empty: 1, notAccruing: 2 })
  })

  it('a row with no measured age says so, and the stalest forward series leads', () => {
    expect(lastRowWords(null)).toBe('not measured')
    expect(lastRowWords(7200)).toBe('2 h ago')
    const fw = (age: number, name: string) => ({
      series: name,
      venue: 'v',
      instrument: 'i',
      last_event_time: 't',
      last_write_time: null,
      last_row_age_secs: age,
      rows_written_last_beat: null,
    })
    expect(stalestFirst([fw(10, 'a'), fw(9000, 'b'), fw(50, 'c')]).map((f) => f.series)).toEqual(['b', 'c', 'a'])
  })
})

const item = (over: Partial<InboxItem>): InboxItem => ({
  source: 'monitor',
  id: 'i1',
  at: '2026-10-03T10:00:00Z',
  severity: 'warning',
  state: 'unread',
  title: 't',
  detail: null,
  dismissible: true,
  pinned_reason: null,
  ...over,
})

describe('the inbox ranks by what needs a person and pins a live trip', () => {
  it('a pinned trip has no dismiss control and says why', () => {
    const pinned = item({ severity: 'critical', dismissible: false, pinned_reason: 'the kill switch is still tripped' })
    expect(dismissControl(pinned)).toEqual({ enabled: false, why: 'the kill switch is still tripped' })
    expect(dismissControl(item({})).enabled).toBe(true)
    // A server that forgot the reason still produces a sentence.
    expect(dismissControl(item({ dismissible: false })).why).toBeTruthy()
  })

  it('orders unread first, then severity, then newest', () => {
    const out = inboxOrder([
      item({ id: 'read-crit', state: 'acknowledged', severity: 'critical' }),
      item({ id: 'unread-info', severity: 'info', at: '2026-10-03T12:00:00Z' }),
      item({ id: 'unread-crit', severity: 'critical', at: '2026-10-03T09:00:00Z' }),
      item({ id: 'unread-warn-new', at: '2026-10-03T11:00:00Z' }),
    ])
    expect(out.map((i) => i.id)).toEqual(['unread-crit', 'unread-warn-new', 'unread-info', 'read-crit'])
  })

  it('shows a word for an unknown severity rather than downgrading it', () => {
    expect(severityView('catastrophic')).toEqual({ word: 'catastrophic', tone: 'warn' })
    expect(severityView('critical').tone).toBe('neg')
  })

  it('labels the shell badge', () => {
    expect([0, null, undefined, 3, 250].map(unreadLabel)).toEqual([null, null, null, '3', '99+'])
    expect(hasUnreadCritical([item({ severity: 'critical' })])).toBe(true)
    expect(hasUnreadCritical([item({ severity: 'critical', state: 'acknowledged' })])).toBe(false)
  })
})

const q = (over: Partial<ExecQualityRow> = {}): ExecQualityRow => ({
  strategy_id: 's',
  instrument_id: 'BTC-USD',
  orders: 2,
  fill_count: 2,
  fill_ratio: { status: 'measured', value: 0.5 },
  slippage_bps_p50: { status: 'unmeasured', reason: 'no fill model prediction is recorded' },
  slippage_bps_p95: { status: 'unmeasured', reason: 'no fill model prediction is recorded' },
  submit_to_fill_ms_p50: { status: 'measured', value: 1500 },
  submit_to_fill_ms_p95: { status: 'measured', value: 20 },
  ...over,
})

describe('an unmeasured statistic is never printed as a number', () => {
  it('prints words, with the reason on hover', () => {
    const m = q().slippage_bps_p50
    expect(measuredText(m, fmtBps)).toBe('not measured')
    expect(reasonOf(m)).toMatch(/no fill model/)
    expect(measuredText({ status: 'measured', value: 0 }, fmtBps)).toBe('0.0 bps')
    expect(reasonOf({ status: 'measured', value: 0 })).toBeUndefined()
  })

  it('names a statistic no row has, once', () => {
    expect(unproducedStatistics([q(), q()])).toEqual(['slippage versus the fill model'])
    expect(unproducedStatistics([])).toEqual([])
    expect(slippageReason([q()])).toMatch(/no fill model/)
  })

  it('counts fills for the empty state', () => {
    expect(totalFills([q({ fill_count: 0 }), q({ fill_count: 0 })])).toBe(0)
    expect(totalFills([q({ fill_count: 3 }), q({ fill_count: 4 })])).toBe(7)
  })
})

describe('a gate stack authorises only when it is complete and passing', () => {
  const stack = (over: object) => ({
    subject: 'exp-1',
    recorded: 16,
    of: 16,
    passed: 16,
    complete: true,
    blocked_at: null,
    profile_id: 'p',
    verdicts: [],
    ...over,
  })

  it('says nothing recorded is not a pass', () => {
    const w = gateStackWords(stack({ recorded: 0, passed: 0, complete: false }))
    expect(w.word).toBe('nothing recorded')
    expect(w.tone).toBe('warn')
  })

  it('names the first failing gate', () => {
    const w = gateStackWords(stack({ passed: 11, blocked_at: 12 }))
    expect(w.word).toBe('blocked at gate 12')
    expect(w.tone).toBe('neg')
  })

  it('does not call twelve passes out of twelve recorded a complete stack', () => {
    const w = gateStackWords(stack({ recorded: 12, passed: 12, complete: false }))
    expect(w.word).toBe('incomplete')
    expect(w.sentence).toMatch(/did not run/)
  })

  it('is green only when whole', () => {
    expect(gateStackWords(stack({})).tone).toBe('pos')
  })

  it('states the rollback consequence with the alias and the version', () => {
    const t = rollbackConsequence('EgoGemma', 'production', 14)
    expect(t).toContain('"production"')
    expect(t).toContain('version 14')
    expect(rollbackConsequence('M', 'a', undefined)).toContain('its current version')
  })

  it('picks out the denied and held actions', () => {
    const row = (verdict: string | null, kind: 'audit' | 'decision' = 'audit') => ({
      kind,
      at: 't',
      what: 'w',
      phase: 'pre',
      verdict,
      envelope: null,
      actor_kind: 'agent',
      actor_id: 'a',
      on_behalf_of: null,
      propensity: null,
      exploration_flag: null,
      candidate_count: null,
      detail: {},
    })
    expect(preventedRows([row('denied'), row('allowed'), row('pending_approval'), row('denied', 'decision')])).toHaveLength(2)
  })
})
