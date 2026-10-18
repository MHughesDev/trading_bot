import type { ReactElement } from 'react'
import { render } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { MemoryRouter, Route, Routes } from 'react-router-dom'

/** Renders a page inside a plain MemoryRouter and a query client that never retries. */
export function renderRoute(element: ReactElement, opts: { path?: string; entry?: string } = {}) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: 0 } } })
  const path = opts.path ?? '/'
  return render(
    <QueryClientProvider client={qc}>
      <MemoryRouter initialEntries={[opts.entry ?? '/']}>
        <Routes>
          <Route path={path} element={element} />
          <Route path="*" element={<div data-testid="elsewhere" />} />
        </Routes>
      </MemoryRouter>
    </QueryClientProvider>,
  )
}

export const never = () => new Promise<never>(() => {})

export const ok = <T,>(data: T) => Promise.resolve({ data })

export function version(n: number, over: Record<string, unknown> = {}) {
  return {
    version: n,
    model_id: 'm1',
    status: 'candidate',
    created_at: '2026-10-01T00:00:00Z',
    metrics: {},
    ...over,
  }
}

export function model(over: Record<string, unknown> = {}) {
  return {
    model_id: 'm1',
    slug: 'm1',
    display_name: 'BTC forecaster',
    model_kind: 'forecaster',
    asset_class: 'crypto',
    definition: { model_kind: 'forecaster' },
    status: 'candidate',
    created_by: 'u',
    created_at: '2026-10-01T00:00:00Z',
    updated_at: '2026-10-02T00:00:00Z',
    ...over,
  }
}

export function verdict(over: Record<string, unknown> = {}) {
  return {
    model_id: 'm1',
    version: 2,
    allowed: false,
    rules: [
      { rule: 'return_forecast', applies: true, status: 'pass', reason: 'R2 0.02', measured: { oos_r2: 0.02 } },
      { rule: 'vol_forecast', applies: true, status: 'fail', reason: 'HARQ is better', measured: { qlike: 1.2 } },
      { rule: 'tail_risk', applies: true, status: 'not_computable', reason: 'no ES backtest', measured: {} },
    ],
    baselines: [
      { name: 'har', family: 'har', variant: 'rv', folds: [0, 1, 2], scores: { qlike: 1.0, crps: 0.5 }, contamination: null },
    ],
    complexity: null,
    ...over,
  }
}
