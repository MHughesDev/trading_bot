// Service tokens for headless API access (the MCP server process).
// The full token is shown exactly once, at mint time — copy it then.
//
// Moved here from `components/agent/` when the desk replaced that surface. It
// was never an agent component: it is a settings concern that happened to live
// next to the agent page. Rebuilt on the primitives, which also takes five
// back-compat alias usages out of the tree.
import { useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Copy, Trash2 } from 'lucide-react'
import { Button, IconButton } from '@/components/primitives/Button'
import { Input } from '@/components/primitives/Field'

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

  const create = useMutation({
    mutationFn: (l: string) => serviceTokensApi.create(l).then((r) => r.data),
    onSuccess: (res) => {
      setMinted({ token: res.token, label: res.label })
      setCopied(false)
      setLabel('')
      void qc.invalidateQueries({ queryKey: ['service-tokens'] })
    },
  })

  const revoke = useMutation({
    mutationFn: (prefix: string) => serviceTokensApi.revoke(prefix),
    onSuccess: () => void qc.invalidateQueries({ queryKey: ['service-tokens'] }),
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
    <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--s-3)' }}>
      <p className="sec" style={{ fontSize: 'var(--t-12)', lineHeight: 1.55, margin: 0 }}>
        Mint a long-lived service token for headless clients — set it as <span className="mono">PLATFORM_API_TOKEN</span>{' '}
        for the trading-bot MCP server.
      </p>

      {minted && (
        <div className="callout pos">
          <div>
            <div style={{ fontSize: 'var(--t-12)' }}>
              Token “{minted.label}” created — copy it now, it will not be shown again.
            </div>
            <div style={{ display: 'flex', alignItems: 'center', gap: 'var(--s-2)', marginTop: 'var(--s-2)' }}>
              <code className="mono" style={{ flex: 1, fontSize: 'var(--t-11)', wordBreak: 'break-all' }}>
                {minted.token}
              </code>
              <Button size="xs" variant="secondary" onClick={() => void copyToken()}>
                <Copy size={12} aria-hidden /> {copied ? 'Copied' : 'Copy'}
              </Button>
            </div>
          </div>
        </div>
      )}

      <div style={{ display: 'flex', gap: 'var(--s-2)' }}>
        <Input
          aria-label="Token label"
          placeholder="Label, e.g. mcp-server"
          value={label}
          onChange={(e) => setLabel(e.target.value)}
        />
        <Button
          size="sm"
          variant="primary"
          disabled={!label.trim()}
          loading={create.isPending}
          onClick={() => create.mutate(label.trim())}
        >
          Mint token
        </Button>
      </div>

      {data && data.length > 0 && (
        <ul style={{ listStyle: 'none', margin: 0, padding: 0, display: 'flex', flexDirection: 'column', gap: 'var(--s-2)' }}>
          {data.map((t) => (
            <li key={t.token_prefix} className="well between">
              <span style={{ fontSize: 'var(--t-11)' }}>
                <span className="mono">{t.token_prefix}…</span> <span className="sec">{t.label ?? 'unlabeled'}</span>{' '}
                <span className="sub">expires {new Date(t.expires_at).toLocaleDateString()}</span>
              </span>
              <IconButton
                label="Revoke this token"
                disabled={revoke.isPending}
                onClick={() => revoke.mutate(t.token_prefix)}
              >
                <Trash2 size={13} aria-hidden />
              </IconButton>
            </li>
          ))}
        </ul>
      )}
    </div>
  )
}
