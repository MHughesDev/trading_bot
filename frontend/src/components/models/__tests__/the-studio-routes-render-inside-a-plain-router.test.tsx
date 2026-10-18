/** N-5.1: the one page App.tsx mounts at /mlops/studio/* renders each surface in a plain MemoryRouter. */
import { render, screen } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { MemoryRouter, Route, Routes } from 'react-router-dom'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { model, ok } from './testkit'

const studio = vi.hoisted(() => ({ fleet: vi.fn(), families: vi.fn(), verdict: vi.fn(), importance: vi.fn() }))
const mlops = vi.hoisted(() => ({
  get: vi.fn(),
  listVersions: vi.fn(),
  getAliases: vi.fn(),
  listRuns: vi.fn(),
  listDeployments: vi.fn(),
  usedBy: vi.fn(),
}))
vi.mock('@/api/modelsStudio', () => ({ studioApi: studio }))
vi.mock('@/api/mlops', () => ({ modelsApi: mlops }))
vi.mock('@xyflow/react', () => import('./xyflowStub'))

import { ModelStudioPage } from '../../../pages/ModelStudioPage'

function at(entry: string) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: 0 } } })
  return render(
    <QueryClientProvider client={qc}>
      <MemoryRouter initialEntries={[entry]}>
        <Routes>
          <Route path="/mlops/studio/*" element={<ModelStudioPage />} />
          <Route path="/mlops" element={<div data-testid="classic" />} />
        </Routes>
      </MemoryRouter>
    </QueryClientProvider>,
  )
}

beforeEach(() => {
  vi.clearAllMocks()
  studio.fleet.mockReturnValue(ok({ generated_at: 'x', counts: { models: 0, training: 0, evaluating: 0, stale: 0, serving: 0 }, models: [] }))
  studio.families.mockReturnValue(ok({ families: [] }))
  studio.verdict.mockReturnValue(ok({ model_id: 'm1', version: 1, allowed: false, rules: [], baselines: [], complexity: null }))
  studio.importance.mockReturnValue(ok({ available: false, reason: 'none' }))
  mlops.get.mockReturnValue(ok(model()))
  mlops.listVersions.mockReturnValue(ok({ versions: [] }))
  mlops.getAliases.mockReturnValue(ok({}))
  mlops.listRuns.mockReturnValue(ok({ runs: [] }))
  mlops.listDeployments.mockReturnValue(ok({ deployments: [] }))
  mlops.usedBy.mockReturnValue(ok({ strategies: [] }))
})

describe('the studio route table', () => {
  it('renders the Command Center at the index', async () => {
    at('/mlops/studio')
    expect(await screen.findByRole('heading', { name: 'Model Studio' })).toBeInTheDocument()
  })

  it('renders the wizard at /new', async () => {
    at('/mlops/studio/new')
    expect(await screen.findByRole('heading', { name: 'New model' })).toBeInTheDocument()
  })

  it('renders the cockpit for a model id, on its Overview by default', async () => {
    at('/mlops/studio/m1')
    expect(await screen.findByRole('heading', { name: 'BTC forecaster' })).toBeInTheDocument()
    expect(screen.getByRole('tab', { name: 'Overview' })).toHaveAttribute('aria-selected', 'true')
  })

  it('opens a deep-linked tab', async () => {
    at('/mlops/studio/m1/training')
    expect(await screen.findByText('No training run yet')).toBeInTheDocument()
    expect(screen.getByRole('tab', { name: 'Training' })).toHaveAttribute('aria-selected', 'true')
  })
})
