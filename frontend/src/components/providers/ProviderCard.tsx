// One AI provider, for the signed-in user (PROVIDERS track, 2026-10-05).
//
// The card has exactly four states, and each is a statement the server made:
//
//   Not connected                        no key stored for this user
//   Connected · key ending ••••abcd ·    a stored key that last answered a real
//     verified 4 min ago                 provider call at that time
//   Key rejected: <reason>               the last check failed, in the
//                                        provider's words with the key cut out
//   state unknown                        the status read did not come back
//
// Connect is one step: paste, then **Save & test**. The server verifies the key
// with a real call before it stores anything, so a key that is saved is a key
// that worked. The key never comes back: the field is cleared on success and the
// card shows the last four characters the server reports.
import { useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Badge } from '@/components/primitives/Badge'
import { Button } from '@/components/primitives/Button'
import { Field, Input } from '@/components/primitives/Field'
import { ConfirmDialog } from '@/components/primitives/Modal'
import { Select } from '@/components/primitives/Overlay'
import { relativeTime } from '@/lib/format'
import {
  PROVIDERS_QUERY_KEY,
  llmKeysApi,
  modelEnabled,
  serverMessage,
  type ProviderMeta,
  type ProviderStatus,
} from '@/api/llm'
import { cardState } from './providers.helpers'

export interface ProviderCardProps {
  meta: ProviderMeta
  /** `null` when the status read did not answer. Never folded into "not connected". */
  status: ProviderStatus | null
  /** False when the platform cannot store keys (CRED_KEK unset). */
  canSave: boolean
}

export function ProviderCard({ meta, status, canSave }: ProviderCardProps) {
  const qc = useQueryClient()
  const [key, setKey] = useState('')
  const [host, setHost] = useState('')
  const [replacing, setReplacing] = useState(false)
  const [confirmRemove, setConfirmRemove] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const state = cardState(status)
  const connected = state === 'connected' || state === 'rejected'
  const refresh = () => void qc.invalidateQueries({ queryKey: PROVIDERS_QUERY_KEY })

  const save = useMutation({
    mutationFn: () => {
      const body: { api_key?: string; base_url?: string } = {}
      if (key.trim()) body.api_key = key.trim()
      if (host.trim()) body.base_url = host.trim()
      return llmKeysApi.save(meta.id, body)
    },
    onMutate: () => setError(null),
    onSuccess: () => {
      setKey('')
      setHost('')
      setReplacing(false)
      refresh()
    },
    onError: (e: unknown) => {
      // The pasted key stays in the field so a typo can be fixed in place.
      setError(serverMessage(e, 'The key could not be verified. Check it and try again.'))
    },
  })

  const verify = useMutation({
    mutationFn: () => llmKeysApi.verify(meta.id),
    onMutate: () => setError(null),
    // The outcome, pass or fail, is recorded on the status row; re-read it.
    onSettled: refresh,
    onError: (e: unknown) => {
      const status = (e as { response?: { status?: number } })?.response?.status
      // A rejected key is the row's `last_error`, drawn by the headline. Anything
      // else (the platform itself failing) is said here.
      if (status !== 422 && status !== 429) setError(serverMessage(e, 'The check could not run.'))
    },
  })

  const remove = useMutation({
    mutationFn: () => llmKeysApi.remove(meta.id),
    onSuccess: () => {
      setConfirmRemove(false)
      refresh()
    },
    onError: (e: unknown) => setError(serverMessage(e, 'The key could not be removed.')),
  })

  const makeDefault = useMutation({
    mutationFn: (model?: string) => llmKeysApi.setDefault(meta.id, model),
    onSuccess: refresh,
    onError: (e: unknown) => setError(serverMessage(e, 'The default could not be saved.')),
  })

  // Only asked for once a key is connected: listing needs one.
  const models = useQuery({
    queryKey: ['llm-models', meta.id, status?.verified_at ?? null],
    queryFn: () => llmKeysApi.models(meta.id),
    enabled: state === 'connected',
    staleTime: 5 * 60_000,
    retry: false,
  })

  const showForm = canSave && (state === 'disconnected' || replacing)
  const label = meta.label
  // A default stored before the model policy may name a model that is no longer
  // enabled; the server opens conversations on the baseline, so show that.
  const storedModel = status?.default_model ?? null
  const currentModel =
    (storedModel && modelEnabled(meta.id, storedModel) ? storedModel : meta.suggestedModel) ?? null
  const modelOptions = (() => {
    const ids = new Set((models.data ?? []).map((m) => m.id).filter((id) => modelEnabled(meta.id, id)))
    if (currentModel) ids.add(currentModel)
    return [...ids].sort().map((id) => ({ value: id, label: id }))
  })()

  return (
    <div className="well" data-testid={`provider-card-${meta.id}`} style={{ display: 'flex', flexDirection: 'column', gap: 'var(--s-2)' }}>
      <div className="desk-card-hd" style={{ display: 'flex', alignItems: 'center', gap: 'var(--s-2)', flexWrap: 'wrap' }}>
        <strong style={{ fontSize: 'var(--t-13)' }}>{label}</strong>
        {state === 'unknown' && <Badge tone="warn">state unknown</Badge>}
        {state === 'disconnected' && <Badge tone="neutral">Not connected</Badge>}
        {state === 'connected' && <Badge tone="pos">Connected</Badge>}
        {state === 'rejected' && <Badge tone="neg">Key rejected</Badge>}
        {connected && status?.is_default && <Badge tone="accent">default</Badge>}
      </div>

      {state === 'unknown' && (
        <p className="sub">
          The platform did not say whether a {label} key is connected. That is not the same as none being
          connected, so there is nothing to paste here until it answers.
        </p>
      )}
      {connected && status && (
        <p className="sub" data-testid={`provider-line-${meta.id}`}>
          {status.key_last4 ? `key ending ••••${status.key_last4}` : meta.hosted ? 'key stored' : (status.base_url ?? 'default host')}
          {status.verified_at ? ` · verified ${relativeTime(status.verified_at)}` : ' · never verified'}
        </p>
      )}
      {state === 'rejected' && status?.last_error && (
        <p className="sub" role="alert" style={{ color: 'var(--fg-neg)' }} data-testid={`provider-error-${meta.id}`}>
          Key rejected: {status.last_error}
        </p>
      )}

      {showForm && (
        <form
          onSubmit={(e) => {
            e.preventDefault()
            if (meta.hosted && !key.trim()) return
            save.mutate()
          }}
          style={{ display: 'flex', flexDirection: 'column', gap: 'var(--s-2)' }}
        >
          {meta.hosted ? (
            <Field
              label="API key"
              error={error ? `Key rejected: ${error}` : null}
              hint={
                meta.keysUrl ? (
                  <>
                    Create one at{' '}
                    <a href={meta.keysUrl} target="_blank" rel="noreferrer noopener">
                      {meta.keysUrl.replace('https://', '')}
                    </a>
                    . It is verified, encrypted at rest, and never shown again.
                  </>
                ) : undefined
              }
            >
              <Input
                type="password"
                autoComplete="off"
                spellCheck={false}
                aria-label={`${label} API key`}
                placeholder={meta.keyPlaceholder}
                value={key}
                onChange={(e) => setKey(e.target.value)}
              />
            </Field>
          ) : (
            <Field label="Host" error={error} hint="Leave empty for the default local address.">
              <Input
                aria-label={`${label} host`}
                placeholder={meta.id === 'ollama' ? 'http://localhost:11434' : 'http://localhost:8000'}
                value={host}
                onChange={(e) => setHost(e.target.value)}
              />
            </Field>
          )}
          <div className="desk-actions" style={{ display: 'flex', gap: 'var(--s-2)' }}>
            <Button
              type="submit"
              size="sm"
              variant="primary"
              loading={save.isPending}
              disabled={meta.hosted && !key.trim()}
            >
              Save &amp; test
            </Button>
            {replacing && (
              <Button
                size="sm"
                variant="ghost"
                onClick={() => {
                  setReplacing(false)
                  setKey('')
                  setError(null)
                }}
              >
                Cancel
              </Button>
            )}
          </div>
        </form>
      )}

      {connected && !replacing && (
        <>
          {meta.hosted && state === 'connected' && (
            <Field label="Default model" hint="The model a new desk conversation on this provider starts with.">
              <Select<string>
                value={currentModel}
                onChange={(m) => makeDefault.mutate(m)}
                ariaLabel={`${label} default model`}
                small
                width={260}
                disabled={models.isPending && !currentModel}
                options={modelOptions}
              />
            </Field>
          )}
          {models.isError && state === 'connected' && (
            <p className="sub">The model list did not load; the default model above still applies.</p>
          )}
          {error && (
            <p className="sub" role="alert" style={{ color: 'var(--fg-neg)' }}>
              {error}
            </p>
          )}
          <div className="desk-actions" style={{ display: 'flex', gap: 'var(--s-2)', flexWrap: 'wrap' }}>
            <Button size="xs" loading={verify.isPending} onClick={() => verify.mutate()}>
              Test again
            </Button>
            {canSave && (
              <Button size="xs" onClick={() => setReplacing(true)}>
                Replace
              </Button>
            )}
            {!status?.is_default && state === 'connected' && (
              <Button size="xs" loading={makeDefault.isPending} onClick={() => makeDefault.mutate(undefined)}>
                Make default
              </Button>
            )}
            <Button size="xs" variant="danger" onClick={() => setConfirmRemove(true)}>
              Remove
            </Button>
          </div>
        </>
      )}

      <ConfirmDialog
        open={confirmRemove}
        onCancel={() => setConfirmRemove(false)}
        onConfirm={() => remove.mutate()}
        busy={remove.isPending}
        title={`Remove your ${label} key?`}
        consequence={`The key ending ••••${status?.key_last4 ?? '????'} is deleted from this platform. New conversations on ${label} cannot start until you connect a key again; the key itself stays valid at ${label} until you revoke it there.`}
        confirmLabel="Remove key"
      />
    </div>
  )
}
