// Service tokens for headless API access (the MCP server process).
// The full token is shown exactly once, at mint time — copy it then.

import { useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Copy, KeyRound, Loader2, Trash2 } from 'lucide-react'
import { serviceTokensApi } from '@/api/agent'

export function ApiAccessCard() {
  const qc = useQueryClient()
  const [label, setLabel] = useState('')
  const [minted, setMinted] = useState<{ token: string; label: string } | null>(null)
  const [copied, setCopied] = useState(false)

  const { data } = useQuery({
    queryKey: ['service-tokens'],
    queryFn: () => serviceTokensApi.list().then((r) => r.data.tokens),
  })

  const createMutation = useMutation({
    mutationFn: (l: string) => serviceTokensApi.create(l).then((r) => r.data),
    onSuccess: (res) => {
      setMinted({ token: res.token, label: res.label })
      setCopied(false)
      setLabel('')
      qc.invalidateQueries({ queryKey: ['service-tokens'] })
    },
  })

  const revokeMutation = useMutation({
    mutationFn: (prefix: string) => serviceTokensApi.revoke(prefix),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['service-tokens'] }),
  })

  const copyToken = async () => {
    if (!minted) return
    try {
      await navigator.clipboard.writeText(minted.token)
      setCopied(true)
    } catch {
      // Clipboard unavailable — the token stays visible for manual copy.
    }
  }

  return (
    <div className="rounded-xl border border-border bg-surface-2 p-4 space-y-3">
      <div className="flex items-center gap-2">
        <KeyRound className="h-4 w-4 text-text-muted" />
        <div>
          <div className="text-sm font-medium text-text">API Access (MCP server)</div>
          <div className="text-xs text-text-dim mt-0.5">
            Mint a long-lived service token for headless clients — set it as
            PLATFORM_API_TOKEN for the trading-bot MCP server.
          </div>
        </div>
      </div>

      {minted && (
        <div className="rounded-lg bg-pos-subtle border border-line-pos px-3 py-2 space-y-1.5">
          <div className="text-xs text-pos">
            Token “{minted.label}” created — copy it now, it will not be shown again.
          </div>
          <div className="flex items-center gap-2">
            <code className="flex-1 text-[11px] font-mono text-text break-all">
              {minted.token}
            </code>
            <button
              onClick={copyToken}
              className="flex items-center gap-1 rounded-md px-2 py-1 text-xs text-text-muted hover:text-text border border-border"
            >
              <Copy className="h-3 w-3" />
              {copied ? 'Copied' : 'Copy'}
            </button>
          </div>
        </div>
      )}

      <div className="flex gap-2">
        <input
          type="text"
          value={label}
          onChange={(e) => setLabel(e.target.value)}
          placeholder="Label, e.g. mcp-server"
          className="flex-1 rounded-lg px-3 py-1.5 text-sm bg-surface border border-border text-text placeholder:text-text-dim focus:outline-none focus:ring-1 focus:ring-accent"
        />
        <button
          disabled={!label.trim() || createMutation.isPending}
          onClick={() => createMutation.mutate(label.trim())}
          className="flex items-center gap-1.5 rounded-lg px-3 py-1.5 text-sm font-medium bg-accent text-on-accent hover:bg-accent/80 disabled:opacity-40 transition-colors"
        >
          {createMutation.isPending && <Loader2 className="h-3.5 w-3.5 animate-spin" />}
          Mint token
        </button>
      </div>

      {data && data.length > 0 && (
        <div className="space-y-1.5">
          {data.map((t) => (
            <div
              key={t.token_prefix}
              className="flex items-center justify-between rounded-lg border border-border bg-surface px-3 py-1.5"
            >
              <div className="text-xs">
                <span className="font-mono text-text">{t.token_prefix}…</span>
                <span className="text-text-muted ml-2">{t.label ?? 'unlabeled'}</span>
                <span className="text-text-dim ml-2">
                  expires {new Date(t.expires_at).toLocaleDateString()}
                </span>
              </div>
              <button
                disabled={revokeMutation.isPending}
                onClick={() => revokeMutation.mutate(t.token_prefix)}
                className="text-text-dim hover:text-neg transition-colors"
                title="Revoke"
              >
                <Trash2 className="h-3.5 w-3.5" />
              </button>
            </div>
          ))}
        </div>
      )}
    </div>
  )
}
