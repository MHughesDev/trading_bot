/**
 * The desk starts conversations on a provider *and* a model the user has a key
 * for, and draws a call to action, not a dead send button, when there is none
 * (PROVIDERS track, 2026-10-05).
 *
 * Before: the composer offered capability profiles only, and both of the desk's
 * doors opened the conversation on the server's default provider (Anthropic).
 * An OpenAI-only user could pick nothing that ran, and a user with no key at all
 * typed a question into a box whose Send failed after the click.
 */
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { MemoryRouter } from 'react-router-dom'
import type { ReactElement } from 'react'
import { describe, expect, it, vi } from 'vitest'

import type { AgentProfile } from '@/api/agent'
import type { ProviderStatus, ProvidersStatus } from '@/api/llm'
import { Composer } from '@/components/desk/Composer'
import { ConnectProviderCta } from '../ConnectProviderCta'
import { conversationEngine, engineChoices, needsProvider, parseEngineId } from '../engines'

function row(provider: string, over: Partial<ProviderStatus> = {}): ProviderStatus {
  return {
    provider: provider as ProviderStatus['provider'],
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

function status(...rows: ProviderStatus[]): ProvidersStatus {
  const all = ['anthropic', 'openai', 'vllm', 'ollama'].map((p) => rows.find((r) => r.provider === p) ?? row(p))
  return { providers: all, encryption_available: true, encryption_problem: null }
}

function profile(over: Partial<AgentProfile>): AgentProfile {
  return {
    model_id: 'claude-opus-5',
    provider: 'anthropic-agent-sdk',
    tier: 'frontier',
    is_local: false,
    tool_calling: 'multi_step',
    max_steps: 400,
    multi_step_evidence: null,
    requires_api_key: true,
    is_default: true,
    installed: null,
    resident: null,
    backend_reachable: null,
    ...over,
  }
}

const SHIPPED: AgentProfile[] = [
  profile({}),
  profile({ model_id: 'gpt-5', provider: 'openai', is_default: false }),
  profile({
    model_id: 'qwen2.5:7b-instruct',
    provider: 'ollama',
    tier: 'degraded',
    is_local: true,
    requires_api_key: false,
    is_default: false,
    backend_reachable: false,
    installed: null,
  }),
]

describe('what the composer offers', () => {
  it('an OpenAI-only user is offered OpenAI, on the model they chose, and nothing they cannot run', () => {
    const choices = engineChoices(
      status(row('openai', { configured: true, is_default: true, default_model: 'gpt-4o-mini' })),
      SHIPPED,
    )
    // The shipped gpt-5 profile is not offered: the platform does not enable it.
    expect(choices.map((c) => c.id)).toEqual(['openai|gpt-4o-mini'])
    expect(choices[0].isDefault).toBe(true)
    // No Anthropic: no key. No Ollama: its backend did not answer.
    expect(choices.some((c) => c.provider === 'anthropic' || c.provider === 'ollama')).toBe(false)
    // A model with no profile of its own runs under the frontier profile, and is unmeasured.
    expect(choices[0]).toMatchObject({ tier: 'frontier', frontier: true, measured: false })
  })

  it('the default provider comes first when both are connected', () => {
    const choices = engineChoices(
      status(
        row('anthropic', { configured: true }),
        row('openai', { configured: true, is_default: true, default_model: 'gpt-4o-mini' }),
      ),
      SHIPPED,
    )
    expect(choices[0].id).toBe('openai|gpt-4o-mini')
    expect(choices.map((c) => c.provider)).toContain('anthropic')
  })

  it('only the enabled baseline models are offered, whatever default was stored', () => {
    const choices = engineChoices(
      status(
        row('anthropic', { configured: true, default_model: 'claude-opus-5' }),
        row('openai', { configured: true, is_default: true, default_model: 'gpt-6.1-sol' }),
      ),
      SHIPPED,
    )
    expect(choices.map((c) => c.id)).toEqual(['openai|gpt-4o-mini', 'anthropic|claude-sonnet-5'])
  })

  it('a reachable local model is offered without a key', () => {
    const local = SHIPPED.map((p) => (p.is_local ? { ...p, backend_reachable: true, installed: true } : p))
    const choices = engineChoices(status(), local)
    expect(choices.map((c) => c.id)).toEqual(['ollama|qwen2.5:7b-instruct'])
    expect(needsProvider(status(), local)).toBe(false)
  })

  it('needs a provider only when both reads answered and nothing can run', () => {
    expect(needsProvider(status(), SHIPPED)).toBe(true)
    // A read that never came back is not evidence that nothing is connected.
    expect(needsProvider(undefined, SHIPPED)).toBe(false)
    expect(needsProvider(status(), undefined)).toBe(false)
  })

  it('an engine id round-trips model ids that hold colons and slashes', () => {
    expect(parseEngineId('ollama|qwen2.5:7b-instruct')).toEqual({ provider: 'ollama', model: 'qwen2.5:7b-instruct' })
    expect(parseEngineId('openai|ft:gpt-5:org/x')).toEqual({ provider: 'openai', model: 'ft:gpt-5:org/x' })
    expect(parseEngineId('claude-opus-5')).toBeNull()
    expect(parseEngineId('nobody|x')).toBeNull()
  })

  it('a conversation shows the engine it was opened on, and says when its key is gone', () => {
    const convo = { provider: 'openai', model: 'gpt-5' }
    const live = conversationEngine(convo, status(row('openai', { configured: true })), SHIPPED)
    expect(live?.choice.label).toBe('OpenAI · gpt-5')
    expect(live?.keyMissing).toBe(false)
    expect(conversationEngine(convo, status(), SHIPPED)?.keyMissing).toBe(true)
    // Unknown is not missing.
    expect(conversationEngine(convo, undefined, SHIPPED)?.keyMissing).toBe(false)
  })
})

function withRouter(ui: ReactElement) {
  const qc = new QueryClient()
  return render(
    <QueryClientProvider client={qc}>
      <MemoryRouter>{ui}</MemoryRouter>
    </QueryClientProvider>,
  )
}

describe('the call to action replaces the send button', () => {
  it('with no provider: a link to AI providers, and no question box or Send to press', () => {
    const onSubmit = vi.fn()
    withRouter(
      <Composer
        running={false}
        profiles={[]}
        model=""
        onModelChange={() => {}}
        onSubmit={onSubmit}
        connect={<ConnectProviderCta />}
      />,
    )
    expect(screen.getByTestId('connect-provider-cta')).toHaveTextContent(/Add an API key to start chatting/)
    expect(screen.getByRole('link', { name: /Add an API key/ })).toHaveAttribute('href', '/settings#ai')
    expect(screen.queryByTestId('send')).toBeNull()
    expect(screen.queryByLabelText('Ask, or describe what to research')).toBeNull()
    expect(onSubmit).not.toHaveBeenCalled()
  })

  it('with a provider: the active provider and model are on the picker, and Send sends', async () => {
    const onSubmit = vi.fn()
    const choices = engineChoices(status(row('openai', { configured: true, is_default: true, default_model: 'gpt-4o-mini' })), SHIPPED)
    withRouter(
      <Composer running={false} profiles={choices} model={choices[0].id} onModelChange={() => {}} onSubmit={onSubmit} />,
    )
    expect(screen.queryByTestId('connect-provider-cta')).toBeNull()
    expect(screen.getByRole('combobox', { name: 'Model' })).toHaveTextContent('OpenAI · gpt-4o-mini')
    fireEvent.change(screen.getByLabelText('Ask, or describe what to research'), { target: { value: 'hello' } })
    fireEvent.click(screen.getByTestId('send'))
    await waitFor(() => expect(onSubmit).toHaveBeenCalledTimes(1))
  })
})
