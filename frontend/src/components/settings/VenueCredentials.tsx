// Venue Credentials section for Settings.
// Verify-before-save: credentials are health-checked at the venue before persisting.
// Credentials are never echoed back from the API — no pre-fill of secret fields.
// Per C-093/C-095.

import { useState } from 'react'
import { useMutation } from '@tanstack/react-query'
import { api } from '@/lib/api'
import { cn } from '@/lib/utils'
import { CheckCircle, XCircle, Loader2, ChevronDown, ChevronRight } from 'lucide-react'

interface VenueConfig {
  slug: string
  label: string
  fields: Array<{ name: string; label: string; placeholder: string }>
}

const VENUES: VenueConfig[] = [
  {
    slug: 'coinbase',
    label: 'Coinbase',
    fields: [
      { name: 'api_key', label: 'API Key', placeholder: 'Your Coinbase API key' },
      { name: 'api_secret', label: 'API Secret', placeholder: 'Your Coinbase API secret' },
    ],
  },
  {
    slug: 'alpaca',
    label: 'Alpaca',
    fields: [
      { name: 'api_key', label: 'API Key', placeholder: 'Your Alpaca key ID' },
      { name: 'api_secret', label: 'API Secret', placeholder: 'Your Alpaca secret' },
    ],
  },
  {
    slug: 'kraken',
    label: 'Kraken',
    fields: [
      { name: 'api_key', label: 'API Key', placeholder: 'Your Kraken API key' },
      { name: 'api_secret', label: 'API Secret', placeholder: 'Your Kraken private key' },
    ],
  },
  {
    slug: 'oanda',
    label: 'OANDA (FX Demo)',
    fields: [
      { name: 'api_key', label: 'API Token', placeholder: 'OANDA access token' },
      { name: 'account_id', label: 'Account ID', placeholder: 'OANDA account ID' },
    ],
  },
  {
    slug: 'kalshi',
    label: 'Kalshi',
    fields: [
      { name: 'api_key', label: 'API Key', placeholder: 'Kalshi API key' },
      { name: 'api_secret', label: 'API Secret', placeholder: 'Kalshi API secret' },
    ],
  },
  {
    slug: 'tradier',
    label: 'Tradier',
    fields: [
      { name: 'api_key', label: 'Access Token', placeholder: 'Tradier access token' },
    ],
  },
  {
    slug: 'tradovate',
    label: 'Tradovate (Demo)',
    fields: [
      { name: 'api_key', label: 'Username', placeholder: 'Tradovate username' },
      { name: 'api_secret', label: 'Password', placeholder: 'Tradovate password' },
    ],
  },
]

type VerifyStatus = 'idle' | 'verifying' | 'success' | 'error'

function VenueCard({ venue }: { venue: VenueConfig }) {
  const [open, setOpen] = useState(false)
  const [fields, setFields] = useState<Record<string, string>>(
    Object.fromEntries(venue.fields.map((f) => [f.name, ''])),
  )
  const [verifyStatus, setVerifyStatus] = useState<VerifyStatus>('idle')
  const [verifyError, setVerifyError] = useState('')
  const [savedAt, setSavedAt] = useState<string | null>(null)

  const setField = (name: string, value: string) => {
    setFields((prev) => ({ ...prev, [name]: value }))
    setVerifyStatus('idle')
  }

  const saveMutation = useMutation({
    mutationFn: async () => {
      // Step 1: Verify with the venue health check.
      setVerifyStatus('verifying')
      setVerifyError('')
      try {
        await api.post(`/api/venues/${venue.slug}/health`, { credentials: fields })
      } catch {
        throw new Error('Venue verification failed — check your credentials and try again.')
      }
      // Step 2: Only save on successful verification.
      await api.put('/auth/venue-credentials', {
        venue: venue.slug,
        ...fields,
      })
    },
    onSuccess: () => {
      setVerifyStatus('success')
      setSavedAt(new Date().toLocaleString())
      setFields(Object.fromEntries(venue.fields.map((f) => [f.name, ''])))
    },
    onError: (e) => {
      setVerifyStatus('error')
      setVerifyError(e instanceof Error ? e.message : 'Failed to save credentials')
    },
  })

  const disconnectMutation = useMutation({
    mutationFn: () => api.delete(`/auth/venue-credentials/${venue.slug}`),
    onSuccess: () => {
      setSavedAt(null)
      setVerifyStatus('idle')
    },
  })

  const hasInput = venue.fields.some((f) => fields[f.name])

  return (
    <div className="rounded-xl border border-border bg-surface-2">
      <button
        onClick={() => setOpen((o) => !o)}
        className="flex w-full items-center justify-between px-4 py-3 text-sm"
      >
        <span className="font-semibold text-text">{venue.label}</span>
        <div className="flex items-center gap-2">
          {savedAt ? (
            <span className="flex items-center gap-1 text-xs text-green-400">
              <CheckCircle className="h-3.5 w-3.5" />
              Connected
            </span>
          ) : (
            <span className="text-xs text-text-dim">Not connected</span>
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
          {savedAt && (
            <p className="text-xs text-text-dim">
              Last connected {savedAt}. Enter new credentials to update.
            </p>
          )}

          {venue.fields.map((f) => (
            <div key={f.name}>
              <label className="block text-xs text-text-dim mb-1">{f.label}</label>
              <input
                type="password"
                value={fields[f.name]}
                onChange={(e) => setField(f.name, e.target.value)}
                placeholder={f.placeholder}
                autoComplete="off"
                className="w-full rounded-lg px-3 py-1.5 text-sm bg-surface border border-border text-text placeholder:text-text-dim focus:outline-none focus:ring-1 focus:ring-accent"
              />
            </div>
          ))}

          {verifyStatus === 'error' && (
            <div className="flex items-start gap-2 rounded-lg bg-red-500/10 border border-red-500/20 px-3 py-2 text-xs text-red-400">
              <XCircle className="h-3.5 w-3.5 mt-0.5 shrink-0" />
              {verifyError}
            </div>
          )}
          {verifyStatus === 'success' && (
            <div className="flex items-center gap-2 rounded-lg bg-green-500/10 border border-green-500/20 px-3 py-2 text-xs text-green-400">
              <CheckCircle className="h-3.5 w-3.5 shrink-0" />
              Credentials verified and saved.
            </div>
          )}

          <div className="flex gap-2">
            <button
              disabled={!hasInput || saveMutation.isPending}
              onClick={() => saveMutation.mutate()}
              className={cn(
                'flex items-center gap-1.5 rounded-lg px-3 py-1.5 text-sm font-medium transition-colors',
                'bg-accent text-white hover:bg-accent/80 disabled:opacity-40',
              )}
            >
              {saveMutation.isPending && verifyStatus === 'verifying' && (
                <Loader2 className="h-3.5 w-3.5 animate-spin" />
              )}
              {saveMutation.isPending ? 'Verifying…' : 'Connect'}
            </button>

            {savedAt && (
              <button
                disabled={disconnectMutation.isPending}
                onClick={() => disconnectMutation.mutate()}
                className="rounded-lg px-3 py-1.5 text-sm text-text-muted hover:text-red-400 hover:bg-red-400/10 border border-border transition-colors disabled:opacity-40"
              >
                {disconnectMutation.isPending ? 'Disconnecting…' : 'Disconnect'}
              </button>
            )}
          </div>
        </div>
      )}
    </div>
  )
}

export function VenueCredentials() {
  return (
    <div className="space-y-3">
      {VENUES.map((v) => (
        <VenueCard key={v.slug} venue={v} />
      ))}
    </div>
  )
}
