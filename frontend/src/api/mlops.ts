// Typed client for the ML Ops API (Rust backend, /api/models).
//
// Uses the single shared `api` instance from lib/api so there is one
// token-attaching client and one 401 interceptor across the app.
import { api as client } from '@/lib/api'

export type ModelStatus =
  | 'draft'
  | 'training'
  | 'evaluating'
  | 'candidate'
  | 'active'
  | 'archived'
  | 'failed'

export type ModelKind =
  | 'forecaster'
  | 'signal_ranker'
  | 'trade_decision'
  | 'risk_sizing'
  | 'embedding'
  | 'external_llm_adapter'

export interface ModelDefinition {
  schema_version?: string
  model_kind: ModelKind
  display_name?: string
  framework?: string
  runtime?: 'python' | 'rust'
  asset_class?: string
  auto_retrain?: boolean
  [key: string]: unknown
}

// Mirror the Rust ModelRecord
export interface AiModel {
  model_id: string
  slug: string
  display_name: string
  description?: string
  model_kind: ModelKind
  asset_class: string
  definition: ModelDefinition
  status: ModelStatus
  created_by: string
  created_at: string
  updated_at: string
}

export interface ModelVersion {
  version: number
  model_id: string
  status: ModelStatus
  artifact_uri?: string
  artifact_hash?: string
  size_bytes?: number
  version_note?: string
  // Mixed-type: numeric scores plus objective/arch strings, feature_importance
  // objects, best_iteration null, etc. Consumers must guard before formatting.
  metrics?: Record<string, unknown>
  framework_version?: string
  created_at: string
}

export interface ModelRunSnapshot {
  run_id: string
  model_id: string
  kind: 'training' | 'evaluation'
  status: 'queued' | 'running' | 'succeeded' | 'failed' | 'cancelled'
  progress: number
  phase: string
  metrics?: Record<string, unknown>
  error?: string
  created_at: string
  finished_at?: string
}

/// One persisted evaluation, as `GET /api/models/{id}/evaluations/{eval_id}`
/// returns it. NB the *list* route returns run snapshots (`ModelRunSnapshot`),
/// not this: they carry no `version`, so the version only comes from this read.
export interface EvaluationRun {
  eval_id: string
  model_id: string
  version: number
  status: string
  metrics?: Record<string, unknown> | null
  scorecard?: Scorecard | null
  regression_report?: RegressionReport | null
  baseline_version?: number | null
  created_at: string
}

export interface RegressionCheck {
  metric: string
  baseline?: number | null
  candidate?: number | null
  delta?: number
  delta_pct?: number
  verdict: string
}

export interface RegressionReport {
  verdict: string
  tolerance_pct?: number
  checks?: RegressionCheck[]
}

/// One check the server's promotion gate evaluated.
export interface GateCheck {
  check: string
  passed: boolean
  required: boolean
}

/// What `POST .../promote` answers. A failed gate is HTTP 200 with
/// `promoted: false`, so a caller that only checks the status code reports a
/// promotion that did not happen.
export interface PromoteResult {
  promoted: boolean
  reason?: string
  version?: number
  alias?: string
  gate_checks?: GateCheck[]
  override_reason?: string | null
}

export interface Scorecard {
  overall: number
  quality: number
  speed: number
  cost: number
  safety: number
  reliability: number
}

export interface AliasMap {
  [alias: string]: number // alias name -> version number
}

export interface ModelDeployment {
  deployment_id: string
  version: number
  environment: string
  alias: string
  status: string
  traffic_pct: number
  deployed_at: string
}

/// The lineage route currently answers `{ model_id, aliases, note }` with no
/// `nodes`/`edges`; both are optional so a caller cannot read them as present.
export interface LineageGraph {
  nodes?: Array<{ id: string; type: string; data: Record<string, unknown> }>
  edges?: Array<{ id: string; source: string; target: string }>
  aliases?: AliasMap
  note?: string
}

export interface ForNodeModel {
  id: string
  slug: string
  display_name: string
  status: ModelStatus
  has_production: boolean
}

/// A single model forecast, as returned by the inference service.
export interface Forecast {
  direction: 'up' | 'down' | 'flat'
  magnitude: string
  confidence: number
  horizon: string
}

/// Response from a Test Lab inference run.
export interface TestInferenceResponse {
  model_id: string
  version: number
  kind: string
  predictions?: Forecast[]
  latency_ms: number
  // LLM-kind responses carry text instead of predictions.
  text?: string
  tokens?: number
  cost_usd?: string
  trace_id?: string
}

/// The model's expected feature schema + (optionally) a computed sample vector.
export interface FeatureVectorResponse {
  feature_set: string
  feature_order: string[]
  features: Record<string, number>
  source: 'computed' | 'schema'
  instrument_id?: string
  timeframe?: string
  as_of_ms?: number
  bars_used?: number
}

/// A saved Test Lab case.
export interface TestCase {
  case_id: string
  name: string
  input: { instrument_id?: string; features?: Record<string, number> }
  expected?: unknown
  created_at: string
}

/// One (instrument, timeframe) pair with stored-bar coverage in ClickHouse.
export interface MarketInstrument {
  instrument_id: string
  timeframe: string
  bars: number
  first_ms: number
  last_ms: number
}

/// Data selection sent with a training run — which real bars to train on.
export interface TrainDataSelection {
  instruments: string[]
  timeframe: string
  lookback_days: number
  feature_set_ref?: string
  label_horizon?: string
}

export const modelsApi = {
  list: (params?: { kind?: ModelKind; status?: ModelStatus; asset_class?: string; q?: string }) =>
    client.get<{ models: AiModel[]; total: number }>('/api/models', { params }),

  get: (id: string) => client.get<AiModel>(`/api/models/${id}`),

  create: (body: {
    definition: ModelDefinition
    display_name: string
    description?: string
  }) => client.post<{ model_id: string; slug: string }>('/api/models', body),

  patch: (id: string, body: { display_name?: string; description?: string }) =>
    client.patch<AiModel>(`/api/models/${id}`, body),

  delete: (id: string) => client.delete(`/api/models/${id}`),

  archive: (id: string) => client.post(`/api/models/${id}/archive`),

  // Training
  startTrain: (
    id: string,
    body: {
      dataset_id?: string
      version_note?: string
      hyperparams?: Record<string, unknown>
      data?: TrainDataSelection
    },
  ) => client.post<{ run_id: string }>(`/api/models/${id}/train`, body),

  // Market data coverage — what instruments/timeframes have stored bars.
  marketInstruments: () =>
    client.get<{ instruments: MarketInstrument[] }>('/api/market/instruments'),

  listRuns: (id: string) =>
    client.get<{ runs: ModelRunSnapshot[] }>(`/api/models/${id}/runs`),

  getRun: (id: string, runId: string) =>
    client.get<ModelRunSnapshot>(`/api/models/${id}/runs/${runId}`),

  cancelRun: (id: string, runId: string) =>
    client.post(`/api/models/${id}/runs/${runId}/cancel`),

  // Versions
  listVersions: (id: string) =>
    client.get<{ versions: ModelVersion[] }>(`/api/models/${id}/versions`),

  evaluate: (id: string, version: number) =>
    client.post(`/api/models/${id}/versions/${version}/evaluate`),

  // The server reads `environment` and `override_reason`. This used to send
  // `reason`, which the server ignores, so an override was never logged.
  promote: (
    id: string,
    version: number,
    body: { environment?: 'paper' | 'live'; override_reason?: string },
  ) => client.post<PromoteResult>(`/api/models/${id}/versions/${version}/promote`, body),

  // Test Lab inference — the backend expects a list of instances.
  testVersion: (
    id: string,
    version: number,
    // An LLM adapter reads `prompt`; every other kind reads instrument + features.
    instances: Array<{ instrument_id?: string; features?: Record<string, number>; prompt?: string }>,
  ) =>
    client.post<TestInferenceResponse>(`/api/models/${id}/versions/${version}/test`, {
      instances,
    }),

  // Compute / fetch the model's expected feature vector for prefill.
  featureVector: (
    id: string,
    params?: { instrument_id?: string; timeframe?: string },
  ) =>
    client.get<FeatureVectorResponse>(`/api/models/${id}/feature-vector`, { params }),

  // Saved test cases.
  listTestCases: (id: string) =>
    client.get<{ test_cases: TestCase[] }>(`/api/models/${id}/test-cases`),

  addTestCase: (
    id: string,
    body: { name: string; input: unknown; expected?: unknown },
  ) => client.post<{ case_id: string }>(`/api/models/${id}/test-cases`, body),

  deleteTestCase: (id: string, caseId: string) =>
    client.delete(`/api/models/${id}/test-cases/${caseId}`),

  // Evaluations
  // Run snapshots of the evaluations this server process is holding (in memory).
  listEvals: (id: string) =>
    client.get<{ evaluations: ModelRunSnapshot[] }>(`/api/models/${id}/evaluations`),

  // The server reads `versions=a,b`; `a`/`b` params were silently ignored.
  compareEvals: (id: string, a: number, b: number) =>
    client.get<unknown>(`/api/models/${id}/evaluations/compare`, {
      params: { versions: `${a},${b}` },
    }),

  getEval: (id: string, evalId: string) =>
    client.get<EvaluationRun>(`/api/models/${id}/evaluations/${evalId}`),

  // Aliases & deployments
  getAliases: (id: string) => client.get<AliasMap>(`/api/models/${id}/aliases`),

  rollback: (id: string, alias: string) =>
    client.post(`/api/models/${id}/aliases/${alias}/rollback`),

  listDeployments: (id: string) =>
    client.get<{ deployments: ModelDeployment[] }>(`/api/models/${id}/deployments`),

  createDeployment: (
    id: string,
    body: { version: number; environment: string; alias: string; traffic_pct: number },
  ) => client.post<unknown>(`/api/models/${id}/deployments`, body),

  // Other
  lineage: (id: string) => client.get<LineageGraph>(`/api/models/${id}/lineage`),

  getEvalDetail: (id: string, evalId: string) =>
    client.get<EvaluationRun>(`/api/models/${id}/evaluations/${evalId}`),

  traces: (id: string, limit?: number) =>
    client.get<{ traces: unknown[] }>(`/api/models/${id}/traces`, { params: { limit } }),

  /** The recorded per-epoch loss curve of a run (`mlops.training_metric`). `null` loss is not measured. */
  trainingCurve: (id: string, runId: string) =>
    client.get<{ run_id: string; points: CurvePoint[] }>(`/api/models/${id}/runs/${runId}/curve`),

  /** The model's event log (created, promoted, rolled back, archived, renamed), newest first. */
  events: (id: string, limit?: number) =>
    client.get<{ model_id: string; events: ModelEvent[] }>(`/api/models/${id}/events`, { params: { limit } }),

  // Rows are `{ strategy_id, display_name }`; normalise with `normaliseUsedBy`.
  usedBy: (id: string) =>
    client.get<{ strategies: unknown[] }>(`/api/models/${id}/used-by`),

  forNode: (kind: ModelKind, assetClass?: string) =>
    client.get<{ models: ForNodeModel[] }>('/api/models/for-node', {
      params: { kind, asset_class: assetClass },
    }),
}

/// Pickers for the AI inference block. The target resolves to a single forecast
/// through the inference gateway.
export const inferenceTargetsApi = {
  models: (assetClass?: string) => modelsApi.forNode('forecaster', assetClass),
}

export interface CurvePoint {
  epoch: number
  train_loss: number | null
  val_loss: number | null
}

export interface ModelEvent {
  kind: string
  payload: Record<string, unknown>
  actor: string | null
  created_at: string
}
