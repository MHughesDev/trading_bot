/**
 * S-6.20 — "a profile with no calibration row renders `unmeasured` rather than
 * a default; providers and keys work here and the old form is gone."
 *
 * Amended by the PROVIDERS track (2026-10-05): every user now connects their own
 * key, and the one place a key is set is Settings -> AI providers. The Models
 * page keeps what it owes the clause — which providers are connected, beside the
 * tiers they configure — and links there instead of holding a second form. The
 * form itself is held by `components/providers/__tests__/`.
 *
 * The first clause is held by `hosted-is-not-measured.test.tsx`. This file holds
 * the other two: the Models page reports each provider and reaches the one form,
 * and nothing in the tree still imports the retired `LlmCredentialForm`.
 */
import { render, screen, waitFor } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { MemoryRouter } from 'react-router-dom'
import { readdirSync, readFileSync, statSync } from 'node:fs'
import { join } from 'node:path'
import { describe, expect, it, vi } from 'vitest'

const llm = vi.hoisted(() => ({
  status: vi.fn(),
  models: vi.fn(() => Promise.resolve([])),
}))
vi.mock('@/api/llm', async (orig) => ({ ...(await orig<object>()), llmKeysApi: llm }))
vi.mock('@/api/agent', async (orig) => ({
  ...(await orig<object>()),
  conversationsApi: {
    listProfiles: vi.fn(() =>
      Promise.resolve({ data: { profiles: [], unprofiled_local_models: [], default_provider: 'anthropic', default_model: 'claude-opus-5' } }),
    ),
  },
}))

import { Models } from '../Models'

function withClient(ui: React.ReactElement) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(
    <QueryClientProvider client={qc}>
      <MemoryRouter>{ui}</MemoryRouter>
    </QueryClientProvider>,
  )
}

const row = (provider: string, over: Record<string, unknown> = {}) => ({
  provider,
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
})

describe('providers on the Models page', () => {
  it('says which keys are connected and links to the one place they are set', async () => {
    llm.status.mockResolvedValue({
      providers: [row('anthropic', { configured: true, key_last4: 'abcd', is_default: true }), row('openai')],
      encryption_available: true,
      encryption_problem: null,
    })
    withClient(<Models />)
    await waitFor(() => expect(screen.getByText(/Anthropic: connected ····abcd/)).toBeInTheDocument())
    expect(screen.getByText(/OpenAI: not connected/)).toBeInTheDocument()
    expect(screen.getByRole('link', { name: /Manage AI providers/ })).toHaveAttribute('href', '/settings#ai')
    // No second form: nothing here takes a key.
    expect(screen.queryByLabelText(/API key/)).toBeNull()
  })

  it('does not claim "not connected" when the list never answered', async () => {
    llm.status.mockRejectedValue(new Error('500'))
    withClient(<Models />)
    await waitFor(() => expect(screen.getAllByText(/state unknown/).length).toBe(2))
    expect(screen.queryByText(/not connected/)).toBeNull()
  })
})

describe('the old form is gone', () => {
  it('no source file imports LlmCredentialForm, and no file defines it', () => {
    const root = join(__dirname, '..', '..', '..')
    const offenders: string[] = []
    const walk = (dir: string) => {
      for (const name of readdirSync(dir)) {
        const p = join(dir, name)
        if (statSync(p).isDirectory()) {
          if (name !== '__tests__' && name !== 'node_modules') walk(p)
        } else if (/\.(tsx?|jsx?)$/.test(name)) {
          const src = readFileSync(p, 'utf8')
          if (/import[^;]*LlmCredentialForm|function LlmCredentialForm|const LlmCredentialForm/.test(src)) offenders.push(p)
        }
      }
    }
    walk(root)
    expect(offenders).toEqual([])
  })
})
