// The live half of the Training Console (Set H H-5.5).
//
// What is real and what is not, stated rather than drawn over:
//   * the trainer reports `{epoch, train_loss, val_loss}` on each progress frame
//     and the registry now keeps them (`mlops.training_metric`), so the loss
//     curve is the recorded series. Nothing consumes the `models.jobs`
//     WebSocket lane, so this console POLLS (see useModelRuns, useTrainingCurve);
//     the progress curve is still PROGRESS OVER TIME as this browser saw it;
//   * the previous-best overlay compares the finished run's metrics against the
//     best earlier version's, metric by metric.
import { useMemo } from 'react'
import { WifiOff } from 'lucide-react'
import {
  compareToBest,
  curveSeries,
  etaSeconds,
  formatDuration,
  formatMetric,
  metricLabel,
  type ProgressSample,
} from './studio.helpers'
import type { CurvePoint, ModelRunSnapshot, ModelVersion } from '@/api/mlops'

export function ProgressCurve({ samples }: { samples: ProgressSample[] }) {
  if (samples.length < 2) {
    return (
      <p className="mut" style={{ fontSize: 'var(--t-11)' }}>
        Progress samples appear here as the run reports them.
      </p>
    )
  }
  const w = 320
  const h = 64
  const t0 = samples[0].t
  const span = Math.max(1, samples[samples.length - 1].t - t0)
  const d = samples
    .map((s, i) => `${i === 0 ? 'M' : 'L'}${(((s.t - t0) / span) * (w - 4) + 2).toFixed(1)} ${(h - 2 - (s.progress / 100) * (h - 4)).toFixed(1)}`)
    .join(' ')
  return (
    <svg width="100%" viewBox={`0 0 ${w} ${h}`} role="img" aria-label="Observed run progress over time" preserveAspectRatio="none">
      <line x1={0} x2={w} y1={h - 2} y2={h - 2} stroke="var(--line-hairline)" />
      <path d={d} fill="none" stroke="var(--bg-accent)" strokeWidth={1.75} vectorEffect="non-scaling-stroke" />
    </svg>
  )
}

/**
 * The recorded loss curve: train and validation loss per epoch, from
 * `GET /api/models/{id}/runs/{run_id}/curve`. A run with no validation split
 * draws one line and says so; it is never drawn with a validation line at zero.
 */
export function LossCurve({ points, failed }: { points: CurvePoint[] | undefined; failed?: boolean }) {
  const { train, val, validated } = useMemo(() => curveSeries(points), [points])
  if (failed) {
    return (
      <p className="mut" style={{ fontSize: 'var(--t-11)' }} data-testid="loss-curve-unread">
        The loss curve could not be read; the progress curve below is what this browser observed.
      </p>
    )
  }
  if (train.length < 1) {
    return (
      <p className="mut" style={{ fontSize: 'var(--t-11)' }} data-testid="loss-curve-empty">
        No epoch has reported a loss yet. This model&apos;s trainer may report only progress.
      </p>
    )
  }
  const all = [...train, ...val]
  const xs = all.map((p) => p[0])
  const ys = all.map((p) => p[1])
  const [x0, x1] = [Math.min(...xs), Math.max(...xs)]
  const [y0, y1] = [Math.min(...ys), Math.max(...ys)]
  const w = 320
  const h = 96
  const px = (x: number) => (x1 === x0 ? w / 2 : ((x - x0) / (x1 - x0)) * (w - 8) + 4)
  const py = (y: number) => (y1 === y0 ? h / 2 : h - 4 - ((y - y0) / (y1 - y0)) * (h - 8))
  const path = (s: Array<[number, number]>) =>
    s.map(([x, y], i) => `${i === 0 ? 'M' : 'L'}${px(x).toFixed(1)} ${py(y).toFixed(1)}`).join(' ')
  const last = train[train.length - 1]
  return (
    <div data-testid="loss-curve">
      <svg width="100%" viewBox={`0 0 ${w} ${h}`} role="img" aria-label="Training and validation loss per epoch" preserveAspectRatio="none">
        <line x1={0} x2={w} y1={h - 2} y2={h - 2} stroke="var(--line-hairline)" />
        <path d={path(train)} fill="none" stroke="var(--bg-accent)" strokeWidth={1.75} vectorEffect="non-scaling-stroke" />
        {val.length > 0 && (
          <path d={path(val)} fill="none" stroke="var(--fg-warn)" strokeWidth={1.75} strokeDasharray="4 3" vectorEffect="non-scaling-stroke" />
        )}
      </svg>
      <p className="mut num" style={{ fontSize: 'var(--t-11)' }}>
        epoch {last[0]} · train {last[1].toPrecision(4)}
        {validated > 0 ? ` · validation ${val[val.length - 1][1].toPrecision(4)} (dashed)` : ' · no validation loss reported'}
      </p>
    </div>
  )
}

export function RunEta({ samples, run }: { samples: ProgressSample[]; run: ModelRunSnapshot }) {
  if (run.status !== 'running') return null
  const eta = etaSeconds(samples)
  return (
    <span className="mut" style={{ fontSize: 'var(--t-12)' }}>
      ETA <span className="num">{eta === null ? 'not enough data yet' : formatDuration(eta)}</span>
    </span>
  )
}

export function BestOverlay({
  run,
  versions,
}: {
  run: ModelRunSnapshot
  versions: ModelVersion[] | undefined
}) {
  const rows = useMemo(() => compareToBest(run.metrics, versions), [run.metrics, versions])
  if (run.status !== 'succeeded') {
    return (
      <p className="mut" style={{ fontSize: 'var(--t-11)' }}>
        The comparison with the best earlier version is drawn when the run finishes; metrics are not streamed during training.
      </p>
    )
  }
  if (rows.length === 0) {
    return (
      <p className="mut" style={{ fontSize: 'var(--t-11)' }}>
        No earlier version reports the same headline metrics, so there is nothing to overlay.
      </p>
    )
  }
  return (
    <table className="tbl dense" aria-label="This run against the best earlier version">
      <thead>
        <tr>
          <th className="l">Metric</th>
          <th>This run</th>
          <th>Best earlier</th>
          <th>Delta</th>
        </tr>
      </thead>
      <tbody>
        {rows.map((r) => (
          <tr key={r.key}>
            <td className="l">{metricLabel(r.key)}</td>
            <td className="n">{formatMetric(r.key, r.run)}</td>
            <td className="n">{formatMetric(r.key, r.best)} (v{r.bestVersion})</td>
            <td
              className="n"
              style={{ color: r.better === true ? 'var(--fg-pos)' : r.better === false ? 'var(--fg-neg)' : undefined }}
            >
              {r.delta > 0 ? '+' : ''}
              {r.delta.toFixed(4)}
            </td>
          </tr>
        ))}
      </tbody>
    </table>
  )
}

/** Shown while polling is failing: the numbers on screen are the last ones seen. */
export function LinkLostBanner({ onRetry }: { onRetry: () => void }) {
  return (
    <div className="banner warn" role="status" data-testid="train-link-lost">
      <WifiOff size={13} aria-hidden />
      <span>Lost contact with the registry. Progress shown is the last reading; retrying every few seconds.</span>
      <span className="spacer" />
      <button type="button" className="btn xs ghost" onClick={onRetry}>
        Retry now
      </button>
    </div>
  )
}
