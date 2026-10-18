// What the desk may start a conversation on, for this user (PROVIDERS track).
//
// An "engine" is a provider and a model together, because that pair is what a
// conversation row stores and what the agent manager builds a client from. The
// composer used to offer capability profiles alone, so every conversation was
// opened on the server's default provider (Anthropic) whatever the user had
// connected: an OpenAI-only user could pick nothing that ran.
//
// Pure, so the rules are tested without a network.
import type { AgentProfile } from '@/api/agent'
import { PROVIDERS, modelEnabled, providerMeta, type ProviderId, type ProvidersStatus } from '@/api/llm'
import { profileIsMeasured } from '@/components/desk/Shared.helpers'

export interface EngineChoice {
  /** `provider|model`. Model ids may hold `:` and `/`, never `|`. */
  id: string
  provider: ProviderId
  model: string
  label: string
  tier: string
  measured: boolean
  frontier: boolean
  isDefault: boolean
}

export function engineId(provider: string, model: string): string {
  return `${provider}|${model}`
}

export function parseEngineId(id: string): { provider: ProviderId; model: string } | null {
  const cut = id.indexOf('|')
  if (cut <= 0) return null
  const provider = id.slice(0, cut)
  const model = id.slice(cut + 1)
  if (!providerMeta(provider) || !model) return null
  return { provider: provider as ProviderId, model }
}

/**
 * The provider a profile runs on, as the credential store names it.
 *
 * The Claude profile says `anthropic-agent-sdk`, which is the container runtime's
 * name; the conversation and the key are both `anthropic`.
 */
export function profileProvider(p: Pick<AgentProfile, 'provider'>): ProviderId | null {
  const raw = p.provider.toLowerCase()
  if (raw.includes('anthropic')) return 'anthropic'
  if (raw.includes('openai')) return 'openai'
  if (raw.includes('vllm')) return 'vllm'
  if (raw.includes('ollama')) return 'ollama'
  return null
}

function choice(
  provider: ProviderId,
  model: string,
  profiles: AgentProfile[],
  isDefault: boolean,
): EngineChoice {
  const profile = profiles.find((p) => p.model_id === model)
  // A hosted model with no profile of its own runs under its provider's frontier
  // profile (agent manager `profile_for`), and nobody has measured it.
  const tier = profile?.tier ?? 'frontier'
  return {
    id: engineId(provider, model),
    provider,
    model,
    label: `${providerMeta(provider)?.label ?? provider} · ${model}`,
    tier,
    measured: profile ? profileIsMeasured(profile) : false,
    frontier: tier === 'frontier',
    isDefault,
  }
}

/**
 * Every engine this user can actually start, default first.
 *
 * - A hosted provider appears only when this user has connected a key for it:
 *   its chosen default model, then any shipped profile for it.
 * - A local profile appears only when its backend answered and holds the model.
 *   One that "might" be there is a send button that fails after the click.
 */
export function engineChoices(
  status: ProvidersStatus | undefined,
  profiles: AgentProfile[] | undefined,
): EngineChoice[] {
  const ps = profiles ?? []
  const rows = status?.providers ?? []
  const defaultProvider = rows.find((r) => r.configured && r.is_default)?.provider
  const out: EngineChoice[] = []
  const push = (c: EngineChoice) => {
    if (!out.some((o) => o.id === c.id)) out.push(c)
  }

  const hosted = PROVIDERS.filter((m) => m.hosted)
    .map((m) => ({ meta: m, row: rows.find((r) => r.provider === m.id) }))
    .filter((x) => x.row?.configured)
    // The default provider first, then the order the settings page draws.
    .sort((a, b) => Number(b.meta.id === defaultProvider) - Number(a.meta.id === defaultProvider))

  for (const { meta, row } of hosted) {
    // Only models the platform will call: a stored default from before the model
    // policy falls back to the provider's baseline.
    const stored = row?.default_model
    const model = (stored && modelEnabled(meta.id, stored) ? stored : meta.suggestedModel) ?? ''
    if (model) push(choice(meta.id, model, ps, meta.id === defaultProvider && out.length === 0))
    for (const p of ps) {
      if (!p.is_local && profileProvider(p) === meta.id && modelEnabled(meta.id, p.model_id)) {
        push(choice(meta.id, p.model_id, ps, false))
      }
    }
  }

  for (const p of ps) {
    const provider = profileProvider(p)
    if (!p.is_local || !provider) continue
    if (p.backend_reachable !== true || p.installed === false) continue
    push(choice(provider, p.model_id, ps, false))
  }

  if (out.length > 0 && !out.some((c) => c.isDefault)) out[0] = { ...out[0], isDefault: true }
  return out
}

/**
 * Whether the person must connect a provider before anything can run.
 *
 * Only when both reads answered and neither offered an engine. A request that
 * never came back is not evidence that nothing is connected, and a call to
 * action over a key the platform already holds is a wrong instruction.
 */
export function needsProvider(
  status: ProvidersStatus | undefined,
  profiles: AgentProfile[] | undefined,
): boolean {
  if (status === undefined || profiles === undefined) return false
  return engineChoices(status, profiles).length === 0
}

/**
 * The engine an existing conversation runs on, and whether its provider's key
 * is gone.
 *
 * A conversation's provider and model are fixed when it is opened, so inside one
 * the composer shows that engine rather than offering a switch that would not
 * apply. `keyMissing` is only claimed when the status read answered.
 */
export function conversationEngine(
  convo: { provider: string; model: string } | null | undefined,
  status: ProvidersStatus | undefined,
  profiles: AgentProfile[] | undefined,
): { choice: EngineChoice; keyMissing: boolean } | null {
  if (!convo) return null
  const meta = providerMeta(convo.provider)
  if (!meta) return null
  const row = status?.providers.find((r) => r.provider === meta.id)
  return {
    choice: choice(meta.id, convo.model, profiles ?? [], true),
    keyMissing: meta.hosted && status !== undefined && !row?.configured,
  }
}
