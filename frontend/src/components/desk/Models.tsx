// Models and tiers (AGENT-012 §10; task S-6.20).
//
// Providers and keys move here from `LlmCredentialForm` on the retired agent
// page — rebuilt on the primitives rather than copied, because the original was
// written in the back-compat Tailwind aliases this phase exists to remove.
//
// The acceptance that shapes the page: **a profile with no calibration row
// renders `unmeasured` rather than a default.** A default is a claim, and a
// claim about a model nobody has measured is the kind of number that ends up in
// a decision.
import { useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Badge } from '@/components/primitives/Badge'
import { Button } from '@/components/primitives/Button'
import { Field, Input } from '@/components/primitives/Field'
import { Panel, PanelBody, PanelHeader } from '@/components/primitives/Panel'
import { EmptyState } from '@/components/primitives/States'
import { LLM_PROVIDERS, conversationsApi, llmApi, type AgentProfile, type LlmProvider } from '@/api/agent'
import { TierBadge, profileIsMeasured } from './Shared'

/** Exported so its three credential states can be exercised without a network. */
export function ProviderCard({
  provider,
  label,
  needsKey,
  configured,
  keyLast4,
  baseUrl,
}: {
  provider: LlmProvider
  label: string
  needsKey: boolean
  /**
   * `null` when `/api/llm/credentials` did not answer.
   *
   * Never folded into `false`. "not configured" is a statement about the
   * platform's stored keys, and it was being made from a request that may never
   * have come back — with a **Connect** button beside it inviting somebody to
   * paste a key the platform may already hold.
   */
  configured: boolean | null
  keyLast4: string | null
  baseUrl: string | null
}) {
  const qc = useQueryClient()
  const [apiKey, setApiKey] = useState('')
  const [url, setUrl] = useState('')
  const [error, setError] = useState('')

  const save = useMutation({
    mutationFn: async () => {
      setError('')
      const body: { api_key?: string; base_url?: string } = {}
      if (apiKey.trim()) body.api_key = apiKey.trim()
      if (url.trim()) body.base_url = url.trim()
      return (await llmApi.saveCredential(provider, body)).data
    },
    onSuccess: () => {
      setApiKey('')
      void qc.invalidateQueries({ queryKey: ['llm-credentials'] })
    },
    onError: (e: unknown) => {
      setError(
        (e as { response?: { data?: { message?: string } } })?.response?.data?.message ??
          'Verification failed — check the key and try again.',
      )
    },
  })

  const remove = useMutation({
    mutationFn: () => llmApi.deleteCredential(provider),
    onSuccess: () => void qc.invalidateQueries({ queryKey: ['llm-credentials'] }),
  })

  return (
    <details className="desk-card">
      <summary className="desk-card-hd" style={{ cursor: 'pointer' }}>
        <strong style={{ fontSize: 'var(--t-12)' }}>{label}</strong>
        {configured === null ? (
          <Badge tone="warn">state unknown</Badge>
        ) : configured ? (
          <Badge tone="pos">connected{keyLast4 ? ` ····${keyLast4}` : ''}</Badge>
        ) : (
          <Badge tone="neutral">not configured</Badge>
        )}
      </summary>
      {needsKey && (
        <Field label="API key" error={error || null}>
          <Input
            type="password"
            autoComplete="off"
            aria-label={`${label} API key`}
            value={apiKey}
            onChange={(e) => setApiKey(e.target.value)}
          />
        </Field>
      )}
      <Field label={provider === 'ollama' || provider === 'vllm' ? 'Host' : 'Base URL (optional)'}>
        <Input
          aria-label={`${label} base URL`}
          placeholder={baseUrl ?? (provider === 'ollama' ? 'http://localhost:11434' : 'provider default')}
          value={url}
          onChange={(e) => setUrl(e.target.value)}
        />
      </Field>
      <div className="desk-actions">
        <Button
          size="xs"
          disabled={needsKey && !apiKey.trim()}
          loading={save.isPending}
          onClick={() => save.mutate()}
        >
          {configured === null ? 'Save' : configured ? 'Update' : 'Connect'}
        </Button>
        {configured === true && (
          <Button size="xs" variant="secondary" loading={remove.isPending} onClick={() => remove.mutate()}>
            Remove
          </Button>
        )}
      </div>
    </details>
  )
}

// The measurement rule lives beside the badge it feeds (`Shared.tsx`), because
// three surfaces drew this badge and all three had spelled it out themselves.
// Re-exported here so `profileIsMeasured` stays importable from the page whose
// acceptance names it.
export { profileIsMeasured } from './Shared'

export function ProfileCard({ p }: { p: AgentProfile }) {
  const measured = profileIsMeasured(p)
  return (
    <Panel>
      <PanelHeader
        title={p.model_id}
        badges={<TierBadge tier={p.tier} measured={measured} detail={p.provider} />}
      />
      <PanelBody>
        <p className="sub">
          tool calling: {p.tool_calling} · max steps {p.max_steps} · {p.is_local ? 'local' : 'hosted'}
        </p>
        <p className="sub">
          hardware admission:{' '}
          {p.is_local
            ? `${p.installed === null ? 'the backend did not answer' : p.installed ? 'installed' : 'not installed'}${
                p.resident ? ', resident' : ''
              }`
            : 'not applicable'}
        </p>
        <p className="sub">
          calibration:{' '}
          {measured
            ? `${p.multi_step_evidence?.eval} · ${p.multi_step_evidence?.clean}/${p.multi_step_evidence?.trials} clean on ${p.multi_step_evidence?.device} (${p.multi_step_evidence?.recorded})`
            : 'unmeasured — no scorecard row exists for this profile'}
        </p>
        <p className="sub">
          decode ladder, exposure budget, context budget and per-suite scorecards arrive with AGENT-009 (Set S
          phase 4). The desk shows what is recorded and nothing more.
        </p>
        <div className="desk-actions">
          <Button size="xs" variant="ghost" disabled title="AGENT-009 §4 — not built yet">
            Run scorecard
          </Button>
          <Button size="xs" variant="ghost" disabled title="AGENT-009 §4 — not built yet">
            Run canary
          </Button>
        </div>
      </PanelBody>
    </Panel>
  )
}

export function Models() {
  const profiles = useQuery({
    queryKey: ['desk', 'profiles'],
    queryFn: () => conversationsApi.listProfiles().then((r) => r.data),
  })
  const creds = useQuery({
    queryKey: ['llm-credentials'],
    queryFn: () => llmApi.status().then((r) => r.data),
  })

  return (
    <div className="desk-scroll" style={{ display: 'flex', flexDirection: 'column', gap: 'var(--s-3)' }}>
      <Panel>
        <PanelHeader title="Providers and keys" />
        <PanelBody>
          {creds.data && !creds.data.encryption_available && (
            <div className="callout warn">
              Credential encryption is not configured on the platform (CRED_KEK unset) — keys cannot be stored
              until the operator sets it.
            </div>
          )}
          {LLM_PROVIDERS.map((p) => {
            const s = creds.data?.providers.find((x) => x.provider === p.id)
            return (
              <ProviderCard
                key={p.id}
                provider={p.id}
                label={p.label}
                needsKey={p.needsKey}
                configured={creds.data === undefined ? null : (s?.configured ?? false)}
                keyLast4={s?.key_last4 ?? null}
                baseUrl={s?.base_url ?? null}
              />
            )
          })}
        </PanelBody>
      </Panel>

      {(profiles.data?.profiles ?? []).map((p) => (
        <ProfileCard key={p.model_id} p={p} />
      ))}
      {/* Three states, because a page that renders nothing at all for a failed
          read is a page that says "no profiles" without having asked. Nothing
          can run without a profile, so "none configured" is an alarming claim to
          make on a request that never came back. */}
      {profiles.isPending ? (
        <p className="sub">Reading the capability profiles…</p>
      ) : profiles.data === undefined ? (
        <EmptyState
          message="The profile list did not answer"
          detail="/api/agent/profiles is served, so this is the request failing rather than the surface being unbuilt. Which models may run is unknown, not none."
        />
      ) : profiles.data.profiles.length === 0 ? (
        <EmptyState message="No capability profile is configured" detail="Nothing can run until one is." />
      ) : null}

      {(profiles.data?.unprofiled_local_models ?? []).length > 0 && (
        <Panel>
          <PanelHeader title="Local models with no profile" />
          <PanelBody>
            <p className="sub">
              These are installed but no capability profile covers them, so nothing may run on them.
            </p>
            <ul style={{ fontSize: 'var(--t-12)', paddingLeft: 'var(--s-4)' }}>
              {(profiles.data?.unprofiled_local_models ?? []).map((m) => (
                <li key={m.model_id}>{m.model_id}</li>
              ))}
            </ul>
          </PanelBody>
        </Panel>
      )}
    </div>
  )
}
