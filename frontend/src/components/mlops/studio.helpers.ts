// Pure logic behind the AI Model Studio surfaces (Set H phase 5).
//
// Honesty rules, the same ones the Research Desk follows:
//   * a value the platform did not measure is `null`, never `0` and never an
//     invented default -- a card with no accuracy shows a dash, not "0.0%";
//   * a failed gate is not a promotion: `POST .../promote` answers HTTP 200 with
//     `promoted: false`, and `parsePromoteResult` is what tells the two apart;
//   * a winner needs a measurement on BOTH sides. The server compares
//     `scorecard.overall` and treats a missing one as 0.0, which crowns the side
//     that has any evaluation over the side that has none.
import type {
  AiModel,
  AliasMap,
  EvaluationRun,
  GateCheck,
  ModelDeployment,
  ModelKind,
  ModelStatus,
  ModelVersion,
  PromoteResult,
  RegressionCheck,
  RegressionReport,
  Scorecard,
} from '@/api/mlops'

// ── Labels ───────────────────────────────────────────────────────────────────

export const KIND_LABELS: Record<ModelKind, string> = {
  forecaster: 'Forecaster',
  signal_ranker: 'Signal Ranker',
  trade_decision: 'Trade Decision',
  risk_sizing: 'Risk Sizing',
  embedding: 'Embedding',
  external_llm_adapter: 'LLM Adapter',
}

export const ALL_KINDS = Object.keys(KIND_LABELS) as ModelKind[]

export const ALL_STATUSES: ModelStatus[] = [
  'draft',
  'training',
  'evaluating',
  'candidate',
  'active',
  'archived',
  'failed',
]

export const ALIAS_SLOTS = ['production', 'candidate', 'staging', 'fallback'] as const

// ── Metrics ──────────────────────────────────────────────────────────────────

/** Headline quality metrics, in the order a reader would look for them. */
export const HEADLINE_METRICS = [
  'val_auc',
  'accuracy',
  'val_accuracy',
  'val_directional_accuracy',
  'val_logloss',
  'val_r2',
  'rmse',
  'val_rmse',
  'mae',
  'val_mae',
  'test_auc',
  'test_accuracy',
  'test_rmse',
  'test_mae',
]

/** Error-like metrics, where a smaller number is the better one. */
export function isLowerBetter(key: string): boolean {
  return /rmse|mae|logloss|loss|error|crps|pinball|mape/i.test(key)
}

export function numericMetrics(metrics: unknown): Array<[string, number]> {
  if (!metrics || typeof metrics !== 'object') return []
  return Object.entries(metrics as Record<string, unknown>).filter(
    (e): e is [string, number] => typeof e[1] === 'number' && Number.isFinite(e[1]),
  )
}

export interface PrimaryMetric {
  key: string
  value: number
}

export function primaryMetric(metrics: unknown): PrimaryMetric | null {
  const nums = numericMetrics(metrics)
  if (nums.length === 0) return null
  for (const key of HEADLINE_METRICS) {
    const hit = nums.find(([k]) => k === key)
    if (hit) return { key: hit[0], value: hit[1] }
  }
  return null
}

/** Headline metrics first, the rest after, each group keeping server order. */
export function sortMetricEntries(entries: Array<[string, number]>): Array<[string, number]> {
  const rank = (k: string) => {
    const i = HEADLINE_METRICS.indexOf(k)
    return i === -1 ? HEADLINE_METRICS.length : i
  }
  return [...entries].sort((a, b) => rank(a[0]) - rank(b[0]))
}

/**
 * The primary-metric series across versions, oldest first, for a sparkline.
 * The key is the latest version's headline metric, and only versions that
 * report that same key contribute -- two different metrics on one line is not a
 * trend.
 */
export function metricSeries(versions: ModelVersion[] | null | undefined): {
  key: string | null
  points: number[]
} {
  if (!versions || versions.length === 0) return { key: null, points: [] }
  const ordered = [...versions].sort((a, b) => a.version - b.version)
  let key: string | null = null
  for (let i = ordered.length - 1; i >= 0 && key === null; i--) {
    key = primaryMetric(ordered[i].metrics)?.key ?? null
  }
  if (key === null) return { key: null, points: [] }
  const points: number[] = []
  for (const v of ordered) {
    const hit = numericMetrics(v.metrics).find(([k]) => k === key)
    if (hit) points.push(hit[1])
  }
  return { key, points }
}

export function formatMetric(key: string, value: number): string {
  // Ratios in [0,1] read as percentages for the accuracy family; the rest keep
  // their native scale.
  if (/acc|auc|r2/i.test(key) && value >= 0 && value <= 1) return `${(value * 100).toFixed(1)}%`
  return Math.abs(value) >= 100 ? value.toFixed(1) : value.toFixed(3)
}

/** `val_auc` -> `Val AUC`-ish short label for a metric chip. */
export function metricLabel(key: string): string {
  return key
    .replace(/^val_/, '')
    .replace(/^test_/, 'test ')
    .replace(/_/g, ' ')
}

// ── Used-by ──────────────────────────────────────────────────────────────────

export interface UsedByRef {
  id: string
  name: string
}

/**
 * The route answers `{ strategy_id, display_name }`; older callers read
 * `{ id, name }`. Both are accepted and anything without an id is dropped
 * rather than rendered as a chip that links nowhere.
 */
export function normaliseUsedBy(raw: unknown): UsedByRef[] {
  if (!Array.isArray(raw)) return []
  const out: UsedByRef[] = []
  for (const row of raw) {
    if (!row || typeof row !== 'object') continue
    const r = row as Record<string, unknown>
    const id = typeof r.strategy_id === 'string' ? r.strategy_id : typeof r.id === 'string' ? r.id : null
    if (!id) continue
    const name =
      typeof r.display_name === 'string' ? r.display_name : typeof r.name === 'string' ? r.name : id
    out.push({ id, name })
  }
  return out
}

// ── Command-center summary ───────────────────────────────────────────────────

export interface ModelSummary {
  productionVersion: number | null
  latestVersion: number | null
  primary: PrimaryMetric | null
  series: number[]
  /** `null` = the used-by query has not answered (unknown), `[]` = answered, none. */
  usedBy: UsedByRef[] | null
}

export function summariseModel(input: {
  versions?: ModelVersion[] | null
  aliases?: AliasMap | null
  usedBy?: UsedByRef[] | null
}): ModelSummary {
  const versions = input.versions ?? null
  const latest =
    versions && versions.length > 0 ? [...versions].sort((a, b) => b.version - a.version)[0] : null
  const prod = input.aliases?.production
  // The headline number is the PRODUCTION version's when there is one, else the
  // latest version's: a card that shows a candidate's accuracy beside an Active
  // pill is quoting the wrong model.
  const shown =
    (typeof prod === 'number' ? versions?.find((v) => v.version === prod) : undefined) ?? latest
  return {
    productionVersion: typeof prod === 'number' ? prod : null,
    latestVersion: latest ? latest.version : null,
    primary: shown ? primaryMetric(shown.metrics) : null,
    series: metricSeries(versions).points,
    usedBy: input.usedBy ?? null,
  }
}

export type SortKey = 'name' | 'kind' | 'status' | 'metric' | 'version' | 'updated'
export type SortDir = 'asc' | 'desc'

/** Rows with no value sort last in BOTH directions: absent is not smallest. */
export function sortModels(
  models: AiModel[],
  summaries: Record<string, ModelSummary | undefined>,
  key: SortKey,
  dir: SortDir,
): AiModel[] {
  const sign = dir === 'asc' ? 1 : -1
  const valueOf = (m: AiModel): string | number | null => {
    const s = summaries[m.model_id]
    switch (key) {
      case 'name':
        return m.display_name.toLowerCase()
      case 'kind':
        return m.model_kind
      case 'status':
        return m.status
      case 'metric':
        return s?.primary?.value ?? null
      case 'version':
        return s?.productionVersion ?? s?.latestVersion ?? null
      case 'updated':
        return Date.parse(m.updated_at) || null
    }
  }
  return [...models].sort((a, b) => {
    const va = valueOf(a)
    const vb = valueOf(b)
    if (va === null && vb === null) return 0
    if (va === null) return 1
    if (vb === null) return -1
    if (va < vb) return -1 * sign
    if (va > vb) return 1 * sign
    return 0
  })
}

export function assetClassOptions(models: AiModel[], selected: string): string[] {
  const set = new Set(models.map((m) => m.asset_class).filter(Boolean))
  if (selected) set.add(selected)
  return [...set].sort()
}

// ── View-mode persistence ────────────────────────────────────────────────────

export type ViewMode = 'cards' | 'table'
export const VIEW_MODE_KEY = 'studio.commandCenter.view'

/** Browser storage can be absent, full or throw; the toggle must still render. */
export function readViewMode(storage: Pick<Storage, 'getItem'> | null | undefined): ViewMode {
  try {
    return storage?.getItem(VIEW_MODE_KEY) === 'table' ? 'table' : 'cards'
  } catch {
    return 'cards'
  }
}

export function writeViewMode(
  storage: Pick<Storage, 'setItem'> | null | undefined,
  mode: ViewMode,
): void {
  try {
    storage?.setItem(VIEW_MODE_KEY, mode)
  } catch {
    // Persistence is a convenience; losing it must not break the toggle.
  }
}

// ── Promotion gate ───────────────────────────────────────────────────────────

export const GATE_LABELS: Record<string, string> = {
  passed_eval_suite: 'Passed an evaluation',
  artifact_integrity: 'Artifact hash recorded',
  rollback_available: 'Rollback available',
  no_metric_regression: 'No metric regression vs production',
}

export function gateLabel(check: string): string {
  return GATE_LABELS[check] ?? check.replace(/_/g, ' ')
}

export type PreflightState = 'pass' | 'fail' | 'server'

export interface PreflightCheck {
  check: string
  label: string
  /** `server` = this client cannot know; the server decides on confirm. */
  state: PreflightState
}

/**
 * What the modal can show BEFORE confirming, from data already loaded. The
 * regression check needs the evaluation's report, which the version list does
 * not carry, so it stays `server` rather than being guessed.
 */
export function preflightChecks(
  version: Pick<ModelVersion, 'status' | 'artifact_hash'>,
  hasProduction: boolean,
  environment: 'paper' | 'live',
): PreflightCheck[] {
  const evaluated = version.status === 'candidate' || version.status === 'active'
  return [
    {
      check: 'passed_eval_suite',
      label: gateLabel('passed_eval_suite'),
      state: evaluated ? 'pass' : 'fail',
    },
    {
      check: 'artifact_integrity',
      label: gateLabel('artifact_integrity'),
      state: version.artifact_hash ? 'pass' : 'fail',
    },
    {
      check: 'rollback_available',
      label: gateLabel('rollback_available'),
      state: environment !== 'live' || hasProduction ? 'pass' : 'fail',
    },
    { check: 'no_metric_regression', label: gateLabel('no_metric_regression'), state: 'server' },
  ]
}

export function preflightBlocksLive(checks: PreflightCheck[]): boolean {
  return checks.some((c) => c.state === 'fail')
}

export interface ParsedPromote {
  promoted: boolean
  refused: boolean
  checks: GateCheck[]
  overrideReason: string | null
}

/** Never trust the status code: a refused gate is a 200. */
export function parsePromoteResult(data: PromoteResult | null | undefined): ParsedPromote {
  const checks = Array.isArray(data?.gate_checks)
    ? (data?.gate_checks ?? []).filter(
        (c): c is GateCheck => !!c && typeof c.check === 'string' && typeof c.passed === 'boolean',
      )
    : []
  const promoted = data?.promoted === true
  return {
    promoted,
    refused: !promoted,
    checks,
    overrideReason: typeof data?.override_reason === 'string' ? data.override_reason : null,
  }
}

export function failedRequiredChecks(checks: GateCheck[]): GateCheck[] {
  return checks.filter((c) => c.required && !c.passed)
}

// ── Deployments: traffic split ───────────────────────────────────────────────

export const DEPLOY_ENVIRONMENTS = ['paper', 'live'] as const

/** Traffic already allocated to ACTIVE deployments in one environment. */
export function trafficAllocated(deployments: ModelDeployment[], environment: string): number {
  return deployments
    .filter((d) => d.environment === environment && d.status === 'active')
    .reduce((sum, d) => sum + (Number.isFinite(d.traffic_pct) ? d.traffic_pct : 0), 0)
}

export interface TrafficCheck {
  ok: boolean
  remaining: number
  message: string | null
}

/** Mirrors the server's rule (`allocated + requested <= 100`, per environment). */
export function validateTraffic(
  requested: number,
  deployments: ModelDeployment[],
  environment: string,
): TrafficCheck {
  const allocated = trafficAllocated(deployments, environment)
  const remaining = Math.max(0, 100 - allocated)
  if (!Number.isFinite(requested) || requested < 0 || requested > 100) {
    return { ok: false, remaining, message: 'Traffic must be between 0 and 100%.' }
  }
  if (allocated + requested > 100) {
    return {
      ok: false,
      remaining,
      message: `${environment} already routes ${allocated}% to active deployments; ${requested}% more would exceed 100%.`,
    }
  }
  return { ok: true, remaining, message: null }
}

export interface AliasSlot {
  alias: string
  version: number | null
}

/** All four canonical slots, so an unset one reads as unset instead of missing. */
export function aliasSlots(aliases: AliasMap | null | undefined): AliasSlot[] {
  const map = aliases ?? {}
  const slots: AliasSlot[] = ALIAS_SLOTS.map((alias) => ({
    alias,
    version: typeof map[alias] === 'number' ? map[alias] : null,
  }))
  for (const [alias, version] of Object.entries(map)) {
    if (!(ALIAS_SLOTS as readonly string[]).includes(alias) && typeof version === 'number') {
      slots.push({ alias, version })
    }
  }
  return slots
}

/** The version a rollback of `current` lands on: the server picks the next lower. */
export function rollbackTarget(current: number | null, versions: number[]): number | null {
  if (current === null) return null
  const lower = versions.filter((v) => v < current)
  return lower.length > 0 ? Math.max(...lower) : null
}

// ── Version timeline ─────────────────────────────────────────────────────────

export type TimelineTone = 'pos' | 'info' | 'neg' | 'neutral' | 'warn' | 'accent'

export function versionTone(status: ModelStatus, isProduction: boolean): TimelineTone {
  if (isProduction) return 'pos'
  switch (status) {
    case 'active':
      return 'pos'
    case 'candidate':
      return 'info'
    case 'failed':
      return 'neg'
    case 'training':
      return 'accent'
    case 'evaluating':
      return 'warn'
    default:
      return 'neutral'
  }
}

// ── Evaluations ──────────────────────────────────────────────────────────────

export interface EvalRow {
  evalId: string
  version: number
  status: string
  metrics: Array<[string, number]>
  primary: PrimaryMetric | null
  scorecard: Scorecard | null
  verdict: string | null
  checks: RegressionCheck[]
  baselineVersion: number | null
  createdAt: string
  /** The metrics object as served, for structured values (confusion matrix, backtest). */
  rawMetrics: unknown
}

function scorecardOrNull(raw: unknown): Scorecard | null {
  if (!raw || typeof raw !== 'object') return null
  const s = raw as Record<string, unknown>
  const keys = ['overall', 'quality', 'speed', 'cost', 'safety', 'reliability'] as const
  const out: Partial<Scorecard> = {}
  for (const k of keys) {
    const v = s[k]
    if (typeof v !== 'number' || !Number.isFinite(v)) return null
    out[k] = v
  }
  return out as Scorecard
}

export function regressionVerdict(report: RegressionReport | null | undefined): string | null {
  return report && typeof report.verdict === 'string' ? report.verdict : null
}

export function toEvalRow(ev: EvaluationRun): EvalRow {
  return {
    evalId: ev.eval_id,
    version: ev.version,
    status: ev.status,
    metrics: numericMetrics(ev.metrics),
    primary: primaryMetric(ev.metrics),
    scorecard: scorecardOrNull(ev.scorecard),
    verdict: regressionVerdict(ev.regression_report),
    checks: Array.isArray(ev.regression_report?.checks) ? ev.regression_report.checks : [],
    baselineVersion: typeof ev.baseline_version === 'number' ? ev.baseline_version : null,
    createdAt: ev.created_at,
    rawMetrics: ev.metrics ?? null,
  }
}

/** Production's row first; everything else newest first. */
export function pinProduction(rows: EvalRow[], productionVersion: number | null): EvalRow[] {
  const sorted = [...rows].sort((a, b) => Date.parse(b.createdAt) - Date.parse(a.createdAt))
  if (productionVersion === null) return sorted
  const prod = sorted.filter((r) => r.version === productionVersion)
  const rest = sorted.filter((r) => r.version !== productionVersion)
  return [...prod, ...rest]
}

/** The newest succeeded evaluation per version. */
export function latestSucceededByVersion(rows: EvalRow[]): Map<number, EvalRow> {
  const out = new Map<number, EvalRow>()
  for (const r of [...rows].sort((a, b) => Date.parse(a.createdAt) - Date.parse(b.createdAt))) {
    if (r.status === 'succeeded') out.set(r.version, r)
  }
  return out
}

export interface CompareSide {
  version: number
  metrics: Array<[string, number]>
  scorecard: Scorecard | null
}

export interface CompareResult {
  a: CompareSide
  b: CompareSide
  serverWinner: number | null
}

export function parseCompare(raw: unknown): CompareResult | null {
  if (!raw || typeof raw !== 'object') return null
  const r = raw as Record<string, unknown>
  if (!Array.isArray(r.versions) || r.versions.length !== 2) return null
  const side = (x: unknown): CompareSide | null => {
    if (!x || typeof x !== 'object') return null
    const o = x as Record<string, unknown>
    if (typeof o.version !== 'number') return null
    return {
      version: o.version,
      metrics: numericMetrics(o.metrics),
      scorecard: scorecardOrNull(o.scorecard),
    }
  }
  const a = side(r.versions[0])
  const b = side(r.versions[1])
  if (!a || !b) return null
  return {
    a,
    b,
    serverWinner: typeof r.winner_version === 'number' ? r.winner_version : null,
  }
}

export interface MetricDelta {
  key: string
  a: number | null
  b: number | null
  delta: number | null
  /** `a`/`b` = which side is better; `null` = cannot say (missing side or tie). */
  better: 'a' | 'b' | null
}

/** A metric one side never reported has no delta: absent is not zero. */
export function metricDeltas(a: CompareSide, b: CompareSide): MetricDelta[] {
  const am = new Map(a.metrics)
  const bm = new Map(b.metrics)
  const union: Array<[string, number]> = [...new Set([...am.keys(), ...bm.keys()])].map((k) => [k, 0])
  const keys = sortMetricEntries(union).map(([k]) => k)
  return keys.map((key) => {
    const av = am.has(key) ? (am.get(key) as number) : null
    const bv = bm.has(key) ? (bm.get(key) as number) : null
    if (av === null || bv === null) return { key, a: av, b: bv, delta: null, better: null }
    const delta = bv - av
    let better: 'a' | 'b' | null = null
    if (delta !== 0) {
      const bWins = isLowerBetter(key) ? delta < 0 : delta > 0
      better = bWins ? 'b' : 'a'
    }
    return { key, a: av, b: bv, delta, better }
  })
}

/**
 * The winner, only when both sides carry a scorecard. Returns the version, or
 * `null` with the reason the badge should state.
 */
export function trustedWinner(c: CompareResult): { version: number | null; reason: string } {
  if (!c.a.scorecard || !c.b.scorecard) {
    const missing = !c.a.scorecard ? c.a.version : c.b.version
    return { version: null, reason: `v${missing} has no completed evaluation, so there is nothing to compare.` }
  }
  if (c.a.scorecard.overall === c.b.scorecard.overall) {
    return { version: null, reason: 'The two versions tie on overall score.' }
  }
  const version = c.a.scorecard.overall > c.b.scorecard.overall ? c.a.version : c.b.version
  return { version, reason: `Higher overall score (${Math.max(c.a.scorecard.overall, c.b.scorecard.overall).toFixed(2)}).` }
}

/** A confusion matrix, only if the evaluation actually produced a square one. */
export function confusionMatrix(metrics: unknown): number[][] | null {
  if (!metrics || typeof metrics !== 'object') return null
  const m = (metrics as Record<string, unknown>).confusion_matrix
  if (!Array.isArray(m) || m.length === 0) return null
  const rows = m.map((r) =>
    Array.isArray(r) && r.every((c) => typeof c === 'number' && Number.isFinite(c))
      ? (r as number[])
      : null,
  )
  if (rows.some((r) => r === null)) return null
  const grid = rows as number[][]
  return grid.every((r) => r.length === grid.length) ? grid : null
}

// ── Training console ─────────────────────────────────────────────────────────

export interface ProgressSample {
  /** ms since epoch the sample was observed. */
  t: number
  progress: number
}

const MAX_SAMPLES = 240

/** Appends an observed sample; a repeat of the last progress at <10s is dropped. */
export function appendSample(samples: ProgressSample[], next: ProgressSample): ProgressSample[] {
  const last = samples[samples.length - 1]
  if (last && last.progress === next.progress && next.t - last.t < 10_000) return samples
  const out = [...samples, next]
  return out.length > MAX_SAMPLES ? out.slice(out.length - MAX_SAMPLES) : out
}

/** Seconds to completion from the observed rate, or `null` when it cannot be known. */
export function etaSeconds(samples: ProgressSample[]): number | null {
  if (samples.length < 2) return null
  const first = samples[0]
  const last = samples[samples.length - 1]
  const dp = last.progress - first.progress
  const dt = (last.t - first.t) / 1000
  if (dp <= 0 || dt <= 0 || last.progress >= 100) return null
  return Math.round(((100 - last.progress) * dt) / dp)
}

export function formatDuration(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds < 0) return '—'
  if (seconds < 60) return `${Math.round(seconds)}s`
  const m = Math.floor(seconds / 60)
  const s = Math.round(seconds % 60)
  if (m < 60) return `${m}m ${s}s`
  return `${Math.floor(m / 60)}h ${m % 60}m`
}

export interface BestComparison {
  key: string
  run: number
  best: number
  bestVersion: number
  delta: number
  better: boolean | null
}

/**
 * Run metrics vs the best earlier version, per shared numeric headline metric.
 * "Best" respects direction (lowest rmse, highest auc) and excludes versions
 * newer than the run's own output so a run is never compared with itself.
 */
export function compareToBest(
  runMetrics: unknown,
  versions: ModelVersion[] | null | undefined,
  excludeVersion?: number | null,
): BestComparison[] {
  const run = numericMetrics(runMetrics).filter(([k]) => HEADLINE_METRICS.includes(k))
  if (run.length === 0 || !versions) return []
  const out: BestComparison[] = []
  for (const [key, value] of run) {
    let best: { v: number; version: number } | null = null
    for (const ver of versions) {
      if (excludeVersion !== undefined && excludeVersion !== null && ver.version === excludeVersion) continue
      const hit = numericMetrics(ver.metrics).find(([k]) => k === key)
      if (!hit) continue
      const better = isLowerBetter(key) ? hit[1] < (best?.v ?? Infinity) : hit[1] > (best?.v ?? -Infinity)
      if (best === null || better) best = { v: hit[1], version: ver.version }
    }
    if (!best) continue
    const delta = value - best.v
    out.push({
      key,
      run: value,
      best: best.v,
      bestVersion: best.version,
      delta,
      better: delta === 0 ? null : isLowerBetter(key) ? delta < 0 : delta > 0,
    })
  }
  return out
}

// ── Lineage, derived ─────────────────────────────────────────────────────────

export interface LineageNode {
  id: string
  type: 'model' | 'training_run' | 'model_version' | 'deployment' | 'strategy'
  label: string
}
export interface LineageEdge {
  id: string
  source: string
  target: string
}

/**
 * The lineage route returns aliases only, no nodes (its own `note` says the full
 * graph waits on dataset refs), so the graph is derived from what the registry
 * does serve: versions, runs, deployments, aliases and used-by. Datasets are
 * not drawn -- nothing served names one.
 */
export function deriveLineage(input: {
  modelId: string
  modelName: string
  versions: ModelVersion[]
  runs: Array<{ run_id: string; kind: string; status: string }>
  deployments: ModelDeployment[]
  aliases: AliasMap
  usedBy: UsedByRef[]
}): { nodes: LineageNode[]; edges: LineageEdge[] } {
  const nodes: LineageNode[] = []
  const edges: LineageEdge[] = []
  const root = `model:${input.modelId}`
  nodes.push({ id: root, type: 'model', label: input.modelName })

  const prodVersion = input.aliases.production
  for (const v of [...input.versions].sort((a, b) => a.version - b.version)) {
    const id = `version:${v.version}`
    nodes.push({
      id,
      type: 'model_version',
      label: `v${v.version}${v.version === prodVersion ? ' · production' : ''}`,
    })
    edges.push({ id: `${root}>${id}`, source: root, target: id })
  }
  // Training runs carry no version, so they hang off the model, not a version.
  for (const r of input.runs.filter((x) => x.kind === 'training').slice(0, 12)) {
    const id = `run:${r.run_id}`
    nodes.push({ id, type: 'training_run', label: `run ${r.run_id.slice(0, 8)} · ${r.status}` })
    edges.push({ id: `${id}>${root}`, source: id, target: root })
  }
  for (const d of input.deployments) {
    const id = `deployment:${d.deployment_id}`
    nodes.push({ id, type: 'deployment', label: `${d.environment} · ${d.traffic_pct}%` })
    const from = `version:${d.version}`
    if (nodes.some((n) => n.id === from)) edges.push({ id: `${from}>${id}`, source: from, target: id })
  }
  // A strategy references the model by name, not a version, so it attaches to
  // the production version when there is one and otherwise to the model.
  const consumer = typeof prodVersion === 'number' ? `version:${prodVersion}` : root
  const attach = nodes.some((n) => n.id === consumer) ? consumer : root
  for (const s of input.usedBy) {
    const id = `strategy:${s.id}`
    nodes.push({ id, type: 'strategy', label: s.name })
    edges.push({ id: `${attach}>${id}`, source: attach, target: id })
  }
  return { nodes, edges }
}

const LAYER: Record<LineageNode['type'], number> = {
  training_run: 0,
  model: 1,
  model_version: 2,
  deployment: 3,
  strategy: 3,
}

/** Column layout by entity layer; rows stack within a column. */
export function layoutLineage(nodes: LineageNode[]): Map<string, { x: number; y: number }> {
  const rows = new Map<number, number>()
  const out = new Map<string, { x: number; y: number }>()
  for (const n of nodes) {
    const col = LAYER[n.type]
    const row = rows.get(col) ?? 0
    rows.set(col, row + 1)
    out.set(n.id, { x: col * 240, y: row * 84 })
  }
  return out
}

/** Numeric `backtest_*` metrics, when an evaluation attached a backtest. */
export function backtestMetrics(metrics: unknown): Array<[string, number]> {
  return numericMetrics(metrics).filter(([k]) => k.startsWith('backtest_'))
}

// ── Activity ─────────────────────────────────────────────────────────────────

export interface ActivityItem {
  key: string
  at: string
  text: string
}

/**
 * A feed from what the registry serves reads for: training runs, version
 * creation and deployments, joined with the model's own event log (renames,
 * promotions, rollbacks) when it has been read, newest first. A page that has
 * not read the log says so rather than implying the feed is complete.
 */
export function buildActivity(input: {
  runs: Array<{ run_id: string; kind: string; status: string; created_at: string; finished_at?: string }>
  versions: ModelVersion[]
  deployments: ModelDeployment[]
  /** `GET /api/models/{id}/events`; absent while unread, which is not the same as empty. */
  events?: Array<{ kind: string; payload?: Record<string, unknown>; created_at: string }> | null
}): ActivityItem[] {
  const items: ActivityItem[] = eventActivity(input.events)
  for (const r of input.runs) {
    const label = r.kind === 'evaluation' ? 'Evaluation' : 'Training run'
    items.push({
      key: `run:${r.run_id}`,
      at: r.finished_at ?? r.created_at,
      text: `${label} ${r.run_id.slice(0, 8)} ${r.status}`,
    })
  }
  for (const v of input.versions) {
    items.push({ key: `version:${v.version}`, at: v.created_at, text: `Version v${v.version} created` })
  }
  for (const d of input.deployments) {
    items.push({
      key: `dep:${d.deployment_id}`,
      at: d.deployed_at,
      text: `v${d.version} deployed to ${d.environment} at ${d.traffic_pct}%`,
    })
  }
  return items
    .filter((i) => Number.isFinite(Date.parse(i.at)))
    .sort((a, b) => Date.parse(b.at) - Date.parse(a.at))
}

/** What one `model_events` row says, in words. Kinds this build has no words for are shown as themselves. */
export function eventText(e: { kind: string; payload?: Record<string, unknown> }): string {
  const p = e.payload ?? {}
  const v = (k: string) => (typeof p[k] === 'number' ? `v${p[k]}` : null)
  const s = (k: string) => (typeof p[k] === 'string' ? (p[k] as string) : null)
  switch (e.kind) {
    case 'created':
      return `Model created${s('display_name') ? `: ${s('display_name')}` : ''}`
    case 'renamed':
      return `Renamed from "${s('from') ?? '?'}" to "${s('to') ?? '?'}"`
    case 'archived':
      return 'Archived'
    case 'promoted':
      return `${v('version') ?? 'A version'} promoted to ${s('alias') ?? 'production'}`
    case 'alias_set':
      return `${s('alias') ?? 'An alias'} set to ${v('version') ?? 'a version'}`
    case 'rolled_back':
    case 'rollback':
      return `${s('alias') ?? 'An alias'} rolled back from ${v('from_version') ?? '?'} to ${v('to_version') ?? '?'}`
    case 'version_created':
      return `Version ${v('version') ?? ''} created`.replace('  ', ' ')
    default:
      return e.kind.replace(/_/g, ' ')
  }
}

/** The event log as feed rows. Inference traces are not activity and are dropped even if served. */
export function eventActivity(
  events: Array<{ kind: string; payload?: Record<string, unknown>; created_at: string }> | null | undefined,
): ActivityItem[] {
  return (events ?? [])
    .filter((e) => e.kind !== 'inference_trace' && Number.isFinite(Date.parse(e.created_at)))
    .map((e, i) => ({ key: `event:${i}:${e.created_at}:${e.kind}`, at: e.created_at, text: eventText(e) }))
}

// ── Training curve ───────────────────────────────────────────────────────────

export interface CurveSeries {
  train: Array<[number, number]>
  val: Array<[number, number]>
  /** Epochs that reported a validation loss. 0 means the run had no validation split, not a perfect one. */
  validated: number
}

/** Splits the recorded points into the two lines; an unmeasured loss leaves a gap rather than a zero. */
export function curveSeries(
  points: Array<{ epoch: number; train_loss: number | null; val_loss: number | null }> | null | undefined,
): CurveSeries {
  const train: Array<[number, number]> = []
  const val: Array<[number, number]> = []
  for (const p of points ?? []) {
    if (typeof p.train_loss === 'number' && Number.isFinite(p.train_loss)) train.push([p.epoch, p.train_loss])
    if (typeof p.val_loss === 'number' && Number.isFinite(p.val_loss)) val.push([p.epoch, p.val_loss])
  }
  return { train, val, validated: val.length }
}

// ── Inference traces ─────────────────────────────────────────────────────────

export interface TraceStats {
  /** Traces read (the route caps the window), the denominator for every figure. */
  n: number
  /** p95 over real sidecar calls (`miss`); cache hits would flatter it. `null` if none. */
  p95LatencyMs: number | null
  /** errors / (hit + miss + error); abstentions are not attempts. `null` if no attempts. */
  errorRate: number | null
  abstained: number
}

/**
 * Rolls the recent `inference_trace` rows into the figures the health panel can
 * honestly show. The registry keeps no per-version rollup, so these are over the
 * window the traces route returned and say so; cost per 1k is not in a trace and
 * is not estimated.
 */
export function traceStats(traces: unknown[] | null | undefined): TraceStats | null {
  if (!traces) return null
  const rows = traces.filter((t): t is Record<string, unknown> => !!t && typeof t === 'object')
  const count = (r: string) => rows.filter((t) => t.result === r).length
  const miss = rows
    .filter((t) => t.result === 'miss' && typeof t.latency_ms === 'number')
    .map((t) => t.latency_ms as number)
    .sort((a, b) => a - b)
  const attempts = count('hit') + count('miss') + count('error')
  return {
    n: rows.length,
    p95LatencyMs: miss.length > 0 ? miss[Math.min(miss.length - 1, Math.ceil(miss.length * 0.95) - 1)] : null,
    errorRate: attempts > 0 ? count('error') / attempts : null,
    abstained: count('abstain'),
  }
}

// ── Create wizard ────────────────────────────────────────────────────────────

export interface FieldError {
  path: string
  message: string
}

/**
 * The registry answers an invalid definition with HTTP 422 and
 * `{ error: 'invalid_definition', errors: [{ path, message }] }`. Anything else
 * (a network failure, a 500) yields no field errors, so the caller falls back to
 * a general message rather than pinning a transport failure on a field.
 */
export function parseValidationErrors(err: unknown): FieldError[] {
  const data = (err as { response?: { data?: { errors?: unknown } } } | null)?.response?.data
  if (!data || !Array.isArray(data.errors)) return []
  return data.errors.flatMap((e): FieldError[] => {
    if (!e || typeof e !== 'object') return []
    const r = e as Record<string, unknown>
    return typeof r.path === 'string' && typeof r.message === 'string'
      ? [{ path: r.path, message: r.message }]
      : []
  })
}

export function errorFor(errors: FieldError[], path: string): string | null {
  const hit = errors.find((e) => e.path === path || e.path.startsWith(`${path}.`))
  return hit ? hit.message : null
}

export interface AdapterFields {
  provider: string
  model: string
  endpoint: string
  costPer1k: string
}

/** Client-side required-field check for the adapter block, keyed by field. */
export function adapterFieldErrors(f: AdapterFields): Partial<Record<keyof AdapterFields, string>> {
  const out: Partial<Record<keyof AdapterFields, string>> = {}
  if (!f.provider.trim()) out.provider = 'Provider is required.'
  if (!f.model.trim()) out.model = 'Model name is required.'
  if (!f.endpoint.trim()) out.endpoint = 'Endpoint is required.'
  else if (!/^https?:\/\//i.test(f.endpoint.trim())) out.endpoint = 'Endpoint must start with http:// or https://.'
  // Cost is a decimal STRING (ADR-0002); reject anything that is not one.
  if (f.costPer1k.trim() !== '' && !/^\d+(\.\d+)?$/.test(f.costPer1k.trim())) {
    out.costPer1k = 'Cost per 1k tokens must be a decimal number.'
  }
  return out
}

export function adapterBlock(f: AdapterFields) {
  return {
    provider: f.provider.trim(),
    model: f.model.trim(),
    endpoint: f.endpoint.trim(),
    cost_per_1k_tokens: f.costPer1k.trim() === '' ? '0.0' : f.costPer1k.trim(),
  }
}
