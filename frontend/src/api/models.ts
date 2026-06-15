// Typed client for the AI Model Studio API (Rust backend, /api/models).
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
  metrics?: Record<string, number>
  framework_version?: string
  created_at: string
}

export interface ModelRunSnapshot {
  run_id: string
  model_id: string
  kind: 'training' | 'evaluation'
  status: 'running' | 'completed' | 'failed' | 'cancelled'
  progress_pct: number
  phase: string
  metrics?: Record<string, unknown>
  error?: string
  created_at: string
  finished_at?: string
}

export interface EvaluationRun {
  eval_id: string
  model_id: string
  version: number
  dataset_id?: string
  status: string
  metrics?: Record<string, number>
  scorecard?: Scorecard
  regression_ok?: boolean
  created_at: string
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

export interface LineageGraph {
  nodes: Array<{ id: string; type: string; data: Record<string, unknown> }>
  edges: Array<{ id: string; source: string; target: string }>
}

export interface ForNodeModel {
  id: string
  slug: string
  display_name: string
  status: ModelStatus
  has_production: boolean
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
    },
  ) => client.post<{ run_id: string }>(`/api/models/${id}/train`, body),

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

  promote: (id: string, version: number, reason: string) =>
    client.post(`/api/models/${id}/versions/${version}/promote`, { reason }),

  testVersion: (
    id: string,
    version: number,
    body: { instrument_id: string; features: Record<string, number> },
  ) => client.post<unknown>(`/api/models/${id}/versions/${version}/test`, body),

  // Evaluations
  listEvals: (id: string) =>
    client.get<{ evaluations: EvaluationRun[] }>(`/api/models/${id}/evaluations`),

  compareEvals: (id: string, a: number, b: number) =>
    client.get<unknown>(`/api/models/${id}/evaluations/compare`, { params: { a, b } }),

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

  traces: (id: string) =>
    client.get<{ traces: unknown[] }>(`/api/models/${id}/traces`),

  usedBy: (id: string) =>
    client.get<{
      strategies: Array<{ id: string; name: string; status: string }>
    }>(`/api/models/${id}/used-by`),

  forNode: (kind: ModelKind, assetClass?: string) =>
    client.get<{ models: ForNodeModel[] }>('/api/models/for-node', {
      params: { kind, asset_class: assetClass },
    }),
}
