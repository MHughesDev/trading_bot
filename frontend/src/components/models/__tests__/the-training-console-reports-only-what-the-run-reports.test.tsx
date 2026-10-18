/**
 * N-5.5: live progress from the served routes. The checkpoint cadence and the
 * max_gpu_hours budget are shown only when the run reports them; otherwise the
 * console says "not reported" and never invents a number.
 */
import { fireEvent, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { never, ok, renderRoute } from './testkit'

const mlops = vi.hoisted(() => ({
  listRuns: vi.fn(),
  listVersions: vi.fn(),
  events: vi.fn(),
  trainingCurve: vi.fn(),
  cancelRun: vi.fn(),
}))
vi.mock('@/api/modelsStudio', () => ({ studioApi: {} }))
vi.mock('@/api/mlops', () => ({ modelsApi: mlops }))

import { TrainingConsole } from '../TrainingConsole'

const run = (over: Record<string, unknown> = {}) => ({
  run_id: 'abcdef12-0000',
  model_id: 'm1',
  kind: 'training',
  status: 'running',
  progress: 35,
  phase: 'fitting fold 2',
  created_at: '2026-10-04T10:00:00Z',
  ...over,
})

const show = () => renderRoute(<TrainingConsole modelId="m1" />)

beforeEach(() => {
  vi.clearAllMocks()
  mlops.listRuns.mockReturnValue(ok({ runs: [run()] }))
  mlops.listVersions.mockReturnValue(ok({ versions: [] }))
  mlops.events.mockReturnValue(ok({ model_id: 'm1', events: [] }))
  mlops.trainingCurve.mockReturnValue(ok({ run_id: 'r', points: [{ epoch: 1, train_loss: 0.9, val_loss: 1.0 }, { epoch: 2, train_loss: 0.7, val_loss: 0.9 }] }))
  mlops.cancelRun.mockReturnValue(ok({}))
})

describe('the training console', () => {
  it('shows a skeleton while runs load', () => {
    mlops.listRuns.mockReturnValue(never())
    show()
    expect(document.querySelector('.skel')).not.toBeNull()
  })

  it('says there is no run yet', async () => {
    mlops.listRuns.mockReturnValue(ok({ runs: [] }))
    show()
    expect(await screen.findByText('No training run yet')).toBeInTheDocument()
  })

  it('shows an error when runs cannot be read', async () => {
    mlops.listRuns.mockRejectedValue(new Error('x'))
    show()
    expect(await screen.findByRole('alert')).toHaveTextContent('Training runs could not be read')
  })

  it('shows a running run: status, phase, progress and the recorded loss curve', async () => {
    show()
    expect(await screen.findByTestId('run-status')).toHaveTextContent('running')
    expect(screen.getByTestId('run-progress')).toHaveTextContent('35%')
    expect(screen.getByText('fitting fold 2')).toBeInTheDocument()
    expect(await screen.findByTestId('loss-curve')).toBeInTheDocument()
  })

  it('says "not reported" for the checkpoint cadence and the GPU budget when the run does not report them', async () => {
    show()
    const b = await screen.findByTestId('training-budget')
    expect(b).toHaveTextContent('Checkpoint cadencenot reported')
    expect(b).toHaveTextContent('max_gpu_hours)not reported')
    expect(b).not.toHaveTextContent(/\d+ h/)
  })

  it('shows the cadence and the budget when the run reports them', async () => {
    mlops.listRuns.mockReturnValue(ok({ runs: [run({ metrics: { checkpoint_every_epochs: 5, max_gpu_hours: 2, gpu_hours_used: 0.5 } })] }))
    show()
    const b = await screen.findByTestId('training-budget')
    expect(b).toHaveTextContent('5 epochs')
    expect(b).toHaveTextContent('2 h')
    expect(b).toHaveTextContent('0.5 h')
  })

  it('shows why a failed run failed, or that it said nothing', async () => {
    mlops.listRuns.mockReturnValue(ok({ runs: [run({ status: 'failed', error: 'CUDA out of memory' })] }))
    show()
    expect(await screen.findByTestId('run-error')).toHaveTextContent('CUDA out of memory')
  })

  it('cancels the active run through the registry', async () => {
    show()
    fireEvent.click(await screen.findByRole('button', { name: 'Cancel run' }))
    await waitFor(() => expect(mlops.cancelRun).toHaveBeenCalledWith('m1', 'abcdef12-0000'))
  })
})
