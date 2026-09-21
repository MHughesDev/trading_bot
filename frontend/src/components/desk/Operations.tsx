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

// ── What the runs produced (AGENT-013 §10; S-10.13) ──────────────────────────
//
// The one number here that is not about whether the harness *worked*. A barren
// run finished perfectly: it spent from the trial budget — the quantity every
// selection-adjusted statistic on this platform depends on — and produced
// nothing the next run can use.
//
// Rendered as rows and never as a single rate. "12% barren" is something nobody
// can act on; "`survey` on the 7B is barren 60% of the time" names a tier and a
// workflow, so somebody can change one of them.

/** `null` and `0` are different answers, and only one of them is a measurement. */
export function formatRate(rate: number | null, runs: number): string {
  if (rate === null || runs === 0) return 'not measured'
  return `${Math.round(rate * 100)}%`
}

function BarrenPanel() {
  const q = useQuery({
    queryKey: ['desk', 'barren'],
    queryFn: () => deskApi.barren(30),
  })
  const d = q.data

  return (
    <Panel>
      <PanelHeader title="What the runs produced" />
      <PanelBody>
        {d === null && (
          <EmptyState
            message="Not available"
            detail="/api/lifecycle/barren is served, so this is the endpoint failing rather than the endpoint missing."
          />
        )}
        {d && d.rows.length === 0 ? (
          <EmptyState
            message="Nothing measured"
            detail={`No run finished in the last ${d.days} days, so there is no rate — which is not the same as a rate of zero.`}
          />
        ) : (
          d && (
            <table className="tbl">
              <thead>
                <tr>
                  <th style={{ textAlign: 'left' }}>Tier</th>
                  <th style={{ textAlign: 'left' }}>Workflow</th>
                  <th>Runs</th>
                  <th>Barren</th>
                  <th>Rate</th>
                </tr>
              </thead>
              <tbody>
                {d.rows.map((r) => (
                  <tr key={`${r.tier}|${r.workflow}`}>
                    <td style={{ textAlign: 'left' }}>{r.tier}</td>
                    <td style={{ textAlign: 'left' }}>{r.workflow}</td>
                    {/* The denominator travels with the rate: 60% over five runs
                        and over five hundred are different claims. */}
                    <td>{r.runs}</td>
                    <td>{r.barren}</td>
                    <td>{formatRate(r.rate, r.runs)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          )
        )}
        {d && d.unresolved.length > 0 && (
          <p className="sub">
            {/* Named rather than folded into a guessed tier or dropped: folding
                attributes one tier's waste to another, and dropping makes the
                platform look less wasteful the more configurations it retires. */}
            Not attributed to a tier — the platform no longer ships these profiles:{' '}
            {d.unresolved.map((u) => `${u.profile} (${u.runs})`).join(', ')}
          </p>
        )}
        <p className="sub">
          A barren run spent a trial and produced no knowledge unit. Over the last {d?.days ?? 30} days.
        </p>
      </PanelBody>
    </Panel>
  )
}

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
            is exposed by an endpoint yet, so the desk shows the totals that are. What the runs
            *produced* is below, from `/api/lifecycle/barren`.
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

      <BarrenPanel />
    </div>
  )
}

// ── Monitor (§11 — the UI of COMP-007) ───────────────────────────────────────

/**
 * What the kill-switch badge may claim, given what answered.
 *
 * `tripped` and `armed` are both claims about live trading, and the panel used
 * to derive them from `kill.data?.kill_switch_active === true` — so an endpoint
 * that did not answer rendered a green **armed**. That is the platform's one
 * non-negotiable pointed at the control that stops money moving: an unknown
 * state shown as the safe one. Absent is a third word here, not a default to
 * either of the other two.
 */
export function killSwitchBadge(
  status: { kill_switch_active?: boolean } | null | undefined,
): { word: string; tone: 'pos' | 'neg' | 'warn' } {
  if (!status) return { word: 'state unknown', tone: 'warn' }
  return status.kill_switch_active === true
    ? { word: 'tripped', tone: 'neg' }
    : { word: 'armed', tone: 'pos' }
}

/**
 * One row of `/api/monitor/strategies`, read under the names it is served with.
 *
 * The table read `name`, `status` and `breaching_metric`; the route sends
 * `strategy_id`, `state` and `scale_pct`. Every row therefore rendered as two
 * blanks and a dash — a monitor showing a table of empty rows for live
 * strategies, which looks like a monitor with nothing to report. The same class
 * of defect as `cited` against `cited_by_chain`: no error, no clue, and the
 * wrong reading is the reassuring one.
 */
export interface MonitorRow {
  strategy_id: string
  state: string | null
  /**
   * `null` when the row carries no scale.
   *
   * Never folded into `0`: a strategy scaled to 0% has been shut down by the
   * monitor, and one whose scale was not recorded has not. Those are opposite
   * facts and they were one `?? 0` apart.
   */
  scale_pct: number | null
}

export function monitorRow(raw: Record<string, unknown>): MonitorRow {
  const scale = raw.scale_pct
  return {
    strategy_id:
      typeof raw.strategy_id === 'string' ? raw.strategy_id : String(raw.strategy_id ?? 'unnamed'),
    state: typeof raw.state === 'string' ? raw.state : null,
    scale_pct: typeof scale === 'number' && Number.isFinite(scale) ? scale : null,
  }
}

export function Monitor() {
  const [halting, setHalting] = useState(false)
  const strategies = useQuery({ queryKey: ['desk', 'monitor'], queryFn: () => deskApi.monitor() })
  const kill = useQuery({
    queryKey: ['desk', 'kill-switch'],
    queryFn: () => deskApi.killSwitch(),
    refetchInterval: 3_000,
  })
  const scopes = useQuery({
    queryKey: ['desk', 'monitor', 'kill-switch'],
    queryFn: () => deskApi.monitorKillSwitch(),
    refetchInterval: 3_000,
  })
  const tripped = kill.data?.kill_switch_active === true
  const badge = killSwitchBadge(kill.data)
  const windows = scopes.data?.maintenance_windows ?? []

  return (
    <div className="desk-scroll" style={{ display: 'flex', flexDirection: 'column', gap: 'var(--s-3)' }}>
      <Panel>
        <PanelHeader
          title="Kill switch"
          badges={
            <Badge tone={badge.tone} data-testid="kill-switch-badge">
              {badge.word}
            </Badge>
          }
        />
        <PanelBody>
          <p className="sub">
            {/* `scope` is not on `/api/trading/status` and never has been, so
                the old `?? 'global'` printed a word the endpoint never sent —
                and "global" is the widest possible claim to invent. The halted
                scopes below come from the route that does know. */}
            scope {kill.data?.scope ?? 'not recorded on this endpoint'} · tripped by{' '}
            {kill.data?.tripped_by ?? 'not recorded'} · {kill.data?.reason ?? 'no reason recorded'}
          </p>
          <p className="sub" data-testid="halted-scopes">
            {scopes.data === null || scopes.data === undefined
              ? 'halted scopes unknown — /api/monitor/kill-switch did not answer'
              : scopes.data.halted_scopes.length === 0
                ? 'no scope is halted · orders ' +
                  (scopes.data.orders_permitted ? 'permitted' : 'not permitted') +
                  ' · lease ' +
                  scopes.data.lease
                : `halted: ${scopes.data.halted_scopes.join(', ')} · lease ${scopes.data.lease}`}
          </p>
          {/* §4.3: an open maintenance window is a hole in the limits, and it is
              as visible here as a trip. The desk showed it nowhere. */}
          {windows.length > 0 && (
            <p className="sub" data-testid="maintenance-windows">
              {windows.length} open maintenance{' '}
              {windows.length === 1 ? 'window' : 'windows'} — limits are not enforced:{' '}
              {windows
                .map((w) => `${w.reason} (${w.opened_by}, until ${w.expires_at})`)
                .join('; ')}
            </p>
          )}
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
              message="The monitor did not answer"
              detail="/api/monitor/strategies is served (COMP-007), so this is the request failing rather than the phase being unbuilt. The live strategies are unknown, not none."
            />
          ) : strategies.data.strategies.length === 0 ? (
            <EmptyState
              message="No strategy is under the monitor"
              detail="The endpoint answered with an empty list, which is a fact about the monitor rather than about this page."
            />
          ) : (
            <table className="tbl">
              <thead>
                <tr>
                  <th style={{ textAlign: 'left' }}>strategy</th>
                  <th>state</th>
                  <th>scale</th>
                  <th />
                </tr>
              </thead>
              <tbody>
                {strategies.data.strategies.map((s) => {
                  const row = monitorRow(s as Record<string, unknown>)
                  return (
                    <tr key={row.strategy_id}>
                      <td style={{ textAlign: 'left' }}>{row.strategy_id}</td>
                      <td>{row.state ?? 'not reported'}</td>
                      {/* 0% is "the monitor shut this down"; an absent scale is
                          "nobody said". They must not look the same. */}
                      <td>{row.scale_pct === null ? 'not reported' : `${row.scale_pct}%`}</td>
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
          <p className="sub">
            {/* The column that was here read `breaching_metric`, which the route
                does not send. An always-blank column is worse than a missing
                one: it reads as "nothing is breaching". */}
            Which limit a strategy is breaching is not on this route. `/api/monitor/events` carries
            the breaches and is not joined to these rows yet, so this table says what each strategy
            is, not what it did wrong.
          </p>
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
