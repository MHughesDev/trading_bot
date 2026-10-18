/**
 * N-5.9: dataset -> features -> model -> prediction series -> backtest, by
 * content hash. Versions that name the same hash share one node; a link the
 * registry did not record is a visible gap, not an invented node.
 */
import { fireEvent, screen } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { buildHashLineage } from '@/lib/models/lineage'
import { never, ok, renderRoute, version } from './testkit'

const mlops = vi.hoisted(() => ({
  get: vi.fn(),
  listVersions: vi.fn(),
  getAliases: vi.fn(),
  listRuns: vi.fn(),
  listDeployments: vi.fn(),
  usedBy: vi.fn(),
}))
vi.mock('@/api/modelsStudio', () => ({ studioApi: {} }))
vi.mock('@/api/mlops', () => ({ modelsApi: mlops }))
vi.mock('@xyflow/react', () => import('./xyflowStub'))

import { LineageGraphView } from '../LineageGraphView'
import { ModelLineagePage } from '../../../pages/ModelLineagePage'

const full = (n: number, over: Record<string, unknown> = {}) =>
  version(n, {
    artifact_hash: `art${n}aaaaaaaaaaaaaaaa`,
    metrics: { dataset_id: 'ds1111111111111111', feature_dag_hash: 'ft2222222222222222', prediction_series_id: `ps${n}3333333333333333`, backtest_sharpe: 1.1 },
    ...over,
  })

beforeEach(() => {
  vi.clearAllMocks()
  mlops.get.mockReturnValue(ok({ model_id: 'm1', display_name: 'M' }))
  mlops.listVersions.mockReturnValue(ok({ versions: [full(1), full(2)] }))
  mlops.getAliases.mockReturnValue(ok({ production: 2 }))
  mlops.listRuns.mockReturnValue(ok({ runs: [] }))
  mlops.listDeployments.mockReturnValue(ok({ deployments: [] }))
  mlops.usedBy.mockReturnValue(ok({ strategies: [] }))
})

describe('buildHashLineage', () => {
  it('shares one dataset and one feature node between versions that name the same hash', () => {
    const g = buildHashLineage([full(1), full(2)] as never, { production: 2 })
    expect(g.nodes.filter((n) => n.kind === 'dataset')).toHaveLength(1)
    expect(g.nodes.filter((n) => n.kind === 'features')).toHaveLength(1)
    expect(g.nodes.filter((n) => n.kind === 'model_version')).toHaveLength(2)
    expect(g.nodes.filter((n) => n.kind === 'series')).toHaveLength(2)
  })

  it('draws a per-version gap for a link the registry did not record', () => {
    const g = buildHashLineage([version(1, { metrics: {} }), version(2, { metrics: {} })] as never, {})
    const gaps = g.nodes.filter((n) => !n.recorded && n.kind === 'dataset')
    expect(gaps).toHaveLength(2)
    expect(gaps.every((n) => n.hash === null && n.label === 'not recorded')).toBe(true)
  })

  it('chains the five links in order', () => {
    const g = buildHashLineage([full(1)] as never, undefined)
    expect(g.edges.map((e) => e.id.split('>').map((s) => s.split(':')[0]).join('>'))).toEqual([
      'dataset>features',
      'features>model_version',
      'model_version>series',
      'series>backtest',
    ])
  })
})

describe('the lineage graph', () => {
  it('shows skeletons while versions load', () => {
    mlops.listVersions.mockReturnValue(never())
    renderRoute(<LineageGraphView modelId="m1" />)
    expect(document.querySelector('.skel')).not.toBeNull()
  })

  it('shows an error when versions cannot be read', async () => {
    mlops.listVersions.mockRejectedValue(new Error('x'))
    renderRoute(<LineageGraphView modelId="m1" />)
    expect(await screen.findByRole('alert')).toHaveTextContent('Lineage could not be built')
  })

  it('says there is no lineage for a model with no versions', async () => {
    mlops.listVersions.mockReturnValue(ok({ versions: [] }))
    renderRoute(<LineageGraphView modelId="m1" />)
    expect(await screen.findByText('No lineage yet')).toBeInTheDocument()
  })

  it('lists the chain by hash and marks the production version', async () => {
    renderRoute(<LineageGraphView modelId="m1" />)
    const links = await screen.findAllByTestId('lineage-link')
    expect(links.length).toBeGreaterThan(0)
    expect(screen.getAllByTestId('flow-node').some((n) => n.textContent?.includes('production'))).toBe(true)
    expect(screen.queryByTestId('lineage-gaps')).toBeNull()
  })

  it('counts the gaps when the registry recorded no dataset', async () => {
    mlops.listVersions.mockReturnValue(ok({ versions: [version(1, { metrics: {} })] }))
    renderRoute(<LineageGraphView modelId="m1" />)
    expect(await screen.findByTestId('lineage-gaps')).toHaveTextContent('did not record')
  })
})

describe('the existing lineage page', () => {
  it('opens on the content-hash chain and can switch to the registry graph', async () => {
    renderRoute(<ModelLineagePage />, { path: '/mlops/graph', entry: '/mlops/graph?model=m1' })
    expect(await screen.findAllByTestId('lineage-link')).not.toHaveLength(0)
    fireEvent.click(screen.getByRole('radio', { name: 'Registry' }))
    expect(await screen.findByText(/Derived from the registry/)).toBeInTheDocument()
  })
})
