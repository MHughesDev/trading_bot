/**
 * N-5.4: one page per model. The scorecard is never shown without its baselines
 * (a version with none says so), a failing or not-computable rule is visible,
 * and each panel has its own loading / empty / error state.
 */
import { fireEvent, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { model, never, ok, renderRoute, verdict, version } from './testkit'

const studio = vi.hoisted(() => ({ verdict: vi.fn(), importance: vi.fn() }))
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

import { ModelCockpitPage } from '../../../pages/ModelCockpitPage'

const at = (tab = 'overview', q = '') => renderRoute(<ModelCockpitPage />, { path: '/mlops/studio/:id/*', entry: `/mlops/studio/m1/${tab}${q}` })

beforeEach(() => {
  vi.clearAllMocks()
  mlops.get.mockReturnValue(ok(model()))
  mlops.listVersions.mockReturnValue(ok({ versions: [version(1), version(2)] }))
  mlops.getAliases.mockReturnValue(ok({ production: 1 }))
  mlops.listRuns.mockReturnValue(ok({ runs: [{ run_id: 'r1', kind: 'training', status: 'succeeded' }] }))
  mlops.listDeployments.mockReturnValue(ok({ deployments: [] }))
  mlops.usedBy.mockReturnValue(ok({ strategies: [{ strategy_id: 's1', display_name: 'Trend' }] }))
  studio.verdict.mockReturnValue(ok(verdict()))
  studio.importance.mockReturnValue(ok({ available: false, reason: 'no purged folds yet' }))
})

describe('the cockpit shell', () => {
  it('shows a skeleton while the model loads', () => {
    mlops.get.mockReturnValue(never())
    at()
    expect(document.querySelector('.skel')).not.toBeNull()
  })

  it('shows an error, not an empty cockpit, when the model cannot be read', async () => {
    mlops.get.mockRejectedValue(new Error('404'))
    at()
    expect(await screen.findByRole('alert')).toHaveTextContent('This model could not be read')
  })

  it('names the model, offers every tab and moves between them', async () => {
    at()
    expect(await screen.findByRole('heading', { name: 'BTC forecaster' })).toBeInTheDocument()
    for (const t of ['Overview', 'Training', 'Test lab', 'Evaluations', 'Versions and promotion', 'Lineage']) {
      expect(screen.getByRole('tab', { name: t })).toBeInTheDocument()
    }
    fireEvent.click(screen.getByRole('tab', { name: 'Lineage' }))
    expect(await screen.findByRole('list', { name: 'Lineage links' })).toBeInTheDocument()
    expect(screen.getByRole('tab', { name: 'Lineage' })).toHaveAttribute('aria-selected', 'true')
  })
})

describe('the overview', () => {
  it('summarises the lineage and shows the current alias', async () => {
    at()
    const summary = await screen.findByTestId('lineage-summary')
    await waitFor(() => expect(summary).toHaveTextContent('Used by1 strategy'))
    expect(summary).toHaveTextContent('Versions2')
    expect(await screen.findByTestId('aliases')).toHaveTextContent('production')
    expect(screen.getByTestId('aliases')).toHaveTextContent('v1')
  })

  it('shows no alias as a statement, not a blank', async () => {
    mlops.getAliases.mockReturnValue(ok({}))
    at()
    expect(await screen.findByText('No alias set')).toBeInTheDocument()
  })

  it('shows the scorecard with its baselines and the verdict of every rule', async () => {
    at()
    expect(await screen.findByTestId('verdict')).toBeInTheDocument()
    expect(screen.getAllByTestId('baseline-row')).toHaveLength(1)
    expect(screen.getByText(/har\/rv/)).toBeInTheDocument()
    const rules = screen.getAllByTestId('usage-rule')
    expect(rules).toHaveLength(3)
    // The failing rule and the not-computable rule are both visible and both block.
    expect(rules.find((r) => r.dataset.rule === 'vol_forecast')).toHaveAttribute('data-blocking', 'true')
    expect(rules.find((r) => r.dataset.rule === 'tail_risk')).toHaveAttribute('data-blocking', 'true')
    expect(rules.find((r) => r.dataset.rule === 'return_forecast')).toHaveAttribute('data-blocking', 'false')
    expect(screen.getByTestId('verdict-headline')).toHaveTextContent('Blocked by 2 rules')
  })

  it('defaults to the production version and follows ?v=', async () => {
    at('overview', '?v=2')
    await waitFor(() => expect(studio.verdict).toHaveBeenCalledWith('m1', 2))
  })

  it('says a version with no baselines is not a comparison', async () => {
    studio.verdict.mockReturnValue(ok(verdict({ baselines: [] })))
    at()
    expect(await screen.findByTestId('no-baselines')).toHaveTextContent('no baseline rows')
  })

  it('flags unjustified complexity', async () => {
    studio.verdict.mockReturnValue(ok(verdict({ complexity: { unjustified: true, detail: 'loses to vol-timed momentum' } })))
    at()
    expect(await screen.findByTestId('complexity-unjustified')).toHaveTextContent('loses to vol-timed momentum')
  })

  it('shows a verdict error as not cleared, never as a pass', async () => {
    studio.verdict.mockRejectedValue(new Error('500'))
    at()
    const alerts = await screen.findAllByRole('alert')
    expect(alerts.map((a) => a.textContent).join(' ')).toMatch(/verdict could not be read/)
    expect(screen.queryByTestId('verdict-headline')).toBeNull()
  })

  it('says there is nothing to judge when the model has no version', async () => {
    mlops.listVersions.mockReturnValue(ok({ versions: [] }))
    mlops.getAliases.mockReturnValue(ok({}))
    at()
    expect(await screen.findByText('No version to judge')).toBeInTheDocument()
    expect(screen.getByText('No versions yet')).toBeInTheDocument()
  })

  it('shows importance clusters, and the reason when it is not available', async () => {
    at()
    expect(await screen.findByText('Not available for this version')).toBeInTheDocument()
    expect(screen.getByText('no purged folds yet')).toBeInTheDocument()
  })

  it('shows an importance interval only when the server gave one', async () => {
    studio.importance.mockReturnValue(
      ok({ available: true, importance: { folds: 1, clusters: [{ members: ['ema_21', 'ema_20'], mean: 0.01, std_err: null, interval: null }] } }),
    )
    at()
    const row = await screen.findByTestId('importance-cluster')
    expect(row).toHaveTextContent('ema_21, ema_20')
    expect(row).toHaveTextContent('not estimable from 1 fold')
  })
})
