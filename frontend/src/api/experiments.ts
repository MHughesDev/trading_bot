// Typed client for the Backtest Suite API (Set J, Rust backend, /api/backtest/*).
//
// Uses the single shared `api` instance from lib/api so there is one
// token-attaching client and one 401 interceptor across the app (#27). The types
// mirror `backtest::suite::*` view models field-for-field.
import { api as client } from '@/lib/api'

export type ExperimentState = 'candidate' | 'validated' | 'live' | 'decaying' | 'retired'

export type StudyKind =
  | 'parameter_sweep'
  | 'walk_forward'
  | 'cpcv'
  | 'nested_cv'
  | 'permutation_null'
  | 'synthetic_paths'
  | 'cost_sweep'
  | 'trade_monte_carlo'
  | 'regime_conditional'
  | 'neighborhood'

export type NullKind =
  | 'signal_return_decouple'
  | 'block_permutation'
  | 'stationary_bootstrap'
  | 'bar_permutation'
  | 'synthetic_garch'
  | 'regime_block'
  | 'random_entry_matched'

export type SelectionRule = 'none' | 'median_stable_centroid' | 'worst_case_robust'

export type Gate = 'integrity' | 'single_path' | 'robustness' | 'significance' | 'vault'
export type GateStatus = 'locked' | 'ready' | 'passed' | 'failed'

export interface ExperimentView {
  id: string
  experiment_id: string
  strategy_family: string
  strategy_type: string
  state: ExperimentState
  trial_counter: number
  unsafe: boolean
  gate3_passed: boolean
  primary_test: string
  holdout_spent: boolean
  study_count: number
  created: string
  updated: string
}

export interface Distribution {
  metric: string
  dist: number[]
  median: number
  iqr: [number, number]
  worst_5pct: number
  spread: number
}

export interface StudyVerdict {
  summary: string
  positive_median: boolean
  survivable_worst5: boolean
  plateau: boolean | null
}

export interface StudyView {
  study_id: string
  kind: StudyKind
  metric: string
  question: string
  trial_delta: number
  sealed: boolean
  distribution: Distribution
  verdict: StudyVerdict
  // Provenance only — insertion order, never metric-ranked (INV-2).
  members: string[]
  selection_rule: SelectionRule
  carried_forward: boolean
  unsafe: boolean
}

export interface SignificanceView {
  p_value: number
  null_id: string
  null_kind: NullKind
  preserves: string[]
  destroys: string[]
  trial_count_at_eval: number
  raw_p_value: number
  deflated_sharpe: number
  pbo: number
  corroborators_agree: boolean
}

export interface GateView {
  gate: Gate
  status: GateStatus
  summary: string | null
  evidence: string[]
  at: string | null
}

export interface FunnelView {
  gates: GateView[]
  significance: SignificanceView | null
}

export interface NullCatalogEntry {
  kind: NullKind
  preserves: string[]
  destroys: string[]
  recommended: boolean
}

export interface NullChoiceView {
  null_id: string
  kind: NullKind
  preserves: string[]
  destroys: string[]
  recommended: NullKind
  was_override: boolean
  override_reason: string | null
  chosen_at: string
}

export interface NullPickerView {
  recommended: NullKind
  catalog: NullCatalogEntry[]
  chosen: NullChoiceView | null
}

export interface VaultAccessView {
  when: string
  run_id: string
  by: string
}

export interface VaultView {
  spent: boolean
  gate3_passed: boolean
  unsafe: boolean
  can_run: boolean
  access_log: VaultAccessView[]
}

export interface ReconciliationPoint {
  realized: number
  percentile: number
  below_worst_5pct: boolean
}

export interface ReconciliationView {
  verdict: {
    points: ReconciliationPoint[]
    fraction_below_worst5: number
    drifting: boolean
  }
  state: ExperimentState
}

export interface SuiteCalibrationView {
  calibration: {
    n_points: number
    mean_percentile: number
    worst5_coverage: number
    optimistic: boolean
  }
  percentiles: number[]
  experiments_contributing: number
}

export interface CreateExperimentRequest {
  experiment_id: string
  strategy_family: string
  strategy_type: string
  universe_ref: string
  research_start: string
  research_end: string
  holdout_start: string
  holdout_end: string
  eval_resolution?: string
}

export type VarySpec =
  | { vary: 'params'; grid: Record<string, unknown>[] }
  | { vary: 'neighborhood'; param: string; center: number; step: number; k: number }
  | { vary: 'data_windows'; windows: [string, string][] }
  | { vary: 'cpcv_groups'; n_groups: number; k_test: number }
  | { vary: 'seeds'; n: number }
  | { vary: 'cost_ladder'; cost_model_refs: string[] }
  | { vary: 'trade_resamples'; n: number; block: number }
  | { vary: 'regimes'; windows: [string, string, string][] }

export interface RunStudyRequest {
  study_id: string
  kind: StudyKind
  vary: VarySpec
  metric: string
  question: string
  selection_rule?: SelectionRule
  null_ref?: string
}

export const experimentsApi = {
  list: () => client.get<{ experiments: ExperimentView[] }>('/api/backtest/experiments'),
  get: (id: string) => client.get<ExperimentView>(`/api/backtest/experiments/${id}`),
  create: (req: CreateExperimentRequest) =>
    client.post<ExperimentView>('/api/backtest/experiments', req),
  promote: (id: string) => client.post<ExperimentView>(`/api/backtest/experiments/${id}/promote`),
  retire: (id: string) => client.post<ExperimentView>(`/api/backtest/experiments/${id}/retire`),

  listStudies: (id: string) =>
    client.get<{ studies: StudyView[] }>(`/api/backtest/experiments/${id}/studies`),
  runStudy: (id: string, req: RunStudyRequest) =>
    client.post<StudyView>(`/api/backtest/experiments/${id}/studies`, req),

  nullPicker: (id: string) =>
    client.get<NullPickerView>(`/api/backtest/experiments/${id}/nulls`),
  chooseNull: (id: string, kind: NullKind, override_reason?: string) =>
    client.post<NullChoiceView>(`/api/backtest/experiments/${id}/nulls`, { kind, override_reason }),

  funnel: (id: string) => client.get<FunnelView>(`/api/backtest/experiments/${id}/funnel`),
  advanceFunnel: (id: string) =>
    client.post<FunnelView>(`/api/backtest/experiments/${id}/funnel/advance`),

  vault: (id: string) => client.get<VaultView>(`/api/backtest/experiments/${id}/vault`),
  runVault: (id: string) => client.post<VaultView>(`/api/backtest/experiments/${id}/vault`),

  reconcile: (id: string, realized: number[], drift_threshold = 0.1) =>
    client.post<ReconciliationView>(`/api/backtest/experiments/${id}/reconcile`, {
      realized,
      drift_threshold,
    }),
  calibration: () => client.get<SuiteCalibrationView>('/api/backtest/calibration'),
}
