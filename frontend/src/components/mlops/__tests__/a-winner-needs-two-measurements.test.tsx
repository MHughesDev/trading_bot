/**
 * The registry's compare route scores a version with no evaluation as 0.0 and so
 * names the evaluated side the winner. The arena must not repeat that: with a
 * side missing it says there is nothing to compare, and where a confusion
 * matrix or backtest was never recorded it says so rather than drawing one.
 */
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { beforeEach, describe, expect, it, vi } from 'vitest'

const api = vi.hoisted(() => ({
  listEvals: vi.fn(),
  getAliases: vi.fn(),
  getEvalDetail: vi.fn(),
  compareEvals: vi.fn(),
}))
vi.mock('@/api/mlops', () => ({ modelsApi: api }))

import { ModelEvalsTab } from '../ModelEvalsTab'

const card = (overall: number) => ({ overall, quality: 50, speed: 50, cost: 50, safety: 50, reliability: 50 })

const snap = (id: string) => ({
  run_id: id,
  model_id: 'm1',
  kind: 'evaluation',
  status: 'succeeded',
  progress: 100,
  phase: 'done',
  created_at: '2026-10-02T00:00:00Z',
})

const detail = (id: string, version: number, over: Record<string, unknown> = {}) => ({
  eval_id: id,
  model_id: 'm1',
  version,
  status: 'succeeded',
  metrics: { val_auc: 0.5 + version / 10 },
  scorecard: card(40 + version * 10),
  regression_report: { verdict: 'ok', checks: [] },
  baseline_version: null,
  created_at: `2026-10-0${version}T00:00:00Z`,
  ...over,
})

function renderTab() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: 0 } } })
  return render(
    <QueryClientProvider client={qc}>
      <ModelEvalsTab modelId="m1" />
    </QueryClientProvider>,
  )
}

beforeEach(() => {
  vi.clearAllMocks()
  api.getAliases.mockResolvedValue({ data: { production: 1 } })
  api.listEvals.mockResolvedValue({ data: { evaluations: [snap('e1'), snap('e2')] } })
  api.getEvalDetail.mockImplementation((_m: string, id: string) =>
    Promise.resolve({ data: id === 'e1' ? detail('e1', 1) : detail('e2', 2) }),
  )
})

async function pick() {
  await screen.findByLabelText('Version A')
  await waitFor(() => expect(screen.getAllByRole('option', { name: /^v\d/ }).length).toBeGreaterThan(1))
  fireEvent.change(screen.getByLabelText('Version A'), { target: { value: '1' } })
  fireEvent.change(screen.getByLabelText('Version B'), { target: { value: '2' } })
}

describe('a winner needs two measurements', () => {
  it('pins the production row first', async () => {
    renderTab()
    await waitFor(() => expect(document.querySelectorAll('tr[data-production]').length).toBe(2))
    const body = [...document.querySelectorAll('tr[data-production]')]
    expect(body[0]).toHaveAttribute('data-production', 'yes')
    expect(body[0]).toHaveTextContent('production')
  })

  it('names the higher overall score the winner', async () => {
    api.compareEvals.mockResolvedValue({
      data: {
        versions: [
          { version: 1, metrics: { val_auc: 0.6 }, scorecard: card(50) },
          { version: 2, metrics: { val_auc: 0.7 }, scorecard: card(60) },
        ],
        winner_version: 2,
      },
    })
    renderTab()
    await pick()
    expect(await screen.findByTestId('winner-badge')).toHaveTextContent('v2 wins')
    expect(api.compareEvals).toHaveBeenCalledWith('m1', 1, 2)
  })

  it('refuses to crown a side when the other has no evaluation', async () => {
    api.compareEvals.mockResolvedValue({
      data: {
        versions: [
          { version: 1, metrics: null, scorecard: null },
          { version: 2, metrics: { val_auc: 0.7 }, scorecard: card(60) },
        ],
        winner_version: 2,
      },
    })
    renderTab()
    await pick()
    const badge = await screen.findByTestId('winner-badge')
    expect(badge).toHaveTextContent('No winner')
    expect(screen.getByText(/v1 has no completed evaluation/)).toBeInTheDocument()
  })

  it('draws a confusion matrix only when the evaluation recorded one', async () => {
    api.getEvalDetail.mockImplementation((_m: string, id: string) =>
      Promise.resolve({
        data:
          id === 'e1'
            ? detail('e1', 1)
            : detail('e2', 2, { metrics: { val_auc: 0.7, confusion_matrix: [[8, 2], [1, 9]] } }),
      }),
    )
    api.compareEvals.mockResolvedValue({
      data: {
        versions: [
          { version: 1, metrics: { val_auc: 0.6 }, scorecard: card(50) },
          { version: 2, metrics: { val_auc: 0.7 }, scorecard: card(60) },
        ],
      },
    })
    renderTab()
    await pick()
    expect(await screen.findByRole('table', { name: 'Confusion matrix' })).toBeInTheDocument()
  })

  it('says a failed list read is a failed read, not an unevaluated model', async () => {
    api.listEvals.mockRejectedValue(new Error('500'))
    renderTab()
    expect(await screen.findByText('Evaluations could not be loaded.')).toBeInTheDocument()
    expect(screen.queryByText('No evaluations yet')).not.toBeInTheDocument()
  })
})
