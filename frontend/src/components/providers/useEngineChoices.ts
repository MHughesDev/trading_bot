// The engines the signed-in user can start, read once and shared by every
// composer on the desk (PROVIDERS track).
import { useQuery } from '@tanstack/react-query'
import { conversationsApi } from '@/api/agent'
import { PROVIDERS_QUERY_KEY, llmKeysApi } from '@/api/llm'
import { engineChoices, needsProvider, type EngineChoice } from './engines'

export interface EngineChoices {
  choices: EngineChoice[]
  /** Both reads answered and nothing can run: draw the call to action. */
  needsProvider: boolean
  /** Still reading. */
  loading: boolean
}

export function useEngineChoices(): EngineChoices {
  const status = useQuery({
    queryKey: PROVIDERS_QUERY_KEY,
    queryFn: () => llmKeysApi.status(),
    staleTime: 30_000,
  })
  // The same key desk home and the conversation already use, so this is one
  // request, not a second one.
  const profiles = useQuery({
    queryKey: ['desk', 'profiles'],
    queryFn: () => conversationsApi.listProfiles().then((r) => r.data),
    staleTime: 60_000,
  })
  const profileList = profiles.data?.profiles
  return {
    choices: engineChoices(status.data, profileList),
    needsProvider: needsProvider(status.data, profileList),
    loading: status.isPending || profiles.isPending,
  }
}
