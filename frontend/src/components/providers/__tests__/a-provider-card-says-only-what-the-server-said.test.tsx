/**
 * The AI provider card (PROVIDERS track, 2026-10-05).
 *
 * Four states, each a statement the server made — Not connected, Connected with
 * a masked suffix and a verified time, Key rejected with the provider's reason,
 * and "state unknown" when nothing answered — and the four actions a person
 * takes on a key: Save & test in one step, Test again, Replace, Remove (behind a
 * confirmation). The panel around the cards carries the one honest line about
 * subscriptions, and the operator's instruction when keys cannot be stored.
 */
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import type { ReactElement } from 'react'
import { beforeEach, describe, expect, it, vi } from 'vitest'

const llm = vi.hoisted(() => ({
  status: vi.fn(),
  save: vi.fn(),
  verify: vi.fn(),
  remove: vi.fn(),
  setDefault: vi.fn(),
  models: vi.fn(),
}))
vi.mock('@/api/llm', async (orig) => ({ ...(await orig<object>()), llmKeysApi: llm }))

import { PROVIDERS, type ProviderStatus } from '@/api/llm'
import { AiProvidersPanel } from '../AiProvidersPanel'
import { ProviderCard } from '../ProviderCard'
import { cardState } from '../providers.helpers'

const anthropic = PROVIDERS.find((p) => p.id === 'anthropic')!
const openai = PROVIDERS.find((p) => p.id === 'openai')!

function row(over: Partial<ProviderStatus> = {}): ProviderStatus {
  return {
    provider: 'openai',
    configured: false,
    key_last4: null,
    base_url: null,
    updated_at: null,
    verified_at: null,
    last_checked_at: null,
    last_error: null,
    default_model: null,
    is_default: false,
    ...over,
  }
}

function withClient(ui: ReactElement) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } })
  return render(<QueryClientProvider client={qc}>{ui}</QueryClientProvider>)
}

beforeEach(() => {
  for (const f of Object.values(llm)) f.mockReset()
  llm.models.mockResolvedValue([{ id: 'gpt-5', display_name: null }, { id: 'gpt-5-mini', display_name: null }])
})

describe('the headline is the server’s statement', () => {
  it('maps each status to exactly one state', () => {
    expect(cardState(null)).toBe('unknown')
    expect(cardState(row())).toBe('disconnected')
    expect(cardState(row({ configured: true }))).toBe('connected')
    expect(cardState(row({ configured: true, last_error: 'revoked' }))).toBe('rejected')
  })

  it('Not connected: a key field and Save & test, nothing else', () => {
    withClient(<ProviderCard meta={openai} status={row()} canSave />)
    expect(screen.getByText('Not connected')).toBeInTheDocument()
    expect(screen.getByLabelText('OpenAI API key')).toHaveAttribute('type', 'password')
    expect(screen.getByRole('button', { name: 'Save & test' })).toBeDisabled()
    expect(screen.queryByRole('button', { name: 'Remove' })).toBeNull()
    // Where to get one.
    expect(screen.getByRole('link', { name: /platform\.openai\.com\/api-keys/ })).toHaveAttribute(
      'href',
      'https://platform.openai.com/api-keys',
    )
  })

  it('Connected: the masked suffix and the verified time, never the key', () => {
    withClient(
      <ProviderCard
        meta={openai}
        status={row({ configured: true, key_last4: 'abcd', verified_at: new Date(Date.now() - 4 * 60_000).toISOString(), is_default: true, default_model: 'gpt-5' })}
        canSave
      />,
    )
    expect(screen.getByText('Connected')).toBeInTheDocument()
    expect(screen.getByText('default')).toBeInTheDocument()
    expect(screen.getByTestId('provider-line-openai')).toHaveTextContent('key ending ••••abcd · verified 4 min ago')
    expect(screen.queryByLabelText('OpenAI API key')).toBeNull()
    for (const name of ['Test again', 'Replace', 'Remove']) {
      expect(screen.getByRole('button', { name })).toBeInTheDocument()
    }
    // Already the default: no button to make it so.
    expect(screen.queryByRole('button', { name: 'Make default' })).toBeNull()
  })

  it('Key rejected: the provider’s reason, as the server recorded it', () => {
    withClient(
      <ProviderCard
        meta={anthropic}
        status={row({ provider: 'anthropic', configured: true, key_last4: 'wxyz', last_error: 'the provider rejected this key (invalid x-api-key)' })}
        canSave
      />,
    )
    expect(screen.getByText('Key rejected')).toBeInTheDocument()
    expect(screen.getByTestId('provider-error-anthropic')).toHaveTextContent(
      'Key rejected: the provider rejected this key (invalid x-api-key)',
    )
    expect(screen.getByRole('button', { name: 'Replace' })).toBeInTheDocument()
  })

  it('state unknown: no claim, and no form inviting a key the platform may already hold', () => {
    withClient(<ProviderCard meta={anthropic} status={null} canSave />)
    expect(screen.getByText('state unknown')).toBeInTheDocument()
    expect(screen.queryByText('Not connected')).toBeNull()
    expect(screen.queryByRole('button', { name: 'Save & test' })).toBeNull()
  })
})

describe('the actions reach the routes', () => {
  it('Save & test sends the pasted key once and clears the field on success', async () => {
    llm.save.mockResolvedValue({ ok: true, verified: true, model_count: 2, key_last4: 'cdef' })
    withClient(<ProviderCard meta={openai} status={row()} canSave />)
    const field = screen.getByLabelText('OpenAI API key')
    fireEvent.change(field, { target: { value: '  sk-test-not-a-real-key-cdef  ' } })
    fireEvent.click(screen.getByRole('button', { name: 'Save & test' }))
    await waitFor(() => expect(llm.save).toHaveBeenCalledWith('openai', { api_key: 'sk-test-not-a-real-key-cdef' }))
    await waitFor(() => expect(field).toHaveValue(''))
  })

  it('a rejected paste says why, and keeps the key in the field to fix', async () => {
    llm.save.mockRejectedValue({
      response: { status: 422, data: { error: 'key_rejected', message: 'the provider rejected this key (Incorrect API key provided: [redacted].)' } },
    })
    withClient(<ProviderCard meta={openai} status={row()} canSave />)
    const field = screen.getByLabelText('OpenAI API key')
    fireEvent.change(field, { target: { value: 'sk-test-typo' } })
    fireEvent.click(screen.getByRole('button', { name: 'Save & test' }))
    expect(await screen.findByText(/Key rejected: the provider rejected this key/)).toBeInTheDocument()
    expect(field).toHaveValue('sk-test-typo')
  })

  it('Test again re-checks the stored key', async () => {
    llm.verify.mockResolvedValue({ ok: true, verified: true, model_count: 2 })
    withClient(<ProviderCard meta={openai} status={row({ configured: true, key_last4: 'abcd', verified_at: new Date().toISOString() })} canSave />)
    fireEvent.click(screen.getByRole('button', { name: 'Test again' }))
    await waitFor(() => expect(llm.verify).toHaveBeenCalledWith('openai'))
  })

  it('Replace opens the key field without removing the stored key', () => {
    withClient(<ProviderCard meta={openai} status={row({ configured: true, key_last4: 'abcd' })} canSave />)
    fireEvent.click(screen.getByRole('button', { name: 'Replace' }))
    expect(screen.getByLabelText('OpenAI API key')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Cancel' })).toBeInTheDocument()
    expect(llm.remove).not.toHaveBeenCalled()
  })

  it('Remove asks first, and only the confirmation deletes', async () => {
    llm.remove.mockResolvedValue({ ok: true })
    withClient(<ProviderCard meta={openai} status={row({ configured: true, key_last4: 'abcd' })} canSave />)
    fireEvent.click(screen.getByRole('button', { name: 'Remove' }))
    expect(llm.remove).not.toHaveBeenCalled()
    const dialog = await screen.findByRole('dialog')
    expect(dialog).toHaveTextContent('••••abcd')
    fireEvent.click(within(dialog).getByRole('button', { name: 'Remove key' }))
    await waitFor(() => expect(llm.remove).toHaveBeenCalledWith('openai'))
  })

  it('Make default names the provider', async () => {
    llm.setDefault.mockResolvedValue({ ok: true })
    withClient(<ProviderCard meta={anthropic} status={row({ provider: 'anthropic', configured: true, key_last4: 'abcd' })} canSave />)
    fireEvent.click(screen.getByRole('button', { name: 'Make default' }))
    await waitFor(() => expect(llm.setDefault).toHaveBeenCalledWith('anthropic', undefined))
  })
})

describe('the panel', () => {
  it('says plainly that subscriptions cannot be connected, with both key pages', async () => {
    llm.status.mockResolvedValue({ providers: [row({ provider: 'anthropic' }), row()], encryption_available: true, encryption_problem: null })
    withClient(<AiProvidersPanel />)
    const note = screen.getByTestId('subscriptions-note')
    expect(note).toHaveTextContent(
      "Anthropic and OpenAI only allow API keys in third-party apps; subscriptions (Claude Pro/Max, ChatGPT Plus) can't be connected here.",
    )
    expect(within(note).getByRole('link', { name: 'console.anthropic.com/settings/keys' })).toHaveAttribute(
      'href',
      'https://console.anthropic.com/settings/keys',
    )
    expect(within(note).getByRole('link', { name: 'platform.openai.com/api-keys' })).toHaveAttribute(
      'href',
      'https://platform.openai.com/api-keys',
    )
    await waitFor(() => expect(screen.getAllByText('Not connected').length).toBeGreaterThanOrEqual(2))
    // No membership or subscription sign-in anywhere on it.
    expect(screen.queryByRole('button', { name: /sign in|log in|subscription|membership/i })).toBeNull()
  })

  it('without CRED_KEK, prints the operator’s instruction and offers no form', async () => {
    const problem =
      "Keys can't be saved: credential encryption isn't configured on this platform. The operator must set CRED_KEK to 64 hex characters (32 random bytes, e.g. `openssl rand -hex 32`) in the platform's environment and restart it."
    llm.status.mockResolvedValue({ providers: [row({ provider: 'anthropic' }), row()], encryption_available: false, encryption_problem: problem })
    withClient(<AiProvidersPanel />)
    expect(await screen.findByTestId('encryption-problem')).toHaveTextContent(problem)
    expect(screen.queryByRole('button', { name: 'Save & test' })).toBeNull()
  })
})
