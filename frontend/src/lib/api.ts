import axios from 'axios'

export const api = axios.create({
  baseURL: '/',
  withCredentials: true,
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
    api.post('/auth/login', { email, password }),
  register: (email: string, password: string) =>
    api.post('/auth/register', { email, password }),
  me: () => api.get<User>('/auth/me'),
  logout: () => api.post('/auth/logout'),
  touch: () => api.post('/auth/touch'),
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
  lifecycle: (symbol: string) => api.get(`/assets/lifecycle/${symbol}`),
  init: (symbol: string, lookback_days?: number) =>
    api.post(`/assets/init/${symbol}`, lookback_days ? { lookback_days } : {}),
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
  order: (data: Record<string, unknown>) => api.post('/trade/order', data),
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
  list: () => api.get('/strategies'),
  get: (id: string) => api.get(`/strategies/${id}`),
  create: (data: Record<string, unknown>) => api.post('/strategies', data),
  update: (id: string, data: Record<string, unknown>) =>
    api.put(`/strategies/${id}`, data),
  delete: (id: string) => api.delete(`/strategies/${id}`),
}
