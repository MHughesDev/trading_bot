// Evaluations Arena (Set H H-5.7): "is this version better than production?"
//
// The list route returns run snapshots with no `version`, so each succeeded
// snapshot is read once by id for its persisted row (useEvalDetails). Evaluations
// still running are listed as running, with no version claimed for them.
import { useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { Trophy } from 'lucide-react'
import { Button } from '@/components/primitives/Button'
import { Badge, type BadgeTone } from '@/components/primitives/Badge'
import { Panel, PanelBody, PanelHeader } from '@/components/primitives/Panel'
import { EmptyState, ErrorState, PanelLoading } from '@/components/primitives/States'
import { useModelAliases, useModelEvals, useEvalDetails } from '@/hooks/useMlOps'
import { modelsApi } from '@/api/mlops'
import {
  backtestMetrics,
  confusionMatrix,
  formatMetric,
  latestSucceededByVersion,
  metricDeltas,
  metricLabel,
  parseCompare,
  pinProduction,
  toEvalRow,
  trustedWinner,
  type EvalRow,
} from './studio.helpers'
import { format } from 'date-fns'

interface Props {
  modelId: string
}

function verdictTone(v: string | null): BadgeTone {
  if (v === 'regressed') return 'neg'
  if (v === 'ok') return 'pos'
  return 'neutral'
}

function verdictText(v: string | null): string {
  if (v === null) return 'no report'
  if (v === 'no_baseline') return 'no baseline'
  if (v === 'no_data') return 'no comparable metric'
  return v
}

export function ConfusionGrid({ matrix }: { matrix: number[][] }) {
  const max = Math.max(1, ...matrix.flat())
  return (
    <table className="tbl dense" aria-label="Confusion matrix">
      <thead>
        <tr>
          <th className="l">actual \ predicted</th>
          {matrix.map((_, j) => (
            <th key={j}>{j}</th>
          ))}
        </tr>
      </thead>
      <tbody>
        {matrix.map((row, i) => (
          <tr key={i}>
            <td className="l">{i}</td>
            {row.map((c, j) => (
              <td
                key={j}
                className="n"
                style={{
                  background: `color-mix(in srgb, var(${i === j ? '--bg-accent' : '--fg-neg'}) ${Math.round((c / max) * 40)}%, transparent)`,
                }}
              >
                {c}
              </td>
            ))}
          </tr>
        ))}
      </tbody>
    </table>
  )
}

function ComparePanel({
  modelId,
  a,
  b,
  rows,
}: {
  modelId: string
  a: number
  b: number
  rows: Map<number, EvalRow>
}) {
  const q = useQuery({
    queryKey: ['models', modelId, 'compare', a, b],
    queryFn: () => modelsApi.compareEvals(modelId, a, b).then((r) => parseCompare(r.data)),
  })

  if (q.isLoading) return <PanelLoading lines={4} />
  if (q.isError || !q.data) {
    return (
      <ErrorState
        message="The comparison could not be loaded."
        detail="Both versions need a completed evaluation."
        onRetry={() => void q.refetch()}
      />
    )
  }
  const c = q.data
  const winner = trustedWinner(c)
  const deltas = metricDeltas(c.a, c.b)
  const rowB = rows.get(c.b.version)
  const rowA = rows.get(c.a.version)
  const matrixB = confusionMatrix(rowB?.rawMetrics)
  const matrixA = confusionMatrix(rowA?.rawMetrics)
  const pnlA = backtestMetrics(rowA?.rawMetrics)
  const pnlB = backtestMetrics(rowB?.rawMetrics)

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--s-4)' }}>
      <div style={{ display: 'flex', gap: 'var(--s-3)', alignItems: 'center', flexWrap: 'wrap' }}>
        {winner.version !== null ? (
          <Badge tone="pos" data-testid="winner-badge">
            <Trophy size={11} aria-hidden /> v{winner.version} wins
          </Badge>
        ) : (
          <Badge tone="neutral" data-testid="winner-badge">No winner</Badge>
        )}
        <span className="mut">{winner.reason}</span>
      </div>

      <table className="tbl" aria-label="Metric comparison">
        <thead>
          <tr>
            <th className="l">Metric</th>
            <th>v{c.a.version}</th>
            <th>v{c.b.version}</th>
            <th>Delta (v{c.b.version} − v{c.a.version})</th>
          </tr>
        </thead>
        <tbody>
          {c.a.scorecard && c.b.scorecard && (
            <tr>
              <td className="l">overall score</td>
              <td className="n">{c.a.scorecard.overall.toFixed(2)}</td>
              <td className="n">{c.b.scorecard.overall.toFixed(2)}</td>
              <td className="n">{(c.b.scorecard.overall - c.a.scorecard.overall).toFixed(2)}</td>
            </tr>
          )}
          {deltas.map((d) => (
            <tr key={d.key}>
              <td className="l">{d.key}</td>
              <td className="n">{d.a === null ? '—' : d.a.toFixed(4)}</td>
              <td className="n">{d.b === null ? '—' : d.b.toFixed(4)}</td>
              <td
                className="n"
                style={{
                  color: d.better === 'b' ? 'var(--fg-pos)' : d.better === 'a' ? 'var(--fg-neg)' : undefined,
                }}
              >
                {d.delta === null ? 'not comparable' : `${d.delta > 0 ? '+' : ''}${d.delta.toFixed(4)}`}
              </td>
            </tr>
          ))}
        </tbody>
      </table>

      {rowB && rowB.checks.length > 0 && (
        <div>
          <div className="lbl">
            Regression report for v{rowB.version}
            {rowB.baselineVersion !== null && <> against production v{rowB.baselineVersion}</>}
          </div>
          <table className="tbl dense" aria-label="Regression checks">
            <tbody>
              {rowB.checks.map((k) => (
                <tr key={k.metric}>
                  <td className="l">{k.metric}</td>
                  <td className="n">{k.delta_pct != null ? `${k.delta_pct.toFixed(2)}%` : '—'}</td>
                  <td><Badge tone={k.verdict === 'regressed' ? 'neg' : k.verdict === 'improved' ? 'pos' : 'neutral'}>{k.verdict}</Badge></td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}

      {(matrixA || matrixB) && (
        <div style={{ display: 'flex', gap: 'var(--s-4)', flexWrap: 'wrap' }}>
          {matrixA && (
            <div>
              <div className="lbl">v{c.a.version} confusion matrix</div>
              <ConfusionGrid matrix={matrixA} />
            </div>
          )}
          {matrixB && (
            <div>
              <div className="lbl">v{c.b.version} confusion matrix</div>
              <ConfusionGrid matrix={matrixB} />
            </div>
          )}
        </div>
      )}

      {(pnlA.length > 0 || pnlB.length > 0) && (
        <div>
          <div className="lbl">Attached backtest</div>
          <table className="tbl dense" aria-label="Backtest metrics">
            <tbody>
              {[...new Set([...pnlA, ...pnlB].map(([k]) => k))].map((k) => (
                <tr key={k}>
                  <td className="l">{k}</td>
                  <td className="n">{pnlA.find(([x]) => x === k)?.[1].toFixed(4) ?? '—'}</td>
                  <td className="n">{pnlB.find(([x]) => x === k)?.[1].toFixed(4) ?? '—'}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
      {!matrixA && !matrixB && pnlA.length === 0 && pnlB.length === 0 && (
        <p className="mut" style={{ fontSize: 'var(--t-11)' }}>
          These evaluations did not record a confusion matrix or an attached backtest, so none is drawn.
          Forecast-versus-actual is on the Forecast Quality tab.
        </p>
      )}
    </div>
  )
}

export function ModelEvalsTab({ modelId }: Props) {
  const snaps = useModelEvals(modelId)
  const aliases = useModelAliases(modelId)
  const details = useEvalDetails(modelId, snaps.data)
  const [pair, setPair] = useState<{ a: string; b: string }>({ a: '', b: '' })

  if (snaps.isLoading) return <PanelLoading lines={5} />
  if (snaps.isError) {
    return (
      <ErrorState
        message="Evaluations could not be loaded."
        detail="This is a failed request, not a model that was never evaluated."
        onRetry={() => void snaps.refetch()}
      />
    )
  }
  const snapshots = snaps.data ?? []
  if (snapshots.length === 0) {
    return (
      <EmptyState
        message="No evaluations yet"
        detail="Run an evaluation from the Versions tab. This list holds the evaluations since the server last started."
      />
    )
  }

  const prod = typeof aliases.data?.production === 'number' ? aliases.data.production : null
  const rows = pinProduction(
    details.flatMap((d) => (d.data ? [toEvalRow(d.data)] : [])),
    prod,
  )
  const running = snapshots.filter((s) => s.status === 'running' || s.status === 'queued')
  const failed = snapshots.filter((s) => s.status === 'failed')
  const byVersion = latestSucceededByVersion(rows)
  const comparable = [...byVersion.keys()].sort((x, y) => y - x)
  const canCompare = pair.a !== '' && pair.b !== '' && pair.a !== pair.b

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--s-4)' }}>
      <Panel>
        <PanelHeader title="Evaluations" />
        <PanelBody flush>
          <table className="tbl" aria-label="Evaluations">
            <thead>
              <tr>
                <th className="l">Version</th>
                <th>Status</th>
                <th>Primary metric</th>
                <th>Overall</th>
                <th>Regression vs production</th>
                <th>Date</th>
              </tr>
            </thead>
            <tbody>
              {running.map((s) => (
                <tr key={s.run_id}>
                  <td className="l">evaluating…</td>
                  <td><Badge tone="warn">{s.status}</Badge></td>
                  <td className="n">—</td>
                  <td className="n">{Math.round(s.progress)}% · {s.phase}</td>
                  <td>—</td>
                  <td className="n">{format(new Date(s.created_at), 'MMM d, HH:mm')}</td>
                </tr>
              ))}
              {rows.map((r) => (
                <tr key={r.evalId} data-production={r.version === prod ? 'yes' : 'no'}>
                  <td className="l num">
                    v{r.version} {r.version === prod && <Badge tone="pos">production</Badge>}
                  </td>
                  <td><Badge tone={r.status === 'succeeded' ? 'pos' : 'neutral'}>{r.status}</Badge></td>
                  <td className="n">
                    {r.primary ? `${metricLabel(r.primary.key)} ${formatMetric(r.primary.key, r.primary.value)}` : '—'}
                  </td>
                  <td className="n">{r.scorecard ? r.scorecard.overall.toFixed(2) : '—'}</td>
                  <td><Badge tone={verdictTone(r.verdict)}>{verdictText(r.verdict)}</Badge></td>
                  <td className="n">{format(new Date(r.createdAt), 'MMM d, HH:mm')}</td>
                </tr>
              ))}
              {failed.map((s) => (
                <tr key={s.run_id}>
                  <td className="l">—</td>
                  <td><Badge tone="neg">failed</Badge></td>
                  <td colSpan={3} className="l mut">{s.error ?? 'no error recorded'}</td>
                  <td className="n">{format(new Date(s.created_at), 'MMM d, HH:mm')}</td>
                </tr>
              ))}
            </tbody>
          </table>
          {details.some((d) => d.isLoading) && <PanelLoading lines={1} />}
          {details.some((d) => d.isError) && (
            <p className="mut" style={{ padding: 'var(--s-3)', color: 'var(--fg-neg)' }} role="alert">
              Some evaluations could not be read, so their rows are missing — they were not dropped.
            </p>
          )}
        </PanelBody>
      </Panel>

      <Panel>
        <PanelHeader title="Compare two versions" />
        <PanelBody>
          {comparable.length < 2 ? (
            <p className="mut">
              Comparing needs two versions with a completed evaluation; {comparable.length === 0 ? 'none has' : 'one has'} one.
            </p>
          ) : (
            <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--s-4)' }}>
              <div style={{ display: 'flex', gap: 'var(--s-3)', alignItems: 'center', flexWrap: 'wrap' }}>
                <select aria-label="Version A" value={pair.a} onChange={(e) => setPair((p) => ({ ...p, a: e.target.value }))}>
                  <option value="">Version A</option>
                  {comparable.map((v) => (
                    <option key={v} value={v}>v{v}{v === prod ? ' (production)' : ''}</option>
                  ))}
                </select>
                <select aria-label="Version B" value={pair.b} onChange={(e) => setPair((p) => ({ ...p, b: e.target.value }))}>
                  <option value="">Version B</option>
                  {comparable.map((v) => (
                    <option key={v} value={v}>v{v}{v === prod ? ' (production)' : ''}</option>
                  ))}
                </select>
                {prod !== null && comparable.length > 1 && (
                  <Button
                    size="sm"
                    onClick={() => {
                      const other = comparable.find((v) => v !== prod)
                      if (other !== undefined) setPair({ a: String(prod), b: String(other) })
                    }}
                  >
                    Production vs latest
                  </Button>
                )}
              </div>
              {canCompare ? (
                <ComparePanel modelId={modelId} a={Number(pair.a)} b={Number(pair.b)} rows={byVersion} />
              ) : (
                <p className="mut">Pick two different versions.</p>
              )}
            </div>
          )}
        </PanelBody>
      </Panel>
    </div>
  )
}
