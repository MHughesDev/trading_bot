/**
 * The command center's honesty rules (H-5.2): a model with no recorded metric
 * shows a dash and never `0.0%`; a failed list read is an error, not the
 * "No models yet" empty state; used-by is unknown until its read answers; and
 * the cards/table toggle persists.
 */
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { MemoryRouter } from 'react-router-dom'
import { beforeEach, describe, expect, it, vi } from 'vitest'

const api = vi.hoisted(() => ({
  list: vi.fn(),
  listVersions: vi.fn(),
  getAliases: vi.fn(),
  usedBy: vi.fn(),
}))
vi.mock('@/api/mlops', () => ({ modelsApi: api }))
vi.mock('@/components/mlops/PlatformHealthPanel', () => ({ PlatformHealthPanel: () => null }))
vi.mock('@/components/mlops/FalsificationPanel', () => ({ FalsificationPanel: () => null }))

import { MlOpsPage } from '../../../pages/MlOpsPage'
import { VIEW_MODE_KEY } from '../studio.helpers'

const model = (id: string, name: string, over: Record<string, unknown> = {}) => ({
  model_id: id,
  slug: id,
  display_name: name,
  model_kind: 'forecaster',
  asset_class: 'crypto',
  definition: { model_kind: 'forecaster' },
  status: 'active',
  created_by: 'u',
  created_at: '2026-10-01T00:00:00Z',
  updated_at: '2026-10-02T00:00:00Z',
  ...over,
})

function renderPage() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: 0 } } })
  return render(
    <QueryClientProvider client={qc}>
      <MemoryRouter>
        <MlOpsPage />
      </MemoryRouter>
    </QueryClientProvider>,
  )
}

beforeEach(() => {
  vi.clearAllMocks()
  window.localStorage.clear()
  api.listVersions.mockResolvedValue({
    data: {
      versions: [
        { version: 1, model_id: 'a', status: 'candidate', metrics: { val_auc: 0.5 }, created_at: '2026-10-01T00:00:00Z' },
        { version: 2, model_id: 'a', status: 'active', metrics: { val_auc: 0.7 }, created_at: '2026-10-02T00:00:00Z' },
      ],
    },
  })
  api.getAliases.mockResolvedValue({ data: { production: 2 } })
  api.usedBy.mockResolvedValue({ data: { strategies: [{ strategy_id: 's1', display_name: 'Trend follower' }] } })
})

describe('the command center reads the registry', () => {
  it('renders a card with status, headline metric, version and used-by chip', async () => {
    api.list.mockResolvedValue({ data: { models: [model('a', 'Alpha')], total: 1 } })
    renderPage()
    const card = await screen.findByTestId('model-card')
    expect(card).toHaveTextContent('Alpha')
    expect(card).toHaveTextContent('Active')
    await waitFor(() => expect(card).toHaveTextContent('70.0%'))
    expect(card).toHaveTextContent('v2')
    await waitFor(() => expect(card).toHaveTextContent('Trend follower'))
  })

  it('shows a dash, not 0.0%, for a model with no recorded metric', async () => {
    api.list.mockResolvedValue({ data: { models: [model('b', 'Bare')], total: 1 } })
    api.listVersions.mockResolvedValue({ data: { versions: [] } })
    api.getAliases.mockResolvedValue({ data: {} })
    api.usedBy.mockResolvedValue({ data: { strategies: [] } })
    renderPage()
    const card = await screen.findByTestId('model-card')
    await waitFor(() => expect(card).toHaveTextContent('not used by any strategy'))
    expect(card).toHaveTextContent('—')
    expect(card).not.toHaveTextContent('0.0%')
    expect(card).toHaveTextContent('no history yet')
  })

  it('is an error, not the empty state, when the list read fails', async () => {
    api.list.mockRejectedValue(new Error('500'))
    renderPage()
    expect(await screen.findByText('The model list could not be loaded.')).toBeInTheDocument()
    expect(screen.queryByText('No models yet')).not.toBeInTheDocument()
  })

  it('shows the designed empty state for a registry with no models', async () => {
    api.list.mockResolvedValue({ data: { models: [], total: 0 } })
    renderPage()
    expect(await screen.findByText('No models yet')).toBeInTheDocument()
  })

  it('persists the table toggle and restores it on the next visit', async () => {
    api.list.mockResolvedValue({ data: { models: [model('a', 'Alpha')], total: 1 } })
    const first = renderPage()
    await screen.findByTestId('model-card')
    fireEvent.click(screen.getByRole('button', { name: 'Table view' }))
    expect(window.localStorage.getItem(VIEW_MODE_KEY)).toBe('table')
    expect(await screen.findByRole('table')).toBeInTheDocument()
    first.unmount()

    renderPage()
    expect(await screen.findByRole('table')).toBeInTheDocument()
    expect(screen.queryByTestId('model-card')).not.toBeInTheDocument()
  })

  it('passes the filters to the list route', async () => {
    api.list.mockResolvedValue({ data: { models: [], total: 0 } })
    renderPage()
    await screen.findByText('No models yet')
    fireEvent.change(screen.getByLabelText('Kind'), { target: { value: 'embedding' } })
    await waitFor(() =>
      expect(api.list).toHaveBeenLastCalledWith(expect.objectContaining({ kind: 'embedding' })),
    )
    expect(await screen.findByText('No models match these filters.')).toBeInTheDocument()
  })
})
