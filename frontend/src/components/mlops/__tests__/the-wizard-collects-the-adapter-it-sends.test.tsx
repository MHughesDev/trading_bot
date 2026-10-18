/**
 * The wizard's external-LLM path used to send `adapter: { provider: '', model:
 * '', endpoint: '' }` -- a block it never asked for -- and, on the 422 the
 * registry answers with (`errors: [{ path, message }]`), showed only "Failed to
 * create model". It now collects the three fields, refuses to advance without
 * them, sends what was typed, and prints the server's paths.
 */
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { MemoryRouter } from 'react-router-dom'
import { beforeEach, describe, expect, it, vi } from 'vitest'

const api = vi.hoisted(() => ({ create: vi.fn() }))
vi.mock('@/api/mlops', () => ({ modelsApi: api }))

import { ModelCreatePage } from '../../../pages/ModelCreatePage'

function renderWizard() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } })
  return render(
    <QueryClientProvider client={qc}>
      <MemoryRouter>
        <ModelCreatePage />
      </MemoryRouter>
    </QueryClientProvider>,
  )
}

const next = () => fireEvent.click(screen.getByRole('button', { name: /^Next/ }))

async function toAdapterStep() {
  fireEvent.click(screen.getByText('LLM Adapter'))
  next()
  await screen.findByTestId('adapter-fields')
}

beforeEach(() => {
  vi.clearAllMocks()
})

describe('the wizard collects the adapter it sends', () => {
  it('will not advance until provider, model and a valid endpoint are given', async () => {
    renderWizard()
    await toAdapterStep()
    expect(screen.getByRole('button', { name: /^Next/ })).toBeDisabled()

    const [provider, model, endpoint] = screen.getAllByRole('textbox')
    fireEvent.change(provider, { target: { value: 'ollama' } })
    fireEvent.change(model, { target: { value: 'gemma2:9b' } })
    fireEvent.change(endpoint, { target: { value: 'localhost:11434' } })
    expect(await screen.findByText('Endpoint must start with http:// or https://.')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: /^Next/ })).toBeDisabled()

    fireEvent.change(endpoint, { target: { value: 'http://localhost:11434' } })
    await waitFor(() => expect(screen.getByRole('button', { name: /^Next/ })).toBeEnabled())
  })

  it('sends what was typed and shows the 422 paths the server returned', async () => {
    api.create.mockRejectedValue({
      response: {
        status: 422,
        data: { error: 'invalid_definition', errors: [{ path: 'adapter.endpoint', message: 'host not allowed' }] },
      },
    })
    renderWizard()
    await toAdapterStep()
    const [provider, model, endpoint] = screen.getAllByRole('textbox')
    fireEvent.change(provider, { target: { value: 'ollama' } })
    fireEvent.change(model, { target: { value: 'gemma2:9b' } })
    fireEvent.change(endpoint, { target: { value: 'http://localhost:11434' } })
    next() // -> data
    next() // -> identity
    fireEvent.change(await screen.findByPlaceholderText('e.g. BTC Price Forecaster'), { target: { value: 'Local Gemma' } })
    fireEvent.click(screen.getByRole('button', { name: /Create/ }))

    const list = await screen.findByTestId('server-errors')
    expect(list).toHaveTextContent('adapter.endpoint')
    expect(list).toHaveTextContent('host not allowed')
    const sent = api.create.mock.calls[0][0]
    expect(sent.definition.adapter).toEqual({
      provider: 'ollama',
      model: 'gemma2:9b',
      endpoint: 'http://localhost:11434',
      cost_per_1k_tokens: '0.0',
    })
  })

  it('falls back to the general message for a failure that is not a validation error', async () => {
    api.create.mockRejectedValue(new Error('network'))
    renderWizard()
    await toAdapterStep()
    const [provider, model, endpoint] = screen.getAllByRole('textbox')
    fireEvent.change(provider, { target: { value: 'ollama' } })
    fireEvent.change(model, { target: { value: 'm' } })
    fireEvent.change(endpoint, { target: { value: 'http://h' } })
    next()
    next()
    fireEvent.change(await screen.findByPlaceholderText('e.g. BTC Price Forecaster'), { target: { value: 'X' } })
    fireEvent.click(screen.getByRole('button', { name: /Create/ }))
    expect(await screen.findByText('Failed to create model. Please try again.')).toBeInTheDocument()
  })
})
