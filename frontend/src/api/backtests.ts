// Typed client for the Back Testing API (Rust backend, /api/backtests).
//
// Uses the single shared `api` instance from lib/api so there is one
// token-attaching client and one 401 interceptor across the app, rather than a
// per-feature axios instance (#27).
import { api as client } from '@/lib/api'

export type BacktestStatus =
  | 'queued'
  | 'checking_data'
  | 'collecting_data'
  | 'loading_data'
  | 'simulating'
  | 'completed'
  | 'failed'
  | 'cancelled'

export interface MissingRange {
  from: string
  to: string
}

export interface DataCoverage {
  expected_bars: number
  present_bars: number
  collected_bars: number
  missing_ranges: MissingRange[]
}

export interface BacktestSnapshot {
  id: string
  name: string
  strategy_slug: string
  instrument_id: string
  venue_id: string
  asset_class: string
  timeframe: string
  start: string
  end: string
  initial_balance: string
  quote_currency: string
  auto_collect: boolean
  status: BacktestStatus
  progress: number
  error: string | null
  failed_phase: string | null
  coverage: DataCoverage | null
  result: BacktestResult | null
  created_at: string
  started_at: string | null
  finished_at: string | null
}

// Mirrors nautilus_backtest::result::BacktestResult (serde).
export interface BacktestResult {
  elapsed_time_secs?: number
  iterations?: number
  total_events?: number
  total_orders?: number
  total_positions?: number
  summary?: Record<string, string>
  stats_pnls?: Record<string, Record<string, number>>
  stats_returns?: Record<string, number>
  stats_general?: Record<string, number>
}

export interface CreateBacktestRequest {
  name?: string
  strategy_ref?: string
  definition?: Record<string, unknown>
  instrument_id: string
  venue_id?: string
  asset_class: string
  timeframe: string
  start: string
  end: string
  initial_balance: string
  quote_currency: string
  auto_collect: boolean
}

export interface BacktestListPage {
  backtests: BacktestSnapshot[]
  total: number
  limit: number
  offset: number
}

export const backtestsApi = {
  list: (params?: { limit?: number; offset?: number }) =>
    client.get<BacktestListPage>('/api/backtests', { params }),
  get: (id: string) => client.get<BacktestSnapshot>(`/api/backtests/${id}`),
  create: (req: CreateBacktestRequest) =>
    client.post<{ id: string }>('/api/backtests', req),
  stop: (id: string) => client.post(`/api/backtests/${id}/stop`),
  rerun: (id: string) =>
    client.post<{ id: string }>(`/api/backtests/${id}/rerun`),
  remove: (id: string) => client.delete(`/api/backtests/${id}`),
}

// Strategy list for the create form (also Rust backend).
export interface StrategyListItem {
  id: string
  strategy_id: string
}

export const strategyPickerApi = {
  list: () =>
    client.get<{ strategies: StrategyListItem[] }>('/api/strategies'),
}
