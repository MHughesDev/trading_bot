// Typed client for the Model Studio routes (Set N phase 5).
//
// Plain TypeScript types, no runtime schema: the shapes are the ones
// `crates/api/src/routes/models_studio.rs` serves. Every path here resolves to a
// route in `crates/api/src/routes/mod.rs`; a path the server does not serve
// fails `every_client_path_is_served`. Everything else the studio reads
// (versions, runs, curve, test cases, lineage...) goes through `modelsApi` in
// `./mlops`, which this file imports and does not rewrite.
import { api as client } from '@/lib/api'
import type { TrainDataSelection } from './mlops'

// ── dataset declaration (mirror of crates/model-registry/src/dataset_declaration.rs) ──

export type Timeframe = '1m' | '5m' | '15m' | '30m' | '1h' | '4h' | '1d'
export type Scaler = 'none' | 'per_fold_standard' | 'per_fold_robust'
export type BarrierPath = 'high_low' | 'close'
export type WalkMode = 'expanding' | 'rolling'

export type Universe = string[] | { universe: string; as_of_membership: boolean }

export type Sampling =
  | { kind: 'every_bar' }
  | { kind: 'cusum'; h_sigma: number }
  | { kind: 'strategy_events'; strategy_version: string }

export type Target =
  | { kind: 'forward_return'; horizon_bars: number; log: boolean }
  | { kind: 'realized_vol'; horizon_bars: number }
  | { kind: 'triple_barrier'; pt_sigma: number; sl_sigma: number; max_bars: number; path: BarrierPath }
  | {
      kind: 'meta_label'
      strategy_version: string
      pt_sigma: number
      sl_sigma: number
      max_bars: number
      path: BarrierPath
    }
  | { kind: 'rank'; horizon_bars: number }
  | { kind: 'direction'; horizon_bars: number }

export type Weights =
  | { kind: 'uniqueness' }
  | { kind: 'time_decay'; half_life_days: number }
  | { kind: 'none' }

export type Cv =
  | { kind: 'walk_forward'; train_days: number; test_days: number; refit_days: number; mode: WalkMode }
  | { kind: 'purged_kfold'; k: number }
  | { kind: 'cpcv'; n: number; k: number }

/// There is deliberately no purge and no embargo here: the server derives both
/// and `deny_unknown_fields` refuses a request that sends one.
export interface DatasetDeclaration {
  universe: Universe
  timeframe: Timeframe
  window: { start: string; end: string }
  features: string[]
  transforms: { winsorize?: number | null; scaler: Scaler; lags: number[] }
  sampling: Sampling
  target: Target
  weights: Weights
  sequence?: { window: number; stride: number } | null
  cv: Cv
}

export interface DerivedFolds {
  purge_bars: number
  embargo_bars: number
  horizon_bars: number
  lookback_bars: number
  sample_estimate?: number
}

export interface ValidateOk {
  ok: true
  declaration_hash: string
  derived: DerivedFolds
  clip: null | { requested_end: string; clipped_end: string }
  lowering: {
    ok: boolean
    unsupported: Array<{ field: string; why: string }>
    trainer_fields: string[]
  }
}

export interface ValidateRefused {
  ok: false
  errors: Array<{ field: string; message: string }>
}

export type ValidateResult = ValidateOk | ValidateRefused

// ── families ──

export interface ModelFamily {
  family: string
  variant: string
  framework: string | null
  role: 'baseline' | 'candidate' | 'foundation_baseline'
  targets: string[]
  outputs: string[]
  needs_sequence: boolean
  uses_gpu: boolean
  requires: string[]
  selectable_as_candidate: boolean
  entrypoint: string
  description: string
  contamination: null | { model_cutoff: string }
}

// ── fleet ──

export interface FleetActiveRun {
  run_id: string
  kind: string
  status: string
  progress: number
  phase: string
}

export interface FleetModel {
  model_id: string
  display_name: string
  model_kind: string
  status: string
  family: string | null
  production_version: number | null
  latest_version: number | null
  active_run: null | FleetActiveRun
  last_trained_at: string | null
  stale: boolean
  stale_reason: string | null
  serving: boolean
}

export interface Fleet {
  generated_at: string
  counts: { models: number; training: number; evaluating: number; stale: number; serving: number }
  models: FleetModel[]
}

// ── verdict, promotion ──

export type RuleName = 'return_forecast' | 'vol_forecast' | 'tail_risk'
export type RuleStatus = 'pass' | 'fail' | 'not_computable' | 'not_applicable'

export interface UsageRule {
  rule: RuleName
  applies: boolean
  status: RuleStatus
  reason: string
  measured: Record<string, unknown>
}

export interface BaselineRow {
  name: string
  family: string
  variant: string
  folds: number[]
  scores: Record<string, unknown>
  contamination: null | Record<string, unknown>
}

export interface Verdict {
  model_id: string
  version: number
  allowed: boolean
  rules: UsageRule[]
  baselines: BaselineRow[]
  complexity: null | { unjustified: boolean; detail: string }
}

export type ProposalStatus = 'pending' | 'approved' | 'rejected'

export interface PromotionProposal {
  proposal_id: string
  model_id: string
  version: number
  environment: string
  rationale: string
  status: ProposalStatus
  proposed_by: string
  proposed_at: string
  decided_by: string | null
  decided_at: string | null
  decision_note: string | null
  performed_at: string | null
  effect: Record<string, unknown> | null
}

export interface PromotionEvidence {
  model_id: string
  version: number
  environment: string
  gate_checks: Array<{ check: string; passed: boolean; required: boolean }>
  gate_passed: boolean
  verdict: Verdict
  aliases: Record<string, unknown>
  pending_proposals: PromotionProposal[]
}

export interface Importance {
  available: boolean
  reason?: string
  importance?: {
    folds: number
    clusters: Array<{
      members: string[]
      mean: number
      std_err: number | null
      interval: [number, number] | null
    }>
  }
}

export interface StudioTrainBody {
  dataset_id?: string
  version_note?: string
  hyperparams?: Record<string, unknown>
  data?: TrainDataSelection
  declaration?: DatasetDeclaration
  family?: { family: string; variant: string }
}

export const studioApi = {
  families: () => client.get<{ families: ModelFamily[] }>('/api/models/studio/families'),

  validateDeclaration: (body: { declaration: DatasetDeclaration; cutoff?: string }) =>
    client.post<ValidateResult>('/api/models/declarations/validate', body, {
      // A 422 is an answer ({ok:false, errors}), not a transport failure.
      validateStatus: (s) => s === 200 || s === 422,
    }),

  fleet: () => client.get<Fleet>('/api/models/studio/fleet'),

  verdict: (id: string, version: number) =>
    client.get<Verdict>(`/api/models/${id}/versions/${version}/verdict`),

  promotionEvidence: (id: string, version: number) =>
    client.get<PromotionEvidence>(`/api/models/${id}/versions/${version}/promotion-evidence`),

  propose: (id: string, version: number, body: { environment?: string; rationale: string }) =>
    client.post<{ proposal_id: string; status: 'pending' }>(
      `/api/models/${id}/versions/${version}/promotion-proposals`,
      body,
    ),

  proposals: (id: string) =>
    client.get<{ proposals: PromotionProposal[] }>(`/api/models/${id}/promotion-proposals`),

  decide: (proposalId: string, body: { decision: 'approved' | 'rejected'; note: string }) =>
    client.post<unknown>(`/api/models/promotion-proposals/${proposalId}/decision`, body),

  importance: (id: string, version: number) =>
    client.get<Importance>(`/api/models/${id}/versions/${version}/importance`),

  train: (id: string, body: StudioTrainBody) =>
    client.post<{ run_id: string }>(`/api/models/${id}/train`, body),
}
