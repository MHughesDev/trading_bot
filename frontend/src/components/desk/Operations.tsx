// Usage, monitor and the strategy editor (AGENT-012 §8, §11, §12; task S-6.21).
//
// Three surfaces that share one property: they are the operator's, not the
// researcher's. Each is wired against the endpoint its owning phase will provide
// and says plainly when that phase has not landed, rather than rendering a
// placeholder number somebody might believe.
import { useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { Badge } from '@/components/primitives/Badge'
import { Button } from '@/components/primitives/Button'
import { ConfirmDialog } from '@/components/primitives/Modal'
import { Panel, PanelBody, PanelHeader } from '@/components/primitives/Panel'
import { TextArea } from '@/components/primitives/Field'
import { EmptyState } from '@/components/primitives/States'
import { api } from '@/lib/api'
import { deskApi } from '@/api/desk'

// ── Usage (§12) ──────────────────────────────────────────────────────────────

export function Usage() {
  const usage = useQuery({
    queryKey: ['desk', 'usage'],
    queryFn: () => deskApi.usage().then((r) => r.data),
  })
  const u = usage.data as Record<string, number> | undefined

  return (
    <div className="desk-scroll">
      <Panel>
        <PanelHeader title="Spend" />
        <PanelBody>
          {!u ? (
            <EmptyState message="No usage recorded" detail="/api/agent/usage answered with nothing." />
          ) : (
            <table className="tbl">
              <tbody>
                {Object.entries(u).map(([k, v]) => (
                  <tr key={k}>
                    <td style={{ textAlign: 'left' }}>{k}</td>
                    <td>{typeof v === 'number' ? v.toLocaleString() : String(v)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
          <p className="sub">
            Spend by day, by conversation, by model and by layer, cost per finding and per gate, and budget
            policies come from AGENT-009's published numbers and the per-layer accounting in AGENT-005. Neither
            is exposed by an endpoint yet, so the desk shows the totals that are.
          </p>
          <Button
            size="xs"
            variant="secondary"
            onClick={() => {
              if (!u) return
              const csv = ['metric,value', ...Object.entries(u).map(([k, v]) => `${k},${v}`)].join('\n')
              const url = URL.createObjectURL(new Blob([csv], { type: 'text/csv' }))
              const a = document.createElement('a')
              a.href = url
              a.download = 'desk-usage.csv'
              a.click()
              URL.revokeObjectURL(url)
            }}
          >
            Export CSV
          </Button>
        </PanelBody>
      </Panel>
    </div>
  )
}

// ── Monitor (§11 — the UI of COMP-007) ───────────────────────────────────────

export function Monitor() {
  const [halting, setHalting] = useState(false)
  const strategies = useQuery({ queryKey: ['desk', 'monitor'], queryFn: () => deskApi.monitor() })
  const kill = useQuery({
    queryKey: ['desk', 'kill-switch'],
    queryFn: () => deskApi.killSwitch(),
    refetchInterval: 3_000,
  })
  const tripped = kill.data?.kill_switch_active === true

  return (
    <div className="desk-scroll" style={{ display: 'flex', flexDirection: 'column', gap: 'var(--s-3)' }}>
      <Panel>
        <PanelHeader
          title="Kill switch"
          badges={<Badge tone={tripped ? 'neg' : 'pos'}>{tripped ? 'tripped' : 'armed'}</Badge>}
        />
        <PanelBody>
          <p className="sub">
            scope {kill.data?.scope ?? 'global'} · tripped by {kill.data?.tripped_by ?? 'not recorded'} ·{' '}
            {kill.data?.reason ?? 'no reason recorded'}
          </p>
          {tripped ? (
            <Button size="xs" variant="secondary" onClick={() => void api.post('/api/trading/resume')}>
              Reset (logged)
            </Button>
          ) : (
            <Button size="xs" variant="danger" onClick={() => setHalting(true)}>
              Halt all
            </Button>
          )}
          <ConfirmDialog
            open={halting}
            onCancel={() => setHalting(false)}
            title="Halt all trading?"
            consequence="Every strategy stops placing orders immediately, on every venue. Open positions are not closed. The reset is logged and requires typing HALT."
            typeToConfirm="HALT"
            confirmLabel="Halt all"
            onConfirm={() => {
              setHalting(false)
              void api.post('/api/trading/kill')
            }}
          />
        </PanelBody>
      </Panel>

      <Panel>
        <PanelHeader title="Strategies" />
        <PanelBody>
          {strategies.data === null || strategies.data === undefined ? (
            <EmptyState
              message="The monitor is not built yet"
              detail="/api/monitor/strategies belongs to COMP-007 (Set S phase 7, track 2). This page renders the moment it answers; it will not guess a status for anything live."
            />
          ) : (
            <table className="tbl">
              <thead>
                <tr>
                  <th style={{ textAlign: 'left' }}>strategy</th>
                  <th>status</th>
                  <th>breaching metric</th>
                  <th />
                </tr>
              </thead>
              <tbody>
                {strategies.data.strategies.map((s) => {
                  const row = s as Record<string, unknown>
                  return (
                    <tr key={String(row.id ?? row.name)}>
                      <td style={{ textAlign: 'left' }}>{String(row.name ?? '')}</td>
                      <td>{String(row.status ?? '')}</td>
                      <td>{String(row.breaching_metric ?? '—')}</td>
                      <td>
                        <Button size="xs" variant="ghost">
                          Scale down
                        </Button>
                      </td>
                    </tr>
                  )
                })}
              </tbody>
            </table>
          )}
        </PanelBody>
      </Panel>
    </div>
  )
}

// ── Strategy editor (§8) ─────────────────────────────────────────────────────

export interface ValidatorError {
  line: number
  message: string
  /** The edit that fixes it, applied verbatim. */
  fix: string | null
}

export function StrategyEditor({ initial = '' }: { initial?: string }) {
  const [text, setText] = useState(initial)
  const [errors, setErrors] = useState<ValidatorError[] | null>(null)
  const [preview, setPreview] = useState<Record<string, unknown> | null>(null)

  async function validate() {
    try {
      const { data } = await api.post<{ errors: ValidatorError[] }>('/api/strategies/compile', { source: text })
      setErrors(data.errors ?? [])
    } catch {
      setErrors(null)
    }
  }

  async function runPreview() {
    try {
      const { data } = await api.post<Record<string, unknown>>('/api/strategies/preview', { source: text })
      setPreview(data)
    } catch {
      setPreview(null)
    }
  }

  return (
    <div className="desk-scroll" style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 'var(--s-3)' }}>
      <Panel>
        <PanelHeader
          title="SLv2"
          actions={
            <>
              <Button size="xs" onClick={() => void validate()}>
                Validate
              </Button>
              <Button size="xs" variant="secondary" onClick={() => void runPreview()}>
                Preview
              </Button>
            </>
          }
        />
        <PanelBody>
          <TextArea
            rows={18}
            aria-label="Strategy source"
            value={text}
            onChange={(e) => setText(e.target.value)}
            className="mono"
          />
          {errors === null ? (
            <p className="sub">
              The compiler endpoint (/api/strategies/compile) belongs to AGENT-011, Set S phase 5. Until it
              answers, this editor does not claim your strategy is valid.
            </p>
          ) : errors.length === 0 ? (
            <p className="sub">No validator errors.</p>
          ) : (
            <ul style={{ fontSize: 'var(--t-12)', paddingLeft: 'var(--s-4)' }}>
              {errors.map((e, i) => (
                <li key={i}>
                  line {e.line}: {e.message}{' '}
                  {e.fix && (
                    <Button size="xs" variant="ghost" onClick={() => setText(e.fix as string)}>
                      Apply fix
                    </Button>
                  )}
                </li>
              ))}
            </ul>
          )}
        </PanelBody>
      </Panel>
      <Panel>
        <PanelHeader title="preview_signals" />
        <PanelBody>
          {preview === null ? (
            <EmptyState message="No preview" detail="Preview needs the feature engine's preview_signals (AGENT-011)." />
          ) : (
            <pre className="mono well" style={{ fontSize: 'var(--t-11)', overflow: 'auto', maxHeight: '50vh' }}>
              {JSON.stringify(preview, null, 2)}
            </pre>
          )}
        </PanelBody>
      </Panel>
    </div>
  )
}
