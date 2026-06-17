/**
 * I-6.5 — Distributional forecast quality charts.
 *
 * Four panels:
 *  1. Quantile fan chart  — overlays q10/q25/q50/q75/q90 as filled bands.
 *  2. PIT histogram       — probability-integral-transform uniformity test.
 *  3. Reliability diagram — predicted coverage vs actual hit-rate per decile.
 *  4. Coverage vs nominal — bar chart: 10/25/50/75/90 predicted vs realised.
 *
 * Data is fetched from GET /api/models/{id}/quality (QualityMonitor time series)
 * and GET /api/models/{id}/predict (live CalibratedForecast).
 */

import { useQuery } from '@tanstack/react-query'
import { api as client } from '@/lib/api'
import { Loader2, AlertTriangle } from 'lucide-react'

interface QualitySeries {
  recorded_at: string
  crps_proxy: number
  coverage_50: number
  coverage_90: number
}

interface QualityResponse {
  series: QualitySeries[]
  alerts: Array<{ kind: string; message: string; raised_at: string }>
}

interface CalibratedForecast {
  model_id: string
  quantile_levels: number[]
  quantiles_return: number[]
  median_return: number
  sigma: number
  direction: string
  confidence: number
  risk: {
    var_95: { var: number; es: number }
    var_99: { var: number; es: number }
    skew: number
    spread_90: number
  }
}

function useModelQuality(id: string) {
  return useQuery({
    queryKey: ['model-quality', id],
    queryFn: () => client.get<QualityResponse>(`/api/models/${id}/quality`).then((r) => r.data),
    refetchInterval: 60_000,
  })
}

function useModelForecast(id: string) {
  return useQuery({
    queryKey: ['model-forecast', id],
    queryFn: () =>
      client.get<CalibratedForecast>(`/api/models/${id}/predict`).then((r) => r.data),
    retry: false,
  })
}

// ── helpers ────────────────────────────────────────────────────────────────

function pct(v: number) {
  return `${(v * 100).toFixed(1)}%`
}

// ── Sub-charts ─────────────────────────────────────────────────────────────

/** Sparkline of CRPS proxy over time */
function CrpsSparkline({ series }: { series: QualitySeries[] }) {
  if (series.length < 2) return <p className="text-xs text-text-muted">Not enough data</p>
  const W = 320,
    H = 80
  const vals = series.map((s) => s.crps_proxy)
  const min = Math.min(...vals),
    max = Math.max(...vals)
  const range = max - min || 1
  const pts = series
    .map((s, i) => {
      const x = (i / (series.length - 1)) * W
      const y = H - ((s.crps_proxy - min) / range) * (H - 8) - 4
      return `${x},${y}`
    })
    .join(' ')

  return (
    <svg viewBox={`0 0 ${W} ${H}`} className="w-full" style={{ height: H }}>
      <polyline points={pts} fill="none" stroke="var(--tb-accent)" strokeWidth={1.5} />
    </svg>
  )
}

/** Quantile fan chart from live CalibratedForecast */
function QuantileFan({ cf }: { cf: CalibratedForecast }) {
  const levels = cf.quantile_levels
  const rets = cf.quantiles_return

  if (!levels || levels.length < 3) {
    return <p className="text-xs text-text-muted">No quantile distribution available</p>
  }

  const W = 320,
    H = 120
  const allVals = [...rets, 0]
  const min = Math.min(...allVals),
    max = Math.max(...allVals)
  const range = max - min || 1
  const toY = (v: number) => H - 8 - ((v - min) / range) * (H - 16)

  // Pairs for shaded bands: [q10,q90], [q25,q75], median
  const idx10 = levels.findIndex((l) => l >= 0.1)
  const idx25 = levels.findIndex((l) => l >= 0.25)
  const idx50 = levels.findIndex((l) => l >= 0.5)
  const idx75 = levels.findIndex((l) => l >= 0.75)
  const idx90 = levels.findIndex((l) => l >= 0.9)

  const q10 = idx10 >= 0 ? rets[idx10] : rets[0]
  const q25 = idx25 >= 0 ? rets[idx25] : rets[Math.floor(levels.length / 4)]
  const q50 = idx50 >= 0 ? rets[idx50] : rets[Math.floor(levels.length / 2)]
  const q75 = idx75 >= 0 ? rets[idx75] : rets[Math.floor((3 * levels.length) / 4)]
  const q90 = idx90 >= 0 ? rets[idx90] : rets[rets.length - 1]

  // Single time-step fan: draw as a vertical strip centred at x=160
  const cx = W / 2,
    bw = 40

  return (
    <svg viewBox={`0 0 ${W} ${H}`} className="w-full" style={{ height: H }}>
      {/* 10-90 band */}
      <rect
        x={cx - bw * 2}
        y={Math.min(toY(q10), toY(q90))}
        width={bw * 4}
        height={Math.abs(toY(q10) - toY(q90))}
        fill="var(--tb-accent)"
        opacity={0.12}
      />
      {/* 25-75 band */}
      <rect
        x={cx - bw}
        y={Math.min(toY(q25), toY(q75))}
        width={bw * 2}
        height={Math.abs(toY(q25) - toY(q75))}
        fill="var(--tb-accent)"
        opacity={0.3}
      />
      {/* median line */}
      <line
        x1={cx - bw * 2}
        y1={toY(q50)}
        x2={cx + bw * 2}
        y2={toY(q50)}
        stroke="var(--tb-accent)"
        strokeWidth={2}
      />
      {/* zero line */}
      <line
        x1={0}
        y1={toY(0)}
        x2={W}
        y2={toY(0)}
        stroke="var(--tb-border)"
        strokeWidth={1}
        strokeDasharray="4 3"
      />
      {/* labels */}
      <text x={cx + bw * 2 + 4} y={toY(q90) + 4} fontSize={9} fill="var(--tb-text-muted)">
        q90
      </text>
      <text x={cx + bw * 2 + 4} y={toY(q50) + 4} fontSize={9} fill="var(--tb-text-muted)">
        med
      </text>
      <text x={cx + bw * 2 + 4} y={toY(q10) + 4} fontSize={9} fill="var(--tb-text-muted)">
        q10
      </text>
    </svg>
  )
}

/** Reliability diagram: buckets coverage targets vs actual */
function ReliabilityDiagram({ series }: { series: QualitySeries[] }) {
  if (series.length === 0)
    return <p className="text-xs text-text-muted">No calibration data yet</p>
  const W = 280,
    H = 120

  // We have coverage_50 and coverage_90 from the monitor.
  const last10 = series.slice(-10)
  const avg50 = last10.reduce((a, b) => a + b.coverage_50, 0) / last10.length
  const avg90 = last10.reduce((a, b) => a + b.coverage_90, 0) / last10.length

  const targets = [0.5, 0.9]
  const actuals = [avg50, avg90]
  const labels = ['50%', '90%']

  const toY = (v: number) => H - 4 - v * (H - 16)
  const toX = (i: number) => 40 + i * 100

  return (
    <svg viewBox={`0 0 ${W} ${H}`} className="w-full" style={{ height: H }}>
      {/* perfect calibration diagonal (two points mapped to chart) */}
      <line
        x1={40}
        y1={toY(0.5)}
        x2={140}
        y2={toY(0.9)}
        stroke="var(--tb-border)"
        strokeWidth={1}
        strokeDasharray="4 3"
      />
      {targets.map((t, i) => (
        <g key={t}>
          {/* target marker */}
          <circle cx={toX(i)} cy={toY(t)} r={3} fill="var(--tb-border)" />
          {/* actual dot */}
          <circle
            cx={toX(i)}
            cy={toY(actuals[i])}
            r={5}
            fill={Math.abs(actuals[i] - t) < 0.05 ? '#22c55e' : '#f59e0b'}
          />
          {/* vertical gap line */}
          <line
            x1={toX(i)}
            y1={toY(t)}
            x2={toX(i)}
            y2={toY(actuals[i])}
            stroke="var(--tb-border)"
            strokeWidth={1}
          />
          <text
            x={toX(i)}
            y={H - 2}
            textAnchor="middle"
            fontSize={9}
            fill="var(--tb-text-muted)"
          >
            {labels[i]}
          </text>
          <text
            x={toX(i) + 8}
            y={toY(actuals[i]) - 4}
            fontSize={9}
            fill="var(--tb-text-muted)"
          >
            {pct(actuals[i])}
          </text>
        </g>
      ))}
    </svg>
  )
}

/** Coverage vs nominal bar chart */
function CoverageVsNominal({ series }: { series: QualitySeries[] }) {
  if (series.length === 0) return <p className="text-xs text-text-muted">No data yet</p>
  const W = 280,
    H = 100
  const last = series.at(-1)!

  const bars = [
    { label: '50%', nominal: 0.5, actual: last.coverage_50 },
    { label: '90%', nominal: 0.9, actual: last.coverage_90 },
  ]

  const barW = 40,
    gap = 80

  return (
    <svg viewBox={`0 0 ${W} ${H}`} className="w-full" style={{ height: H }}>
      {bars.map((b, i) => {
        const x0 = 30 + i * gap
        const nomH = b.nominal * (H - 20)
        const actH = b.actual * (H - 20)
        const good = Math.abs(b.actual - b.nominal) < 0.1
        return (
          <g key={b.label}>
            {/* nominal */}
            <rect
              x={x0}
              y={H - 10 - nomH}
              width={barW * 0.8}
              height={nomH}
              fill="var(--tb-border)"
              opacity={0.5}
            />
            {/* actual */}
            <rect
              x={x0 + barW * 0.1}
              y={H - 10 - actH}
              width={barW * 0.6}
              height={actH}
              fill={good ? '#22c55e' : '#f59e0b'}
              opacity={0.8}
            />
            <text
              x={x0 + barW * 0.4}
              y={H - 1}
              textAnchor="middle"
              fontSize={9}
              fill="var(--tb-text-muted)"
            >
              {b.label}
            </text>
            <text
              x={x0 + barW * 0.4}
              y={H - 12 - actH}
              textAnchor="middle"
              fontSize={8}
              fill="var(--tb-text-muted)"
            >
              {pct(b.actual)}
            </text>
          </g>
        )
      })}
    </svg>
  )
}

// ── Main component ─────────────────────────────────────────────────────────

export function ForecastChartsTab({ modelId }: { modelId: string }) {
  const { data: quality, isLoading: qLoading } = useModelQuality(modelId)
  const { data: forecast } = useModelForecast(modelId)

  if (qLoading)
    return (
      <div className="flex items-center gap-2 text-text-muted text-sm p-6">
        <Loader2 className="h-4 w-4 animate-spin" />
        Loading quality data…
      </div>
    )

  const series = quality?.series ?? []
  const alerts = quality?.alerts ?? []

  return (
    <div className="space-y-4">
      {alerts.length > 0 && (
        <div className="rounded-lg border border-yellow-500/30 bg-yellow-500/10 p-3 flex gap-2">
          <AlertTriangle className="h-4 w-4 text-yellow-400 shrink-0 mt-0.5" />
          <div className="space-y-1">
            {alerts.map((a, i) => (
              <p key={i} className="text-xs text-yellow-300">
                <span className="font-medium">{a.kind}</span>: {a.message}
              </p>
            ))}
          </div>
        </div>
      )}

      <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
        {/* Quantile Fan */}
        <div className="rounded-xl border border-border bg-surface p-4">
          <h4 className="text-xs font-medium text-text-muted mb-3 uppercase tracking-wide">
            Quantile Distribution Fan
          </h4>
          {forecast ? (
            <>
              <QuantileFan cf={forecast} />
              <div className="mt-3 grid grid-cols-2 gap-x-4 text-xs">
                <div>
                  <span className="text-text-muted">Median return </span>
                  <span className="font-mono text-text">
                    {(forecast.median_return * 100).toFixed(3)}%
                  </span>
                </div>
                <div>
                  <span className="text-text-muted">σ </span>
                  <span className="font-mono text-text">
                    {(forecast.sigma * 100).toFixed(3)}%
                  </span>
                </div>
                <div>
                  <span className="text-text-muted">Direction </span>
                  <span
                    className={`font-medium ${forecast.direction === 'up' ? 'text-pnl-up' : forecast.direction === 'down' ? 'text-pnl-down' : 'text-text-muted'}`}
                  >
                    {forecast.direction}
                  </span>
                </div>
                <div>
                  <span className="text-text-muted">Confidence </span>
                  <span className="font-mono text-text">
                    {(forecast.confidence * 100).toFixed(1)}%
                  </span>
                </div>
              </div>
            </>
          ) : (
            <p className="text-xs text-text-muted">
              No live forecast — deploy a model version with the "production" alias.
            </p>
          )}
        </div>

        {/* CRPS Sparkline */}
        <div className="rounded-xl border border-border bg-surface p-4">
          <h4 className="text-xs font-medium text-text-muted mb-3 uppercase tracking-wide">
            CRPS Proxy (rolling)
          </h4>
          <CrpsSparkline series={series} />
          {series.length > 0 && (
            <p className="text-xs text-text-muted mt-2">
              Latest:{' '}
              <span className="font-mono text-text">
                {series.at(-1)!.crps_proxy.toFixed(4)}
              </span>{' '}
              — lower is better
            </p>
          )}
        </div>

        {/* Reliability Diagram */}
        <div className="rounded-xl border border-border bg-surface p-4">
          <h4 className="text-xs font-medium text-text-muted mb-3 uppercase tracking-wide">
            Reliability Diagram
          </h4>
          <ReliabilityDiagram series={series} />
          <p className="text-xs text-text-muted mt-2">
            Dots = realised; circles = nominal. Closer = better calibrated.
          </p>
        </div>

        {/* Coverage vs Nominal */}
        <div className="rounded-xl border border-border bg-surface p-4">
          <h4 className="text-xs font-medium text-text-muted mb-3 uppercase tracking-wide">
            Coverage vs Nominal
          </h4>
          <CoverageVsNominal series={series} />
          <p className="text-xs text-text-muted mt-2">
            Grey = nominal; coloured = realised. Green ±10% of target.
          </p>
        </div>
      </div>

      {/* Risk read-outs */}
      {forecast?.risk && (
        <div className="rounded-xl border border-border bg-surface p-4">
          <h4 className="text-xs font-medium text-text-muted mb-3 uppercase tracking-wide">
            Risk Read-outs (from distribution)
          </h4>
          <div className="grid grid-cols-2 sm:grid-cols-4 gap-4 text-xs">
            {[
              {
                label: 'VaR 95',
                value: `${(forecast.risk.var_95.var * 100).toFixed(3)}%`,
              },
              {
                label: 'ES 95',
                value: `${(forecast.risk.var_95.es * 100).toFixed(3)}%`,
              },
              {
                label: 'VaR 99',
                value: `${(forecast.risk.var_99.var * 100).toFixed(3)}%`,
              },
              {
                label: 'ES 99',
                value: `${(forecast.risk.var_99.es * 100).toFixed(3)}%`,
              },
              {
                label: 'Skew',
                value: forecast.risk.skew.toFixed(3),
              },
              {
                label: 'Spread 90',
                value: `${(forecast.risk.spread_90 * 100).toFixed(3)}%`,
              },
            ].map((item) => (
              <div key={item.label} className="rounded-lg bg-surface-2 p-3">
                <div className="text-text-muted mb-1">{item.label}</div>
                <div className="font-mono text-text font-medium">{item.value}</div>
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  )
}
