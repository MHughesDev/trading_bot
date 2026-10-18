/**
 * N-5.6: run a version against saved cases and a latest-bars window; each answer
 * carries its point-in-time status, and an answer that does not report one reads
 * "PIT not reported", never "clean".
 */
import { fireEvent, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { never, ok, renderRoute, version } from './testkit'

const mlops = vi.hoisted(() => ({
  listVersions: vi.fn(),
  getAliases: vi.fn(),
  listTestCases: vi.fn(),
  featureVector: vi.fn(),
  testVersion: vi.fn(),
}))
vi.mock('@/api/modelsStudio', () => ({ studioApi: {} }))
vi.mock('@/api/mlops', () => ({ modelsApi: mlops }))

import { TestLab } from '../TestLab'

const answer = (over: Record<string, unknown> = {}) => ({
  model_id: 'm1',
  version: 2,
  kind: 'forecaster',
  predictions: [{ direction: 'up', magnitude: '0.4%', confidence: 0.61, horizon: '1h' }],
  latency_ms: 12,
  ...over,
})

const show = () => renderRoute(<TestLab modelId="m1" />)

beforeEach(() => {
  vi.clearAllMocks()
  mlops.listVersions.mockReturnValue(ok({ versions: [version(1), version(2)] }))
  mlops.getAliases.mockReturnValue(ok({ production: 1 }))
  mlops.listTestCases.mockReturnValue(
    ok({ test_cases: [{ case_id: 'c1', name: 'Spike day', input: { instrument_id: 'BTC-USD', features: { rsi: 71 } }, created_at: '2026-10-01T00:00:00Z' }] }),
  )
  mlops.featureVector.mockReturnValue(ok({ feature_set: 'x', feature_order: ['rsi'], features: { rsi: 50 }, source: 'computed', bars_used: 120 }))
  mlops.testVersion.mockReturnValue(ok(answer()))
})

describe('the test lab', () => {
  it('shows skeletons while cases load', () => {
    mlops.listTestCases.mockReturnValue(never())
    show()
    expect(document.querySelector('.skel')).not.toBeNull()
  })

  it('says there are no saved cases', async () => {
    mlops.listTestCases.mockReturnValue(ok({ test_cases: [] }))
    show()
    expect(await screen.findByText('No saved cases')).toBeInTheDocument()
  })

  it('shows an error when saved cases cannot be read', async () => {
    mlops.listTestCases.mockRejectedValue(new Error('x'))
    show()
    expect(await screen.findByRole('alert')).toHaveTextContent('Saved cases could not be read')
  })

  it('does not run until something is chosen', async () => {
    show()
    await screen.findByText('Spike day')
    expect(screen.getByRole('button', { name: /run version/i })).toBeDisabled()
  })

  it('runs the chosen case against the selected version and shows the answer', async () => {
    show()
    fireEvent.click(await screen.findByLabelText('Spike day'))
    fireEvent.click(screen.getByRole('button', { name: /run version/i }))
    expect(await screen.findByTestId('answer-row')).toHaveTextContent('up 0.4%')
    expect(mlops.testVersion).toHaveBeenCalledWith('m1', 1, [{ instrument_id: 'BTC-USD', features: { rsi: 71 } }])
  })

  it('builds the window from the latest bars and says where the features came from', async () => {
    show()
    await screen.findByText('Spike day')
    fireEvent.change(screen.getByLabelText('Instrument'), { target: { value: 'ETH-USD' } })
    fireEvent.click(screen.getByRole('button', { name: /run version/i }))
    const row = await screen.findByTestId('answer-row')
    expect(row).toHaveTextContent('features: computed, 120 bars')
    expect(mlops.featureVector).toHaveBeenCalledWith('m1', { instrument_id: 'ETH-USD', timeframe: '1h' })
    expect(mlops.testVersion.mock.calls[0][2][0]).toEqual({ instrument_id: 'ETH-USD', features: { rsi: 50 } })
  })

  it('shows PIT not reported when the answer says nothing, and a violation when it reports one', async () => {
    show()
    fireEvent.click(await screen.findByLabelText('Spike day'))
    fireEvent.click(screen.getByRole('button', { name: /run version/i }))
    expect(await screen.findByTestId('pit-status')).toHaveTextContent('PIT not reported')
    expect(screen.getByTestId('pit-status')).not.toHaveTextContent('clean')

    mlops.testVersion.mockReturnValue(ok(answer({ pit_status: 'violation' })))
    fireEvent.click(screen.getByRole('button', { name: /run version/i }))
    await waitFor(() => expect(screen.getByTestId('pit-status')).toHaveTextContent('PIT violation'))
  })

  it('shows PIT clean only when the answer says so', async () => {
    mlops.testVersion.mockReturnValue(ok(answer({ pit_status: 'ok' })))
    show()
    fireEvent.click(await screen.findByLabelText('Spike day'))
    fireEvent.click(screen.getByRole('button', { name: /run version/i }))
    expect(await screen.findByTestId('pit-status')).toHaveTextContent('PIT clean')
  })

  it('reports a version that did not answer instead of dropping the row', async () => {
    mlops.testVersion.mockRejectedValue(new Error('503'))
    show()
    fireEvent.click(await screen.findByLabelText('Spike day'))
    fireEvent.click(screen.getByRole('button', { name: /run version/i }))
    expect(await screen.findByTestId('answer-row')).toHaveTextContent('did not answer')
  })
})
