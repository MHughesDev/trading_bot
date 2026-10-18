/** N-5.2: loading, empty, error and populated states of the fleet, and the server's own flags. */
import { fireEvent, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { never, ok, renderRoute } from './testkit'

const studio = vi.hoisted(() => ({ fleet: vi.fn() }))
vi.mock('@/api/modelsStudio', () => ({ studioApi: studio }))
vi.mock('@/api/mlops', () => ({ modelsApi: {} }))

import { ModelCommandCenterPage } from '../../../pages/ModelCommandCenterPage'

const fleetModel = (id: string, over: Record<string, unknown> = {}) => ({
  model_id: id,
  display_name: `Model ${id}`,
  model_kind: 'forecaster',
  status: 'active',
  family: 'gbm',
  production_version: null,
  latest_version: 1,
  active_run: null,
  last_trained_at: '2026-10-01T00:00:00Z',
  stale: false,
  stale_reason: null,
  serving: false,
  ...over,
})

const fleet = (models: unknown[], counts = {}) => ({
  generated_at: '2026-10-04T00:00:00Z',
  counts: { models: models.length, training: 0, evaluating: 0, stale: 0, serving: 0, ...counts },
  models,
})

beforeEach(() => vi.clearAllMocks())

describe('the command center', () => {
  it('shows a skeleton while the fleet loads', () => {
    studio.fleet.mockReturnValue(never())
    renderRoute(<ModelCommandCenterPage />)
    expect(screen.queryByTestId('fleet-row')).toBeNull()
    expect(document.querySelector('.skel')).not.toBeNull()
  })

  it('says there are no models, and offers to create one, when the fleet is empty', async () => {
    studio.fleet.mockReturnValue(ok(fleet([])))
    renderRoute(<ModelCommandCenterPage />)
    expect(await screen.findByText('No models yet')).toBeInTheDocument()
  })

  it('shows an error with retry, not an empty fleet, when the read fails', async () => {
    studio.fleet.mockRejectedValue(new Error('boom'))
    renderRoute(<ModelCommandCenterPage />)
    expect(await screen.findByRole('alert')).toHaveTextContent('The fleet could not be read')
    expect(screen.queryByText('No models yet')).toBeNull()
    studio.fleet.mockReturnValue(ok(fleet([fleetModel('a')])))
    fireEvent.click(screen.getByRole('button', { name: /retry/i }))
    expect(await screen.findByTestId('fleet-row')).toBeInTheDocument()
  })

  it('renders training, stale and serving exactly as the server flagged them', async () => {
    studio.fleet.mockReturnValue(
      ok(
        fleet(
          [
            fleetModel('a', { active_run: { run_id: 'r1', kind: 'training', status: 'running', progress: 42, phase: 'fitting' } }),
            fleetModel('b', { stale: true, stale_reason: 'data newer than model' }),
            fleetModel('c', { serving: true, production_version: 3 }),
          ],
          { training: 1, stale: 1, serving: 1 },
        ),
      ),
    )
    renderRoute(<ModelCommandCenterPage />)
    expect(await screen.findAllByTestId('fleet-row')).toHaveLength(3)
    expect(screen.getByTestId('fleet-run')).toHaveTextContent('fitting')
    expect(screen.getByTestId('fleet-run')).toHaveTextContent('42%')
    expect(screen.getByTestId('fleet-stale')).toHaveTextContent('data newer than model')
    expect(screen.getByText('serving v3')).toBeInTheDocument()
    expect(screen.getByTestId('fleet-counts')).toHaveTextContent('Stale')
  })

  it('filters to the stale models and says so when none match', async () => {
    studio.fleet.mockReturnValue(ok(fleet([fleetModel('a')])))
    renderRoute(<ModelCommandCenterPage />)
    await screen.findByTestId('fleet-row')
    fireEvent.click(screen.getByRole('radio', { name: 'Stale' }))
    await waitFor(() => expect(screen.queryByTestId('fleet-row')).toBeNull())
    expect(screen.getByText('No stale models')).toBeInTheDocument()
  })
})
