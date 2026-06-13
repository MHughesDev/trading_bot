// Typed client for the Back Testing API (Rust backend, /api/backtests).
import axios from 'axios'

// Dedicated instance: attaches the M-17 placeholder bearer token the Rust
// API requires (any non-empty token, loopback only) — same convention as
// lib/api.ts.
const client = axios.create({ baseURL: '/', withCredentials: true })
client.interceptors.request.use((config) => {
  if (!config.headers.Authorization) {
    config.headers.Authorization = 'Bearer dev-local'
  }
  return config
})

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

export const backtestsApi = {
  list: () =>
    client.get<{ backtests: BacktestSnapshot[] }>('/api/backtests'),
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
