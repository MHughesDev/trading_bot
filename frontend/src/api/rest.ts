// Typed REST client for the Rust backend API.
import axios from 'axios'
import type { OrderStatus, TradingStatus } from '@/lib/types'

const client = axios.create({
  baseURL: '/',
  withCredentials: true,
})

client.interceptors.response.use(
  (r) => r,
  (err) => {
    if (err.response?.status === 401) {
      const url: string = err.config?.url ?? ''
      const isAuthCheck =
        url.includes('/auth/me') || url.includes('/auth/login')
      if (!isAuthCheck && window.location.pathname !== '/login') {
        window.location.href = '/login'
      }
    }
    return Promise.reject(err)
  },
)

export interface PlaceOrderRequest {
  instrument_id: string
  side: 'buy' | 'sell'
  order_type: 'market' | 'limit'
  qty: string
  limit_price?: string
}

export const ordersApi = {
  place: (req: PlaceOrderRequest) =>
    client.post<{ order_id: string }>('/api/orders', req),
  get: (id: string) => client.get<OrderStatus>(`/api/orders/${id}`),
}

export const tradingApi = {
  status: () => client.get<TradingStatus>('/api/trading/status'),
  kill: () => client.post('/api/trading/kill'),
  resume: () => client.post('/api/trading/resume'),
}

export const assetsApi = {
  list: () =>
    client.get<{ assets: Array<{ id: string; symbol: string }> }>('/api/assets'),
  get: (id: string) => client.get(`/api/instruments/${id}`),
}

export interface CreateSubSpec {
  lane: string
  instrument: string
  depth?: number
  max_fps?: number
}

export const subscriptionsApi = {
  create: (panel_id: string, subscribe: CreateSubSpec[]) =>
    client.post<{
      subscriptions: Array<{
        sub_id: string
        lane: string
        instrument: string
      }>
      errors: unknown[]
    }>('/api/ui/subscriptions', { panel_id, subscribe }),
}

export default client
