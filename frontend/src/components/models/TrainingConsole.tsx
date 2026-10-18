// Training console (N-5.5).
//
// Live progress comes from the routes the registry serves (`/runs`,
// `/runs/{id}/curve`, `/events`), polled: nothing consumes the `models.jobs`
// WebSocket lane (see useModelRuns). The checkpoint cadence and the
// `max_gpu_hours` budget are shown ONLY when the run's metrics report them;
// otherwise the console says "not reported". It never invents a budget.
import { useEffect, useState } from 'react'
import { Button } from '@/components/primitives/Button'
import { Badge, KeyValue } from '@/components/primitives/Badge'
import { Panel, PanelBody, PanelHeader } from '@/components/primitives/Panel'
import { EmptyState, ErrorState, PanelLoading } from '@/components/primitives/States'
import {
  useCancelRun,
  useModelEvents,
  useModelRuns,
  useModelVersions,
  useTrainingCurve,
} from '@/hooks/useMlOps'
import { LinkLostBanner, LossCurve, ProgressCurve, RunEta, BestOverlay } from '@/components/mlops/TrainLive'
import { appendSample, eventText, type ProgressSample } from '@/components/mlops/studio.helpers'
import type { ModelRunSnapshot } from '@/api/mlops'

function reported(m: Record<string, unknown> | undefined, ...keys: string[]): unknown {
  for (const k of keys) if (m && m[k] !== undefined && m[k] !== null) return m[k]
  return undefined
}

/** A reported number or string prints; anything else is "not reported". */
function reportedText(v: unknown, unit = ''): string {
  if (typeof v === 'number' && Number.isFinite(v)) return `${v}${unit}`
  if (typeof v === 'string' && v !== '') return `${v}${unit}`
  return 'not reported'
}

export function TrainingBudget({ run }: { run: ModelRunSnapshot }) {
  const m = run.metrics
  const cadence = reported(m, 'checkpoint_every_epochs', 'checkpoint_cadence', 'checkpoint_every')
  const budget = reported(m, 'max_gpu_hours')
  const used = reported(m, 'gpu_hours_used', 'gpu_hours')
  const over =
    typeof budget === 'number' && typeof used === 'number' && budget > 0 && used >= budget
  return (
    <div data-testid="training-budget">
      <KeyValue k="Checkpoint cadence" v={reportedText(cadence, typeof cadence === 'number' ? ' epochs' : '')} />
      <KeyValue k="GPU-hour budget (max_gpu_hours)" v={reportedText(budget, typeof budget === 'number' ? ' h' : '')} />
      <KeyValue k="GPU hours used" v={reportedText(used, typeof used === 'number' ? ' h' : '')} tone={over ? 'neg' : undefined} />
    </div>
  )
}

function statusTone(s: ModelRunSnapshot['status']) {
  if (s === 'succeeded') return 'pos' as const
  if (s === 'failed') return 'neg' as const
  if (s === 'running' || s === 'queued') return 'accent' as const
  return 'neutral' as const
}

export function TrainingConsole({ modelId }: { modelId: string }) {
  const runsQ = useModelRuns(modelId)
  const versions = useModelVersions(modelId)
  const events = useModelEvents(modelId)
  const cancel = useCancelRun(modelId)
  const [picked, setPicked] = useState<string | null>(null)
  const [samples, setSamples] = useState<ProgressSample[]>([])

  const training = (runsQ.data ?? []).filter((r) => r.kind === 'training')
  const active = training.find((r) => r.status === 'running' || r.status === 'queued')
  const run = training.find((r) => r.run_id === picked) ?? active ?? training[0]
  const live = run?.status === 'running'
  const curve = useTrainingCurve(modelId, run?.run_id ?? null, live)

  // The progress curve is the series of readings this browser observed; the
  // polled row carries only the latest, so there is nothing to derive it from.
  useEffect(() => {
    if (!active) return
    // eslint-disable-next-line react-hooks/set-state-in-effect -- an observed series, not derivable from the polled row
    setSamples((prev) => appendSample(prev, { t: Date.now(), progress: active.progress }))
  }, [active])

  if (runsQ.isLoading) return <PanelLoading lines={6} />
  if (runsQ.isError && !runsQ.data) {
    return <ErrorState message="Training runs could not be read." onRetry={() => void runsQ.refetch()} />
  }
  if (!run) {
    return (
      <EmptyState
        message="No training run yet"
        detail="Start one from New model; its live progress appears here."
      />
    )
  }

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--s-3)' }}>
      {runsQ.isError && <LinkLostBanner onRetry={() => void runsQ.refetch()} />}
      <div style={{ display: 'grid', gap: 'var(--s-3)', gridTemplateColumns: 'repeat(auto-fit, minmax(320px, 1fr))' }}>
        <Panel>
          <PanelHeader
            title={`Run ${run.run_id.slice(0, 8)}`}
            badges={<Badge tone={statusTone(run.status)} data-testid="run-status">{run.status}</Badge>}
            actions={
              active ? (
                <Button size="xs" variant="danger" loading={cancel.isPending} onClick={() => cancel.mutate(active.run_id)}>
                  Cancel run
                </Button>
              ) : undefined
            }
          />
          <PanelBody>
            <KeyValue k="Phase" v={run.phase} />
            <KeyValue k="Progress" v={<span data-testid="run-progress">{Math.round(run.progress)}%</span>} />
            <progress value={run.progress} max={100} aria-label="Run progress" style={{ width: '100%' }} />
            <RunEta samples={samples} run={run} />
            {run.status === 'failed' && (
              <div className="inline-err" role="alert" data-testid="run-error">
                <span className="msg">{run.error ?? 'The run failed and reported no reason.'}</span>
              </div>
            )}
            <TrainingBudget run={run} />
          </PanelBody>
        </Panel>

        <Panel>
          <PanelHeader title="Loss" />
          <PanelBody>
            <LossCurve points={curve.data} failed={curve.isError} />
            <ProgressCurve samples={samples} />
          </PanelBody>
        </Panel>
      </div>

      <Panel>
        <PanelHeader title="Against the best earlier version" />
        <PanelBody>
          <BestOverlay run={run} versions={versions.data} />
        </PanelBody>
      </Panel>

      <div style={{ display: 'grid', gap: 'var(--s-3)', gridTemplateColumns: 'repeat(auto-fit, minmax(320px, 1fr))' }}>
        <Panel>
          <PanelHeader title="Runs" />
          <PanelBody flush>
            <table className="tbl dense" aria-label="Training runs">
              <tbody>
                {training.map((r) => (
                  <tr key={r.run_id} onClick={() => setPicked(r.run_id)} style={{ cursor: 'pointer' }} data-testid="run-row">
                    <td className="mono">{r.run_id.slice(0, 8)}</td>
                    <td>{r.status}</td>
                    <td className="mut">{r.created_at.slice(0, 16).replace('T', ' ')}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </PanelBody>
        </Panel>
        <Panel>
          <PanelHeader title="Events" />
          <PanelBody>
            {events.isLoading ? (
              <PanelLoading lines={3} />
            ) : events.isError ? (
              <ErrorState message="The event log could not be read." onRetry={() => void events.refetch()} />
            ) : (events.data ?? []).length === 0 ? (
              <EmptyState message="No events" />
            ) : (
              <ul style={{ margin: 0, paddingLeft: 'var(--s-4)', fontSize: 'var(--t-12)' }}>
                {(events.data ?? []).slice(0, 12).map((e, i) => (
                  <li key={`${e.created_at}-${i}`}>
                    {eventText(e)} <span className="mut">{e.created_at.slice(0, 16).replace('T', ' ')}</span>
                  </li>
                ))}
              </ul>
            )}
          </PanelBody>
        </Panel>
      </div>
    </div>
  )
}
