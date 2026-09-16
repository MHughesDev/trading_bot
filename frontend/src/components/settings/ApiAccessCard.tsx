// Service tokens — headless and MCP access (Set S task S-6.22).
//
// This card was lost rather than retired. The desk migration deleted
// `components/agent/ApiAccessCard`, `SettingsPage` kept importing it, and the
// only check that would have caught that — `npx tsc --noEmit` — runs against the
// root `tsconfig.json`, which is a solution file holding nothing but
// `references` and therefore checks no files at all. The real command is
// `npx tsc -p tsconfig.app.json --noEmit`.
//
// Nothing replaced it. `serviceTokensApi` and `/auth/service-tokens` both still
// exist, and the MCP server cannot start without a token this is the only way to
// mint (`PLATFORM_API_TOKEN`), so the function had to come back rather than be
// pointed somewhere else.
//
// It lives under `components/settings/` now: minting a service token is an
// account operation, not part of the agent surface the desk replaced, which is
// why it was orphaned when that surface went.
import { useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'

import { Button } from '@/components/primitives/Button'
import { Panel, PanelBody } from '@/components/primitives/Panel'
import { serviceTokensApi, type ServiceToken } from '@/api/agent'

/** The secret, shown once. */
function Minted({ token, onDone }: { token: string; onDone: () => void }) {
  const [copied, setCopied] = useState(false)
  return (
    <div className="settings-token-minted" data-testid="minted-token">
      <p className="sec">
        Copy it now. The platform stores a hash and cannot show it again — if you lose it,
        revoke it and mint another.
      </p>
      <code className="settings-token-value">{token}</code>
      <div className="settings-token-actions">
        <Button
          size="xs"
          variant="primary"
          onClick={() => {
            void navigator.clipboard?.writeText(token).then(
              () => setCopied(true),
              // A clipboard the browser refuses is not an error worth a toast:
              // the token is on screen and selectable.
              () => setCopied(false),
            )
          }}
        >
          {copied ? 'Copied' : 'Copy'}
        </Button>
        <Button size="xs" variant="ghost" onClick={onDone}>
          Done
        </Button>
      </div>
    </div>
  )
}

export function ApiAccessCard() {
  const queryClient = useQueryClient()
  const [label, setLabel] = useState('')
  const [minted, setMinted] = useState<string | null>(null)

  const tokens = useQuery({
    queryKey: ['service-tokens'],
    // `.then(r => r.data)` is the house convention: `@/lib/api` is axios, so
    // every call resolves to a response envelope rather than the body.
    queryFn: () => serviceTokensApi.list().then((r) => r.data),
  })

  const create = useMutation({
    mutationFn: (name: string) => serviceTokensApi.create(name).then((r) => r.data),
    onSuccess: (res) => {
      setMinted(res.token)
      setLabel('')
      void queryClient.invalidateQueries({ queryKey: ['service-tokens'] })
    },
  })

  const revoke = useMutation({
    mutationFn: (prefix: string) => serviceTokensApi.revoke(prefix),
    onSuccess: () => void queryClient.invalidateQueries({ queryKey: ['service-tokens'] }),
  })

  const rows: ServiceToken[] = tokens.data?.tokens ?? []

  return (
    <Panel>
      <PanelBody>
        <p className="sec settings-token-blurb">
          A service token authenticates headless callers — the MCP server reads one from{' '}
          <code>PLATFORM_API_TOKEN</code>. It is scoped to your account and revocable here.
        </p>

        {minted ? (
          <Minted token={minted} onDone={() => setMinted(null)} />
        ) : (
          <div className="settings-token-mint">
            <input
              type="text"
              className="settings-token-label"
              value={label}
              placeholder="what will use it, e.g. mcp-server on my laptop"
              onChange={(e) => setLabel(e.target.value)}
            />
            <Button
              size="xs"
              variant="primary"
              disabled={label.trim().length < 3 || create.isPending}
              onClick={() => create.mutate(label.trim())}
            >
              {create.isPending ? 'Minting…' : 'Mint token'}
            </Button>
          </div>
        )}

        {create.isError && (
          <p className="sec settings-token-error">
            The token was not minted: {String((create.error as Error)?.message ?? 'unknown error')}
          </p>
        )}

        {tokens.isLoading && <p className="sec">Loading…</p>}
        {!tokens.isLoading && rows.length === 0 && (
          <p className="sec">No service tokens. Headless callers cannot authenticate without one.</p>
        )}

        {rows.length > 0 && (
          <table className="settings-token-table">
            <thead>
              <tr>
                <th>Label</th>
                <th>Prefix</th>
                <th>Expires</th>
                <th />
              </tr>
            </thead>
            <tbody>
              {rows.map((t) => (
                <tr key={t.token_prefix}>
                  <td>{t.label ?? '—'}</td>
                  <td>
                    <code>{t.token_prefix}…</code>
                  </td>
                  <td>{new Date(t.expires_at).toLocaleDateString()}</td>
                  <td>
                    <Button
                      size="xs"
                      variant="ghost"
                      disabled={revoke.isPending}
                      onClick={() => revoke.mutate(t.token_prefix)}
                    >
                      Revoke
                    </Button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </PanelBody>
    </Panel>
  )
}
