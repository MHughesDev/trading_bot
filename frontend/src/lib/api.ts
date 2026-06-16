import axios from 'axios'

export const api = axios.create({
  baseURL: '/',
  withCredentials: true,
})

const TOKEN_KEY = 'auth_token'
export const getStoredToken = () => localStorage.getItem(TOKEN_KEY)
export const setStoredToken = (t: string) => localStorage.setItem(TOKEN_KEY, t)
export const clearStoredToken = () => localStorage.removeItem(TOKEN_KEY)

api.interceptors.request.use((config) => {
  if (!config.headers.Authorization) {
    const token = getStoredToken()
    config.headers.Authorization = token ? `Bearer ${token}` : 'Bearer dev-local'
  }
  return config
})

api.interceptors.response.use(
  (r) => r,
  (err) => {
    if (err.response?.status === 401) {
      const url: string = err.config?.url ?? ''
      // Don't redirect for the initial session check — the auth store handles that
      const isAuthCheck = url === '/auth/me' || url.endsWith('/auth/me')
      if (!isAuthCheck && window.location.pathname !== '/login') {
        window.location.href = '/login'
      }
    }
    return Promise.reject(err)
  }
)

// ── Auth ─────────────────────────────────────────────────────────────────────

export interface User {
  id: string
  email: string
  created_at: string
}

export const authApi = {
  login: (email: string, password: string) =>
    api.post<{ token: string; user: User }>('/auth/login', { email, password }),
  register: (email: string, password: string) =>
    api.post<{ token: string; user: User }>('/auth/register', { email, password }),
  me: () => api.get<User>('/auth/me'),
  logout: () => api.post('/auth/logout'),
  touch: () => api.post('/auth/touch'),
  forgotPassword: (email: string) =>
    api.post('/auth/forgot-password', { email }),
  verifyResetCode: (email: string, code: string) =>
    api.post('/auth/verify-reset-code', { email, code }),
  resetPassword: (email: string, code: string, new_password: string) =>
    api.post('/auth/reset-password', { email, code, new_password }),
  getVenueCredentials: () => api.get('/auth/venue-credentials'),
  putVenueCredentials: (data: Record<string, string>) =>
    api.put('/auth/venue-credentials', data),
  verifyVenueCredentials: (data: Record<string, string>) =>
    api.post('/auth/venue-credentials/verify', data),
}

// ── System status ─────────────────────────────────────────────────────────────

export const systemApi = {
  status: () => api.get('/status'),
  routes: () => api.get('/routes'),
  models: () => api.get('/models'),
  params: () => api.get('/params'),
  mode: () => api.get('/system/mode'),
  setMode: (mode: string) => api.post('/system/mode', { mode }),
  microservicesHealth: () => api.get('/microservices/health'),
}

// ── Assets ────────────────────────────────────────────────────────────────────

export const assetApi = {
  initialized: () => api.get<{ assets: { symbol: string; asset_class: string }[] }>('/assets/initialized'),
  lifecycle: (symbol: string) => api.get(`/assets/lifecycle/${symbol}`),
  init: (symbol: string, lookback_days: number, asset_class?: string) =>
    api.post(`/assets/init/${symbol}`, { lookback_days, asset_class }),
  initJob: (jobId: string) => api.get(`/assets/init/jobs/${jobId}`),
  start: (symbol: string) => api.post(`/assets/lifecycle/${symbol}/start`),
  stop: (symbol: string) => api.post(`/assets/lifecycle/${symbol}/stop`),
  executionMode: (symbol: string) => api.get(`/assets/execution-mode/${symbol}`),
  setExecutionMode: (symbol: string, mode: string) =>
    api.put(`/assets/execution-mode/${symbol}`, { mode }),
  deleteExecutionMode: (symbol: string) =>
    api.delete(`/assets/execution-mode/${symbol}`),
  strategy: (symbol: string) => api.get(`/assets/strategy/${symbol}`),
  setStrategy: (symbol: string, strategy_id: string) =>
    api.put(`/assets/strategy/${symbol}`, { strategy_id }),
  deleteStrategy: (symbol: string) => api.delete(`/assets/strategy/${symbol}`),
  models: (symbol: string) => api.get(`/assets/models/${symbol}`),
  chartBars: (symbol: string, start: string, end: string, interval_seconds?: number) =>
    api.get('/assets/chart/bars', { params: { symbol, start, end, interval_seconds } }),
  tradeMarkers: (symbol: string, start?: string, end?: string) =>
    api.get('/assets/chart/trade-markers', { params: { symbol, start, end } }),
}

// ── Paper trading (internal engine) ─────────────────────────────────────────────

export interface PaperOrderView {
  order_id: string
  instrument_id: string
  asset_class: string
  side: 'buy' | 'sell'
  order_type: 'market' | 'limit' | 'stop_limit'
  qty: string
  filled_qty: string
  avg_fill_price: string | null
  status: 'new' | 'partially_filled' | 'filled' | 'cancelled' | 'rejected' | 'unknown'
  created_at: string
  updated_at: string
}

export interface PaperPositionLite {
  instrument_id: string
  quantity: string
  average_entry_price: string
}

export interface PaperInstrumentActivity {
  instrument_id: string
  orders: PaperOrderView[]
  position: PaperPositionLite | null
}

export const paperApi = {
  instrumentActivity: (instrumentId: string) =>
    api.get<PaperInstrumentActivity>(
      `/api/paper/instrument/${encodeURIComponent(instrumentId)}`,
    ),
  resetAll: () => api.post<{ ok: boolean; scope: string }>('/api/paper/reset'),
  resetAccount: (assetClass: string) =>
    api.post<{ ok: boolean; scope: string }>(
      `/api/paper/accounts/${assetClass}/reset`,
    ),
}

// ── Portfolio ─────────────────────────────────────────────────────────────────

export const portfolioApi = {
  positions: () => api.get('/portfolio/positions'),
  pnlSummary: () => api.get('/pnl/summary'),
  pnlSeries: (bucket_seconds?: number) =>
    api.get('/pnl/series', { params: { bucket_seconds } }),
  transactions: (params?: { start?: string; end?: string; symbol?: string; limit?: number }) =>
    api.get('/account/transactions', { params }),
}

// ── Trade ─────────────────────────────────────────────────────────────────────

export const tradeApi = {
  // Manual orders go through the Rust platform's risk-gated order endpoint
  // (/api/orders on :8080), not the legacy Python /trade route.
  order: (data: Record<string, unknown>) => api.post('/api/orders', data),
  flatten: (symbol: string) => api.post('/trade/flatten', { symbol }),
}

// ── Universe ──────────────────────────────────────────────────────────────────

export const universeApi = {
  alpaca: (page?: number, page_size?: number) =>
    api.get('/universe/alpaca', { params: { page, page_size } }),
  coinbase: (page?: number, page_size?: number) =>
    api.get('/universe/coinbase', { params: { page, page_size } }),
  platformSupported: () => api.get('/universe/platform-supported'),
  search: (q: string) => api.get('/universe/search', { params: { q } }),
}

// ── Strategies ────────────────────────────────────────────────────────────────

export const strategiesApi = {
  list: () => api.get('/api/strategies'),
  applyList: (assetClass?: string) =>
    api.get('/api/strategies/apply-list', {
      params: assetClass ? { asset_class: assetClass } : undefined,
    }),
  get: (id: string) => api.get(`/api/strategies/${id}/config`),
  create: (data: Record<string, unknown>) => api.post('/api/strategies', data),
  update: (id: string, data: Record<string, unknown>) =>
    api.put(`/api/strategies/${id}`, data),
  delete: (id: string) => api.delete(`/api/strategies/${id}`),
}
