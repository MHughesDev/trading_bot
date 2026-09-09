// Typed client for the internal agent + LLM credential APIs (Rust backend).
//
// Uses the single shared `api` instance from lib/api so there is one
// token-attaching client and one 401 interceptor across the app (#27).
import { api as client } from '@/lib/api'

// ── LLM providers / credentials ──────────────────────────────────────────────

export type LlmProvider = 'openai' | 'anthropic' | 'ollama'

export const LLM_PROVIDERS: Array<{ id: LlmProvider; label: string; needsKey: boolean }> = [
  { id: 'anthropic', label: 'Anthropic', needsKey: true },
  { id: 'openai', label: 'OpenAI', needsKey: true },
  { id: 'ollama', label: 'Local (Ollama)', needsKey: false },
]

export interface LlmCredentialStatus {
  provider: LlmProvider
  configured: boolean
  key_last4: string | null
  base_url: string | null
  updated_at: string | null
}

export interface LlmModelInfo {
  id: string
  display_name: string | null
}

export const llmApi = {
  status: () =>
    client.get<{ providers: LlmCredentialStatus[]; encryption_available: boolean }>(
      '/api/llm/credentials',
    ),
  saveCredential: (provider: LlmProvider, body: { api_key?: string; base_url?: string }) =>
    client.put<{ ok: boolean; verified: boolean; model_count: number }>(
      `/api/llm/credentials/${provider}`,
      body,
    ),
  deleteCredential: (provider: LlmProvider) =>
    client.delete(`/api/llm/credentials/${provider}`),
  listModels: (provider: LlmProvider, body?: { api_key?: string; base_url?: string }) =>
    client.post<{ models: LlmModelInfo[] }>(`/api/llm/${provider}/models`, body ?? {}),
}

// ── Agent runs ───────────────────────────────────────────────────────────────

export type AgentRunStatus =
  | 'queued'
  | 'running'
  | 'waiting_backtest'
  | 'completed'
  | 'failed'
  | 'cancelled'

export const AGENT_ACTIVE_STATUSES: AgentRunStatus[] = ['queued', 'running', 'waiting_backtest']

export interface AgentRun {
  run_id: string
  status: AgentRunStatus
  goal: string
  provider: LlmProvider
  model: string
  constraints: Record<string, unknown> | null
  iterations: number
  max_iterations: number
  tokens_in: number
  tokens_out: number
  max_total_tokens: number | null
  wallclock_budget_secs: number
  error: string | null
  summary: string | null
  final_strategy_id: string | null
  best_backtest_id: string | null
  created_at: string
  started_at: string | null
  finished_at: string | null
}

export type AgentMessageKind =
  | 'assistant'
  | 'tool_call'
  | 'tool_result'
  | 'status'
  | 'error'
  | 'final'

export interface AgentMessage {
  seq: number
  kind: AgentMessageKind
  content: Record<string, unknown>
  created_at: string
}

export interface StartRunRequest {
  goal: string
  provider: LlmProvider
  model: string
  constraints?: {
    instrument_id?: string
    timeframe?: string
    start?: string
    end?: string
    initial_balance?: string
  }
  max_iterations?: number
  max_total_tokens?: number
  wallclock_budget_secs?: number
}

export const agentApi = {
  start: (req: StartRunRequest) =>
    client.post<{ run_id: string }>('/api/agent/runs', req),
  list: (params?: { limit?: number; offset?: number }) =>
    client.get<{ runs: AgentRun[]; total: number }>('/api/agent/runs', { params }),
  get: (runId: string) => client.get<AgentRun>(`/api/agent/runs/${runId}`),
  messages: (runId: string, params?: { after_seq?: number; limit?: number }) =>
    client.get<{ messages: AgentMessage[]; last_seq: number }>(
      `/api/agent/runs/${runId}/messages`,
      { params },
    ),
  cancel: (runId: string) => client.post(`/api/agent/runs/${runId}/cancel`),
}

// ── Service tokens (headless API access, e.g. the MCP server) ────────────────

export interface ServiceToken {
  token_prefix: string
  label: string | null
  created_at: string
  expires_at: string
}

export const serviceTokensApi = {
  list: () => client.get<{ tokens: ServiceToken[] }>('/auth/service-tokens'),
  create: (label: string) =>
    client.post<{ token: string; token_prefix: string; label: string; expires_at: string }>(
      '/auth/service-tokens',
      { label },
    ),
  revoke: (tokenPrefix: string) => client.delete(`/auth/service-tokens/${tokenPrefix}`),
}
