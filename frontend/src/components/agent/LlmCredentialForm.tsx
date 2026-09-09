// LLM provider credential cards (OpenAI / Anthropic / local Ollama).
// Verify-before-save: keys are checked against the live provider before
// persisting, are encrypted at rest, and are never echoed back.

import { useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { CheckCircle, XCircle, Loader2, ChevronDown, ChevronRight } from 'lucide-react'
import { cn } from '@/lib/utils'
import { llmApi, LLM_PROVIDERS, type LlmProvider } from '@/api/agent'

function ProviderCard({
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
  configured: boolean
  keyLast4: string | null
  baseUrl: string | null
}) {
  const qc = useQueryClient()
  const [open, setOpen] = useState(false)
  const [apiKey, setApiKey] = useState('')
  const [url, setUrl] = useState('')
  const [error, setError] = useState('')

  const saveMutation = useMutation({
    mutationFn: async () => {
      setError('')
      const body: { api_key?: string; base_url?: string } = {}
      if (apiKey.trim()) body.api_key = apiKey.trim()
      if (url.trim()) body.base_url = url.trim()
      const res = await llmApi.saveCredential(provider, body)
      return res.data
    },
    onSuccess: () => {
      setApiKey('')
      qc.invalidateQueries({ queryKey: ['llm-credentials'] })
      qc.invalidateQueries({ queryKey: ['llm-models', provider] })
    },
    onError: (e: unknown) => {
      const detail =
        (e as { response?: { data?: { message?: string } } })?.response?.data?.message
      setError(detail ?? 'Verification failed — check the key and try again.')
    },
  })

  const deleteMutation = useMutation({
    mutationFn: () => llmApi.deleteCredential(provider),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['llm-credentials'] }),
  })

  const canSave = needsKey ? apiKey.trim().length > 0 : true

  return (
    <div className="rounded-xl border border-border bg-surface-2">
      <button
        onClick={() => setOpen((o) => !o)}
        className="flex w-full items-center justify-between px-4 py-3 text-sm"
      >
        <span className="font-semibold text-text">{label}</span>
        <div className="flex items-center gap-2">
          {configured ? (
            <span className="flex items-center gap-1 text-xs text-green-400">
              <CheckCircle className="h-3.5 w-3.5" />
              {keyLast4 ? `Connected ····${keyLast4}` : 'Connected'}
            </span>
          ) : (
            <span className="text-xs text-text-dim">Not configured</span>
          )}
          {open ? (
            <ChevronDown className="h-4 w-4 text-text-dim" />
          ) : (
            <ChevronRight className="h-4 w-4 text-text-dim" />
          )}
        </div>
      </button>

      {open && (
        <div className="border-t border-border px-4 pb-4 pt-3 space-y-3">
          {needsKey && (
            <div>
              <label className="block text-xs text-text-dim mb-1">API Key</label>
              <input
                type="password"
                value={apiKey}
                onChange={(e) => setApiKey(e.target.value)}
                placeholder={`Your ${label} API key`}
                autoComplete="off"
                className="w-full rounded-lg px-3 py-1.5 text-sm bg-surface border border-border text-text placeholder:text-text-dim focus:outline-none focus:ring-1 focus:ring-accent"
              />
            </div>
          )}
          <div>
            <label className="block text-xs text-text-dim mb-1">
              {provider === 'ollama' ? 'Ollama host' : 'Base URL (optional)'}
            </label>
            <input
              type="text"
              value={url}
              onChange={(e) => setUrl(e.target.value)}
              placeholder={
                baseUrl ??
                (provider === 'ollama' ? 'http://localhost:11434' : 'Provider default')
              }
              autoComplete="off"
              className="w-full rounded-lg px-3 py-1.5 text-sm bg-surface border border-border text-text placeholder:text-text-dim focus:outline-none focus:ring-1 focus:ring-accent"
            />
          </div>

          {error && (
            <div className="flex items-start gap-2 rounded-lg bg-red-500/10 border border-red-500/20 px-3 py-2 text-xs text-red-400">
              <XCircle className="h-3.5 w-3.5 mt-0.5 shrink-0" />
              {error}
            </div>
          )}

          <div className="flex gap-2">
            <button
              disabled={!canSave || saveMutation.isPending}
              onClick={() => saveMutation.mutate()}
              className={cn(
                'flex items-center gap-1.5 rounded-lg px-3 py-1.5 text-sm font-medium transition-colors',
                'bg-accent text-white hover:bg-accent/80 disabled:opacity-40',
              )}
            >
              {saveMutation.isPending && <Loader2 className="h-3.5 w-3.5 animate-spin" />}
              {saveMutation.isPending ? 'Verifying…' : configured ? 'Update' : 'Connect'}
            </button>
            {configured && (
              <button
                disabled={deleteMutation.isPending}
                onClick={() => deleteMutation.mutate()}
                className="rounded-lg px-3 py-1.5 text-sm text-text-muted hover:text-red-400 hover:bg-red-400/10 border border-border transition-colors disabled:opacity-40"
              >
                {deleteMutation.isPending ? 'Removing…' : 'Remove'}
              </button>
            )}
          </div>
        </div>
      )}
    </div>
  )
}

export function LlmCredentialForm() {
  const { data } = useQuery({
    queryKey: ['llm-credentials'],
    queryFn: () => llmApi.status().then((r) => r.data),
  })

  return (
    <div className="space-y-3">
      {data && !data.encryption_available && (
        <div className="rounded-lg bg-amber-500/10 border border-amber-500/20 px-3 py-2 text-xs text-amber-400">
          Credential encryption is not configured on the platform (CRED_KEK unset) —
          keys cannot be stored until the operator sets it.
        </div>
      )}
      {LLM_PROVIDERS.map((p) => {
        const status = data?.providers.find((s) => s.provider === p.id)
        return (
          <ProviderCard
            key={p.id}
            provider={p.id}
            label={p.label}
            needsKey={p.needsKey}
            configured={status?.configured ?? false}
            keyLast4={status?.key_last4 ?? null}
            baseUrl={status?.base_url ?? null}
          />
        )
      })}
    </div>
  )
}
