// Connect-your-own-key client (PROVIDERS track, 2026-10-05).
//
// Every user brings their own Anthropic and/or OpenAI **API key**. The key goes
// to the server once, in a PUT body, is verified there with a real provider call,
// and is never sent back: every read below returns a masked suffix at most.
// Nothing in this file stores a key, and no route here puts one in a URL.
//
// Every path is one `crates/api/src/routes/mod.rs` mounts
// (`every_client_path_is_served` holds that):
//   GET    /api/llm/credentials
//   PUT    /api/llm/credentials/{provider}
//   POST   /api/llm/credentials/{provider}/verify
//   DELETE /api/llm/credentials/{provider}
//   PUT    /api/llm/default
//   POST   /api/llm/{provider}/models
import { api as client } from '@/lib/api'

export type ProviderId = 'anthropic' | 'openai' | 'vllm' | 'ollama'

/** The two providers a user brings a key for. */
export type HostedProviderId = 'anthropic' | 'openai'

export interface ProviderMeta {
  id: ProviderId
  label: string
  hosted: boolean
  /** Where the user creates a key. Hosted providers only. */
  keysUrl?: string
  /** The shape of a key, as a placeholder. Never a real key. */
  keyPlaceholder?: string
  /** The model the desk suggests when the user has not chosen one. */
  suggestedModel?: string
  /**
   * The only models the platform will call on this provider, while it is in
   * testing. Mirrors `crates/api/src/model_policy.rs`, which is the authority:
   * the server refuses any other model whatever this list says. Unset = any.
   */
  allowedModels?: string[]
}

export const PROVIDERS: ProviderMeta[] = [
  {
    id: 'anthropic',
    label: 'Anthropic',
    hosted: true,
    keysUrl: 'https://console.anthropic.com/settings/keys',
    keyPlaceholder: 'sk-ant-…',
    suggestedModel: 'claude-sonnet-5',
    allowedModels: ['claude-sonnet-5'],
  },
  {
    id: 'openai',
    label: 'OpenAI',
    hosted: true,
    keysUrl: 'https://platform.openai.com/api-keys',
    keyPlaceholder: 'sk-…',
    suggestedModel: 'gpt-4o-mini',
    allowedModels: ['gpt-4o-mini'],
  },
  { id: 'vllm', label: 'Local (vLLM)', hosted: false },
  { id: 'ollama', label: 'Local (Ollama, dev only)', hosted: false },
]

export function providerMeta(id: string): ProviderMeta | undefined {
  return PROVIDERS.find((p) => p.id === id)
}

/** Whether the platform will call `model` on `provider` (see `allowedModels`). */
export function modelEnabled(provider: string, model: string): boolean {
  const allowed = providerMeta(provider)?.allowedModels
  return !allowed || allowed.includes(model)
}

/** One provider's state for the signed-in user. No field can hold a key. */
export interface ProviderStatus {
  provider: ProviderId
  configured: boolean
  key_last4: string | null
  base_url: string | null
  updated_at: string | null
  /** When the stored key last answered a real call. */
  verified_at: string | null
  last_checked_at: string | null
  /** Why the last check failed, with the key scrubbed out. */
  last_error: string | null
  default_model: string | null
  is_default: boolean
}

export interface ProvidersStatus {
  providers: ProviderStatus[]
  encryption_available: boolean
  /** The operator's instruction, verbatim, when keys cannot be saved. */
  encryption_problem: string | null
}

export interface ProviderModel {
  id: string
  display_name: string | null
}

export const PROVIDERS_QUERY_KEY = ['llm-credentials'] as const

export const llmKeysApi = {
  status: () => client.get<ProvidersStatus>('/api/llm/credentials').then((r) => r.data),
  /** Verify with a real provider call, then store. The key is not returned. */
  save: (provider: ProviderId, body: { api_key?: string; base_url?: string }) =>
    client
      .put<{ ok: boolean; verified: boolean; model_count: number; key_last4: string | null }>(
        `/api/llm/credentials/${provider}`,
        body,
      )
      .then((r) => r.data),
  /** Re-check the stored key; the outcome is recorded on the status row. */
  verify: (provider: ProviderId) =>
    client
      .post<{ ok: boolean; verified: boolean; model_count: number }>(`/api/llm/credentials/${provider}/verify`)
      .then((r) => r.data),
  remove: (provider: ProviderId) => client.delete(`/api/llm/credentials/${provider}`).then((r) => r.data),
  setDefault: (provider: ProviderId, model?: string) =>
    client.put<{ ok: boolean }>('/api/llm/default', { provider, model }).then((r) => r.data),
  models: (provider: ProviderId) =>
    client.post<{ models: ProviderModel[] }>(`/api/llm/${provider}/models`, {}).then((r) => r.data.models),
}

/** The server's sentence from a failed request, or `fallback`. */
export function serverMessage(e: unknown, fallback: string): string {
  const data = (e as { response?: { data?: { message?: unknown } } })?.response?.data
  return typeof data?.message === 'string' && data.message ? data.message : fallback
}
