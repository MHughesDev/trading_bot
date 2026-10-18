// Pure rules for the provider cards (PROVIDERS track). Kept apart from the
// components so the components file exports components only (react-refresh).
import type { ProviderStatus } from '@/api/llm'

export type CardState = 'unknown' | 'disconnected' | 'rejected' | 'connected'

/**
 * What a provider card's headline says.
 *
 * `null` is a status read that did not answer, and it stays `unknown`: "Not
 * connected" beside a Connect button is an invitation to paste a key the
 * platform may already hold.
 */
export function cardState(status: ProviderStatus | null): CardState {
  if (status === null) return 'unknown'
  if (!status.configured) return 'disconnected'
  if (status.last_error) return 'rejected'
  return 'connected'
}
