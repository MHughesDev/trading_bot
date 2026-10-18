/**
 * Three things the cockpit overview used to get wrong against the real routes:
 *  - used-by rows arrive as `{ strategy_id, display_name }`, so reading
 *    `{ id, name }` rendered chips with no name and a React key of `undefined`;
 *  - the scorecard is 0-100 already, and the rings multiplied it by 100;
 *  - it listed the newest evaluation of ANY version as the model's health. The
 *    figure now says which version it describes, and prefers production's.
 */
import { render, screen, waitFor } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { MemoryRouter, Route, Routes } from 'react-router-dom'
import { beforeEach, describe, expect, it, vi } from 'vitest'

const api = vi.hoisted(() => ({
  get: vi.fn(),
  getAliases: vi.fn(),
  usedBy: vi.fn(),
  listEvals: vi.fn(),
  getEvalDetail: vi.fn(),
  listVersions: vi.fn(),
  listRuns: vi.fn(),
  listDeployments: vi.fn(),
  traces: vi.fn(),
}))
vi.mock('@/api/mlops', () => ({ modelsApi: api }))
vi.mock('@/components/mlops/ModelTrainTab', () => ({ ModelTrainTab: () => null }))
vi.mock('@/components/mlops/ModelTestTab', () => ({ ModelTestTab: () => null }))
vi.mock('@/components/mlops/ForecastChartsTab', () => ({ ForecastChartsTab: () => null }))

import { ModelDetailPage } from '../../../pages/ModelDetailPage'

const card = (overall: number) => ({ overall, quality: 61, speed: 62, cost: 63, safety: 64, reliability: 65 })

beforeEach(() => {
  vi.clearAllMocks()
  api.get.mockResolvedValue({
    data: {
      model_id: 'm1',
      slug: 'alpha',
      display_name: 'Alpha',
      model_kind: 'forecaster',
      asset_class: 'crypto',
      definition: { model_kind: 'forecaster' },
      status: 'active',
      created_by: 'u',
      created_at: '2026-10-01T00:00:00Z',
      updated_at: '2026-10-01T00:00:00Z',
    },
  })
  api.getAliases.mockResolvedValue({ data: { production: 1 } })
  api.usedBy.mockResolvedValue({ data: { strategies: [{ strategy_id: 's1', display_name: 'Trend follower' }] } })
  api.listEvals.mockResolvedValue({
    data: {
      evaluations: [
        { run_id: 'e1', model_id: 'm1', kind: 'evaluation', status: 'succeeded', progress: 100, phase: 'done', created_at: '2026-10-01T00:00:00Z' },
        { run_id: 'e2', model_id: 'm1', kind: 'evaluation', status: 'succeeded', progress: 100, phase: 'done', created_at: '2026-10-03T00:00:00Z' },
      ],
    },
  })
  api.getEvalDetail.mockImplementation((_m: string, id: string) =>
    Promise.resolve({
      data:
        id === 'e1'
          ? { eval_id: 'e1', model_id: 'm1', version: 1, status: 'succeeded', scorecard: card(70), created_at: '2026-10-01T00:00:00Z' }
          : { eval_id: 'e2', model_id: 'm1', version: 2, status: 'succeeded', scorecard: card(90), created_at: '2026-10-03T00:00:00Z' },
    }),
  )
  api.listVersions.mockResolvedValue({ data: { versions: [] } })
  api.listRuns.mockResolvedValue({ data: { runs: [] } })
  api.listDeployments.mockResolvedValue({ data: { deployments: [] } })
  api.traces.mockResolvedValue({ data: { traces: [] } })
})

function renderPage() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: 0 } } })
  return render(
    <QueryClientProvider client={qc}>
      <MemoryRouter initialEntries={['/mlops/m1']}>
        <Routes>
          <Route path="/mlops/:id" element={<ModelDetailPage />} />
        </Routes>
      </MemoryRouter>
    </QueryClientProvider>,
  )
}

describe('the cockpit overview reads what the registry serves', () => {
  it('names the strategy from display_name', async () => {
    renderPage()
    expect(await screen.findByText('Trend follower')).toBeInTheDocument()
  })

  it('shows the production version health on a 0-100 scale, saying which version it is', async () => {
    renderPage()
    expect(await screen.findByText('evaluation of v1')).toBeInTheDocument()
    // production (v1) scored 70 overall; the newer v2 scored 90 and must not be shown.
    await waitFor(() => expect(screen.getByText('Overall').previousSibling).toHaveTextContent('70'))
    expect(screen.queryByText('evaluation of v2')).not.toBeInTheDocument()
    expect(screen.queryByText('7000')).not.toBeInTheDocument()
  })

  it('says latency is unmeasured when there are no traces, instead of showing 0 ms', async () => {
    renderPage()
    const stats = await screen.findByTestId('trace-stats')
    await waitFor(() => expect(stats).toHaveTextContent(/not measured/))
    expect(stats).not.toHaveTextContent('0 ms')
  })

  it('reports measured latency and error rate from the traces served', async () => {
    api.traces.mockResolvedValue({
      data: {
        traces: [
          { result: 'miss', latency_ms: 120 },
          { result: 'miss', latency_ms: 80 },
          { result: 'error', latency_ms: 5 },
        ],
      },
    })
    renderPage()
    const stats = await screen.findByTestId('trace-stats')
    await waitFor(() => expect(stats).toHaveTextContent('120 ms'))
    expect(stats).toHaveTextContent('33.3%')
  })
})
