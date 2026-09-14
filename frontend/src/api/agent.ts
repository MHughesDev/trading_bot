// Typed client for the internal agent + LLM credential APIs (Rust backend).
//
// Uses the single shared `api` instance from lib/api so there is one
// token-attaching client and one 401 interceptor across the app (#27).
import { api as client } from '@/lib/api'

// ── LLM providers / credentials ──────────────────────────────────────────────

export type LlmProvider = 'openai' | 'anthropic' | 'ollama' | 'vllm'

export const LLM_PROVIDERS: Array<{ id: LlmProvider; label: string; needsKey: boolean }> = [
  { id: 'anthropic', label: 'Anthropic', needsKey: true },
  { id: 'openai', label: 'OpenAI', needsKey: true },
  // vLLM is the production local backend (ADR-0032): it is the one that does
  // grammar-constrained decoding and the native tool template at the same time.
  // Ollama stays for the dev box, at the degraded tier.
  { id: 'vllm', label: 'Local (vLLM)', needsKey: false },
  { id: 'ollama', label: 'Local (Ollama, dev only)', needsKey: false },
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
  | 'awaiting_approval'
  | 'completed'
  | 'failed'
  // The backend broke a guarantee the harness depends on (ADR-0032). Kept distinct
  // from `failed` because the two send you to different places: `failed` means look
  // at the task, `fenced` means look at the backend.
  | 'fenced'
  // The task could not run on this tier at all, and nothing was spent.
  | 'refused'
  | 'cancelled'

export const AGENT_ACTIVE_STATUSES: AgentRunStatus[] = [
  'queued',
  'running',
  'waiting_backtest',
  'awaiting_approval',
]

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
  | 'user'
  | 'assistant'
  | 'tool_call'
  | 'tool_result'
  | 'core_tool'
  | 'status'
  | 'plan'
  | 'canary'
  | 'exposure'
  | 'policy'
  | 'validation'
  | 'compaction'
  | 'usage'
  | 'degradation'
  | 'error'
  | 'final'

export interface AgentMessage {
  seq: number
  kind: AgentMessageKind
  content: Record<string, unknown>
  created_at: string
}


export const agentApi = {
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

// ── Conversations ───────────────────────────────────────────────────
//
// The user prompts; the agent decides what to look at, how many backtests to run and
// when it is finished. There is no instrument, timeframe, iteration cap or time
// budget on this surface any more — those were never decisions a user could make
// correctly before the work started.

export interface Conversation {
  conversation_id: string
  /** null until the summariser has run; fall back to the first prompt. */
  title: string | null
  provider: string
  model: string
  created_at: string
  last_activity_at: string
  /** Whether an agent is working in this conversation right now. */
  running: boolean
  turns: number
}

export interface Turn {
  run_id: string
  turn_index: number
  status: AgentRunStatus
  /** The user's message that started this turn. */
  prompt: string
  summary: string | null
  error: string | null
  tokens_in: number
  tokens_out: number
  created_at: string
  finished_at: string | null
}

/** One thing this platform can be asked to run, from `GET /api/agent/profiles`. */
export interface AgentProfile {
  model_id: string
  provider: string
  tier: string
  is_local: boolean
  /** 'none' | 'single_shot' | 'multi_step' — what it may be asked to sustain. */
  tool_calling: string
  max_steps: number
  /** Present when a tier below the deployment target earned its multi_step claim. */
  multi_step_evidence: {
    eval: string
    device: string
    trials: number
    clean: number
    recorded: string
  } | null
  requires_api_key: boolean
  is_default: boolean
  /** Whether the local backend actually has this model. null = could not ask. */
  installed: boolean | null
  /** Whether it is loaded in memory right now, so no cold load is owed. */
  resident: boolean | null
  /** Whether the local backend answered at all. null for hosted providers. */
  backend_reachable: boolean | null
}

/** A model this machine holds that no capability profile covers, so it cannot run. */
export interface UnprofiledLocalModel {
  model_id: string
  provider: string
  resident: boolean | null
}

export const conversationsApi = {
  listProfiles: () =>
    client.get<{
      profiles: AgentProfile[]
      unprofiled_local_models: UnprofiledLocalModel[]
      default_provider: string
      default_model: string
    }>('/api/agent/profiles'),
  create: (choice?: { provider: string; model: string }) =>
    client.post<{ conversation_id: string; provider: string; model: string }>(
      '/api/agent/conversations',
      choice ?? {},
    ),
  list: () =>
    client.get<{ conversations: Conversation[] }>('/api/agent/conversations'),
  get: (id: string) =>
    client.get<{ conversation: Conversation; turns: Turn[] }>(
      `/api/agent/conversations/${id}`,
    ),
  send: (id: string, text: string) =>
    client.post<{ run_id: string }>(`/api/agent/conversations/${id}/messages`, { text }),
  cancel: (id: string) => client.post(`/api/agent/conversations/${id}/cancel`),
  archive: (id: string) => client.delete(`/api/agent/conversations/${id}`),
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
