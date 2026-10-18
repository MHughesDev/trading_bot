/**
 * The LLM-adapter test surface sent `features: {}` and no prompt, so the server
 * fell back to its default "Hello" and whatever the person typed never reached
 * the model. It now sends `{ prompt }`, and shows the tokens, cost and trace id
 * the server returns instead of dropping them.
 */
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { beforeEach, describe, expect, it, vi } from 'vitest'

const api = vi.hoisted(() => ({
  get: vi.fn(),
  listVersions: vi.fn(),
  marketInstruments: vi.fn(),
  listTestCases: vi.fn(),
  testVersion: vi.fn(),
  addTestCase: vi.fn(),
  deleteTestCase: vi.fn(),
  featureVector: vi.fn(),
}))
vi.mock('@/api/mlops', () => ({ modelsApi: api }))

import { ModelTestTab } from '../ModelTestTab'

beforeEach(() => {
  vi.clearAllMocks()
  api.get.mockResolvedValue({
    data: {
      model_id: 'm1',
      slug: 'llm',
      display_name: 'LLM',
      model_kind: 'external_llm_adapter',
      asset_class: 'crypto',
      definition: { model_kind: 'external_llm_adapter' },
      status: 'active',
      created_by: 'u',
      created_at: '2026-10-01T00:00:00Z',
      updated_at: '2026-10-01T00:00:00Z',
    },
  })
  api.listVersions.mockResolvedValue({
    data: { versions: [{ version: 1, model_id: 'm1', status: 'active', created_at: '2026-10-01T00:00:00Z' }] },
  })
  api.marketInstruments.mockResolvedValue({ data: { instruments: [] } })
  api.featureVector.mockResolvedValue({ data: { feature_set: 'x', feature_order: [], features: {}, source: 'schema' } })
  api.listTestCases.mockResolvedValue({ data: { test_cases: [] } })
  api.testVersion.mockResolvedValue({
    data: { model_id: 'm1', version: 1, kind: 'llm', text: 'BTC looks calm.', latency_ms: 840, tokens: 57, cost_usd: '0.0021', trace_id: 'tr-9' },
  })
})

describe('the prompt typed in the Test Lab reaches the model', () => {
  it('sends the prompt as the instance and shows tokens, cost and trace id', async () => {
    const qc = new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: 0 } } })
    render(
      <QueryClientProvider client={qc}>
        <ModelTestTab modelId="m1" />
      </QueryClientProvider>,
    )
    const box = await screen.findByDisplayValue(/Summarize current BTC/)
    fireEvent.change(box, { target: { value: 'What is BTC doing?' } })
    await waitFor(() => expect(screen.getByRole('button', { name: /Run/ })).toBeEnabled())
    fireEvent.click(screen.getByRole('button', { name: /Run/ }))
    await screen.findByText('BTC looks calm.')
    expect(api.testVersion).toHaveBeenCalledWith('m1', 1, [{ prompt: 'What is BTC doing?' }])
    const metrics = screen.getByTestId('llm-metrics')
    expect(metrics).toHaveTextContent('840 ms')
    expect(metrics).toHaveTextContent('57 tokens')
    expect(metrics).toHaveTextContent('$0.0021')
    expect(metrics).toHaveTextContent('trace tr-9')
  })
})
