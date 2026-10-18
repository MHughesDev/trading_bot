// The operator surfaces' data layer: the monitor's measurement routes, the
// reconciliation page, the data-coverage census, the alert inbox, execution
// quality and the governance reads (Set S S-7.10, Set T T-0.3 / T-2.2, Set P
// P-0.2, FB-RX-003..005).
//
// # What `Read<T>` is for
//
// Every one of these surfaces answers a question whose wrong answer is
// "nothing is wrong": an empty halt list, an empty defect list, no stale feed.
// An axios rejection folded into `[]` or `null` renders exactly that. So a read
// resolves to one of four *named* outcomes and the caller has to say which:
//
//   * `ok`         — the route answered; an empty list in `data` is a fact.
//   * `not_served` — 404/405/501: this build does not have the route.
//   * `forbidden`  — 403: the route exists and refused this session.
//   * `error`      — anything else (500, 401, a dropped connection), with the
//                    server's own words when it sent any.
//
// Never throws, so a react-query `queryFn` returning it never retries a fact.
import { api as client } from '@/lib/api'

export type Read<T> =
  | { state: 'ok'; data: T }
  | { state: 'not_served' }
  | { state: 'forbidden'; detail: string }
  | { state: 'error'; status: number | null; message: string }

interface ErrorLike {
  response?: { status?: number; data?: unknown }
  message?: string
}

/** The sentence a server error body carries, if it carries one. */
export function serverWords(data: unknown): string | null {
  if (!data || typeof data !== 'object') return null
  const d = data as Record<string, unknown>
  for (const k of ['detail', 'message', 'error']) {
    const v = d[k]
    if (typeof v === 'string' && v.trim() !== '') return v
  }
  return null
}

/** Folds a settled request into a [`Read`]. Exported so tests can drive it. */
export function classify(err: unknown): Read<never> {
  const e = err as ErrorLike
  const status = e?.response?.status ?? null
  const words = serverWords(e?.response?.data)
  if (status === 404 || status === 405 || status === 501) return { state: 'not_served' }
  if (status === 403) return { state: 'forbidden', detail: words ?? 'this session may not read it' }
  return { state: 'error', status, message: words ?? e?.message ?? 'the request failed' }
}

export async function read<T>(p: Promise<{ data: T }>): Promise<Read<T>> {
  try {
    return { state: 'ok', data: (await p).data }
  } catch (err) {
    return classify(err)
  }
}

/** The refusal sentence of a rejected write, in the server's own words. */
export function refusalWords(err: unknown): string {
  const e = err as ErrorLike
  return serverWords(e?.response?.data) ?? e?.message ?? 'the request failed'
}

// ── Monitor: measurement, defects, drills (COMP-007; S-7.10) ────────────────

export type FeedVerdict =
  | 'fresh'
  | 'warning'
  | 'hard_stale'
  | 'unmeasurable'
  | 'session_closed'
  | 'producer_silent'

export interface MonitorFeed {
  symbol: string
  venue: string
  timeframe_secs: number
  last_bar_at: string
  age_secs: number
  session_open: boolean | null
  affirmed_at: string | null
  verdict: FeedVerdict
  strategies: string[]
}

export interface MonitorFeeds {
  feeds: MonitorFeed[]
  /** Deployments whose feed has no gauge at all: measured by nothing. */
  unwatched_deployments: string[]
}

export interface VenueCheck {
  venue: string
  verdict: string
  instruments_checked: number
  detail: string | null
  checked_by: string | null
  checked_at: string
}

export interface MonitorVenues {
  all_venues_reconciled: boolean
  venues: VenueCheck[]
}

export interface CoverageDeployment {
  strategy_id: string
  instrument_id: string
  environment: string | null
  started_at: string | null
  declared_families: string[]
  judged: boolean
  promotion_gate: { allowed: boolean; reason?: string }
  feed_gauged: boolean
}

export interface MonitorCoverage {
  deployments: CoverageDeployment[]
}

export interface PlatformDefect {
  defect_id: string
  source: string
  summary: string
  evidence: unknown
  filed_by: string
  filed_at: string
  status: string
}

export interface DrillRow {
  drill: string
  last_run_at: string | null
  passed: boolean | null
}

export interface DrillRuns {
  suite_due: boolean
  interval_days: number
  drills: DrillRow[]
}

export interface DrillRunResult {
  passed: boolean
  drills: Array<{
    run_id: string
    drill: string
    passed: boolean
    duration_ms: number
    failure: string | null
    defect_id: string | null
  }>
}

export interface MonitorEventRow {
  event_id: string
  occurred_at: string
  scope: string
  family: string
  kind: string
  metric: string | null
  /** The number that tripped it, and the limit it was held to (0075). `null` is not recorded, never 0. */
  observed_value?: number | null
  limit_value?: number | null
  actor: string
  is_drill: boolean
}

export const monitorOpsApi = {
  feeds: () => read(client.get<MonitorFeeds>('/api/monitor/feeds')),
  venues: () => read(client.get<MonitorVenues>('/api/monitor/venues')),
  coverage: () => read(client.get<MonitorCoverage>('/api/monitor/coverage')),
  defects: () => read(client.get<{ defects: PlatformDefect[] }>('/api/monitor/defects')),
  resolveDefect: (id: string, resolution: string) =>
    client.post<{ defect_id: string; status: string }>(`/api/monitor/defects/${id}/resolve`, {
      resolution,
    }),
  drillRuns: () => read(client.get<DrillRuns>('/api/monitor/drill-runs')),
  runDrills: () => client.post<DrillRunResult>('/api/monitor/drill-runs'),
  events: (limit = 50) =>
    read(client.get<{ events: MonitorEventRow[] }>('/api/monitor/events', { params: { limit } })),
}

/** The monitor's push channel (one SSE stream; see `useMonitorStream`). */
export const MONITOR_STREAM = '/api/monitor/stream'

// ── Reconciliation (Set T T-0.3, T-2.2) ──────────────────────────────────────

export interface DivergenceHalt {
  halt_id: string
  strategy_id: string
  kind: string
  reason: string
  raised_at: string
  /** The order gate's own registry holds it, i.e. orders are being refused now. */
  in_force: boolean
  /** Also in the durable table, so a restart will not forget it. */
  persisted: boolean
}

export interface HaltList {
  blocked_orders: number
  halts: DivergenceHalt[]
}

export interface HaltCleared {
  halt_id: string
  strategy_id: string
  cleared_by: string
  cleared_at: string
  halt_reason: string
}

export interface FleetDeployment {
  deployment_id: string
  strategy_id: string
  numeraire: string
  bars: number
  backfilled_excluded: number
  mean_divergence: number | null
  cumulative_divergence: number | null
  slippage_excess_bps: number | null
}

export type FleetVerdict = 'not_computable' | 'no_divergence' | 'idiosyncratic' | 'common_mode'

export interface FleetReport {
  window: { from: string; to: string; days: number }
  policy: { tolerance_per_bar: number; quorum: number; min_bars: number }
  deployments: FleetDeployment[]
  aggregate: {
    deployments: number
    measurable: number
    bars: number
    numeraire: string
    mean_divergence: number | null
    diverging: number
  } | null
  verdict: FleetVerdict
  /** Shape depends on `verdict`; `fleetVerdictWords` reads it defensively. */
  detail: unknown
  defect_id: string | null
}

export const reconciliationApi = {
  halts: () => read(client.get<HaltList>('/api/reconciliation/halts')),
  /** A named human, the strategy id typed out, and a note — all three, or 400. */
  clearHalt: (strategyId: string, body: { confirm_strategy: string; note: string }) =>
    client.post<HaltCleared>(
      `/api/reconciliation/halts/${encodeURIComponent(strategyId)}/clear`,
      body,
    ),
  fleet: () => read(client.get<FleetReport>('/api/reconciliation/fleet')),
}

// ── Data coverage (Set P P-0.2) ──────────────────────────────────────────────

export interface FloorVerdict {
  satisfied: boolean
  shortfall_days: number | null
  /** The floor's sentence, written by the server so the page cannot re-word it. */
  permits: string
}

export interface CoverageSeries {
  instrument: string
  venue: string
  timeframe: string
  series_kind: string
  as_of: string
  first_bar: string | null
  last_bar: string | null
  expected_bars: number
  present_bars: number
  missing_bps: number
  continuous_days: number
  derived_knowledge_bars: number | null
  observed_knowledge_bars: number | null
  last_row_age_secs: number | null
  accruing: boolean
  strict_v1: FloorVerdict
  paper_v1: FloorVerdict
}

export interface ForwardSeries {
  series: string
  venue: string
  instrument: string
  last_event_time: string
  last_write_time: string | null
  last_row_age_secs: number
  rows_written_last_beat: number | null
}

export interface DataCoverage {
  census_as_of: string | null
  census_taken: boolean
  floors: { strict_v1_days: number; paper_v1_days: number }
  series: CoverageSeries[]
  forward:
    | { measured: true; series: ForwardSeries[] }
    | { measured: false; reason: string; series: null }
}

export const dataCoverageApi = {
  get: () => read(client.get<DataCoverage>('/api/data/coverage')),
}

// ── Platform health, gates, timeline, aliases (FB-RX-004) ────────────────────

export interface GateVerdict {
  gate_no: number
  gate_name: string
  passed: boolean
  statistic: number | null
  threshold: number | null
  detail: string
  profile_id: string
  n_eff: number | null
  trial_count_at_eval: number | null
  decided_at: string
}

export interface GateStack {
  subject: string
  recorded: number
  of: number
  passed: number
  complete: boolean
  blocked_at: number | null
  profile_id: string | null
  verdicts: GateVerdict[]
}

export interface TimelineRow {
  kind: 'audit' | 'decision'
  at: string
  what: string
  phase: string
  verdict: string | null
  envelope: string | null
  actor_kind: string
  actor_id: string
  on_behalf_of: string | null
  propensity: number | null
  exploration_flag: boolean | null
  candidate_count: number | null
  detail: unknown
}

export interface DeploymentRecord {
  deployment_id: string
  strategy_name: string
  revision: string | null
  instrument_id: string | null
  environment: string
  venue: string | null
  starting_capital: string
  started_at: string
  stopped_at: string | null
  stop_reason: string | null
  running: boolean
}

export interface GovernedModel {
  model_id: string
  slug: string
  display_name: string
  model_kind: string
  status: string
}

export const governanceApi = {
  gates: (subject: string) =>
    read(client.get<GateStack>(`/api/platform/gates/${encodeURIComponent(subject)}`)),
  timeline: (limit = 200) =>
    read(
      client.get<{ rows: TimelineRow[]; prevented: number; limit: number }>('/api/platform/timeline', {
        params: { limit },
      }),
    ),
  deployments: () => read(client.get<DeploymentRecord[]>('/api/deployments')),
  models: () => read(client.get<{ models: GovernedModel[] }>('/api/models')),
  /** alias name -> version number. */
  aliases: (id: string) => read(client.get<Record<string, number>>(`/api/models/${id}/aliases`)),
  /** The existing rollback route: the only write on the governance page. */
  rollback: (id: string, alias: string) =>
    client.post<unknown>(`/api/models/${id}/aliases/${alias}/rollback`),
}

// ── Alerts inbox (FB-RX-005) ─────────────────────────────────────────────────

export type AlertSeverity = 'critical' | 'warning' | 'info'
export type AlertState = 'unread' | 'acknowledged' | 'dismissed'
export type AlertSource = 'monitor' | 'price' | 'calibration'

export interface InboxItem {
  source: AlertSource
  id: string
  at: string
  severity: AlertSeverity
  state: AlertState
  title: string
  detail: string | null
  /** `false` while a kill-switch trip this item reports is still in force. */
  dismissible: boolean
  /** Why it is not dismissible, when it is not. */
  pinned_reason: string | null
}

export interface Inbox {
  unread: number
  items: InboxItem[]
}

export const alertsInboxApi = {
  inbox: () => read(client.get<Inbox>('/api/alert-inbox')),
  acknowledge: (source: AlertSource, id: string) =>
    client.post(`/api/alert-inbox/${source}/${encodeURIComponent(id)}/acknowledge`),
  dismiss: (source: AlertSource, id: string) =>
    client.post(`/api/alert-inbox/${source}/${encodeURIComponent(id)}/dismiss`),
}

// ── Execution quality (FB-RX-003) ────────────────────────────────────────────

/** A statistic either has a value or says why not; it is never `0` for "no data". */
export type Measured<T> = { status: 'measured'; value: T } | { status: 'unmeasured'; reason: string }

export interface ExecQualityRow {
  strategy_id: string
  instrument_id: string
  orders: number
  fill_count: number
  /** filled quantity over ordered quantity. */
  fill_ratio: Measured<number>
  slippage_bps_p50: Measured<number>
  slippage_bps_p95: Measured<number>
  submit_to_fill_ms_p50: Measured<number>
  submit_to_fill_ms_p95: Measured<number>
}

export interface ExecQuality {
  window_days: number
  rows: ExecQualityRow[]
}

export const execQualityApi = {
  get: (days = 30) =>
    read(client.get<ExecQuality>('/api/execution/quality', { params: { days } })),
}
