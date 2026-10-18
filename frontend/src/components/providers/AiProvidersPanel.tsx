// AI providers: the one place a user connects their own keys (PROVIDERS track).
//
// Settings holds it; the desk's call to action and the Models page link here.
// One place, because two forms for one key are two places for it to be wrong.
import { useQuery } from '@tanstack/react-query'
import { Panel, PanelBody } from '@/components/primitives/Panel'
import { ErrorState, PanelLoading } from '@/components/primitives/States'
import { PROVIDERS, PROVIDERS_QUERY_KEY, llmKeysApi } from '@/api/llm'
import { ProviderCard } from './ProviderCard'

export const SUBSCRIPTIONS_NOTE =
  "Anthropic and OpenAI only allow API keys in third-party apps; subscriptions (Claude Pro/Max, ChatGPT Plus) can't be connected here."

export function AiProvidersPanel() {
  const status = useQuery({
    queryKey: PROVIDERS_QUERY_KEY,
    queryFn: () => llmKeysApi.status(),
  })
  const data = status.data
  const canSave = data?.encryption_available !== false
  const hosted = PROVIDERS.filter((p) => p.hosted)
  const local = PROVIDERS.filter((p) => !p.hosted)

  return (
    <Panel>
      <PanelBody>
        <p className="sec" style={{ fontSize: 'var(--t-12)', lineHeight: 1.55, marginBottom: 'var(--s-3)' }}>
          The research desk runs on your own API key, billed to your account at the provider. Paste a key and
          it is tested with a real call before it is saved, encrypted at rest, and never shown again — only
          its last four characters.
        </p>
        <p className="sub" data-testid="subscriptions-note" style={{ marginBottom: 'var(--s-4)' }}>
          {SUBSCRIPTIONS_NOTE} Get a key at{' '}
          <a href="https://console.anthropic.com/settings/keys" target="_blank" rel="noreferrer noopener">
            console.anthropic.com/settings/keys
          </a>{' '}
          or{' '}
          <a href="https://platform.openai.com/api-keys" target="_blank" rel="noreferrer noopener">
            platform.openai.com/api-keys
          </a>
          .
        </p>

        {data && !data.encryption_available && (
          <div className="callout warn" role="alert" data-testid="encryption-problem" style={{ marginBottom: 'var(--s-4)' }}>
            {data.encryption_problem ??
              "Keys can't be saved: the operator must set CRED_KEK (64 hex characters) and restart the platform."}
          </div>
        )}

        {status.isPending ? (
          <PanelLoading lines={3} />
        ) : status.isError ? (
          <ErrorState
            message="Your providers could not be read"
            detail="This is not the same as none being connected. Nothing here was changed."
            onRetry={() => void status.refetch()}
          />
        ) : null}

        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--s-3)' }}>
          {hosted.map((meta) => (
            <ProviderCard
              key={meta.id}
              meta={meta}
              status={data ? (data.providers.find((p) => p.provider === meta.id) ?? null) : null}
              canSave={canSave}
            />
          ))}
        </div>

        <details style={{ marginTop: 'var(--s-4)' }}>
          <summary className="sub" style={{ cursor: 'pointer' }}>
            Local models (vLLM, Ollama) — no key, runs on hardware you own
          </summary>
          <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--s-3)', marginTop: 'var(--s-3)' }}>
            {local.map((meta) => (
              <ProviderCard
                key={meta.id}
                meta={meta}
                status={data ? (data.providers.find((p) => p.provider === meta.id) ?? null) : null}
                canSave={canSave}
              />
            ))}
          </div>
        </details>
      </PanelBody>
    </Panel>
  )
}
