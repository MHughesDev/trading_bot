import { useState } from 'react'
import { motion, AnimatePresence } from 'framer-motion'
import { Loader2, GitCompare, CheckCircle2, XCircle } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import { useModelEvals } from '@/hooks/useMlOps'
import { modelsApi } from '@/api/mlops'
import type { EvaluationRun } from '@/api/mlops'
import { format } from 'date-fns'

interface Props {
  modelId: string
}

interface CompareResult {
  a: EvaluationRun
  b: EvaluationRun
  deltas: Record<string, { a: number; b: number; delta: number }>
}

function CompareModal({
  modelId,
  evals,
  onClose,
}: {
  modelId: string
  evals: EvaluationRun[]
  onClose: () => void
}) {
  const [versionA, setVersionA] = useState<string>('')
  const [versionB, setVersionB] = useState<string>('')
  const [result, setResult] = useState<CompareResult | null>(null)
  const [loading, setLoading] = useState(false)

  const evalsByVersion = evals.reduce(
    (acc, e) => {
      acc[e.version] = e
      return acc
    },
    {} as Record<number, EvaluationRun>,
  )

  async function handleCompare() {
    if (!versionA || !versionB) return
    setLoading(true)
    try {
      const rawRes = await modelsApi.compareEvals(modelId, parseInt(versionA), parseInt(versionB))
      const raw = rawRes.data as Record<string, unknown>
      const evalA = evalsByVersion[parseInt(versionA)]
      const evalB = evalsByVersion[parseInt(versionB)]
      if (!evalA || !evalB) return

      const deltas: Record<string, { a: number; b: number; delta: number }> = {}
      const metricsA = evalA.metrics ?? {}
      const metricsB = evalB.metrics ?? {}
      const allKeys = new Set([...Object.keys(metricsA), ...Object.keys(metricsB)])
      for (const key of allKeys) {
        const a = metricsA[key] ?? 0
        const b = metricsB[key] ?? 0
        deltas[key] = { a, b, delta: b - a }
      }

      // Overlay with server-provided deltas if available
      if (raw && typeof raw === 'object') {
        const serverDeltas = raw as Record<string, unknown>
        for (const [k, v] of Object.entries(serverDeltas)) {
          if (typeof v === 'number' && deltas[k]) {
            deltas[k].delta = v
          }
        }
      }

      setResult({ a: evalA, b: evalB, deltas })
    } catch {
      // ignore
    } finally {
      setLoading(false)
    }
  }

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center">
      <div className="absolute inset-0 bg-scrim backdrop-blur-sm" onClick={onClose} />
      <motion.div
        initial={{ opacity: 0, scale: 0.95 }}
        animate={{ opacity: 1, scale: 1 }}
        exit={{ opacity: 0, scale: 0.95 }}
        className="relative z-10 w-full max-w-2xl rounded-xl border border-border bg-surface p-6 shadow-xl mx-4 max-h-[80vh] overflow-y-auto"
      >
        <h3 className="text-base font-semibold text-text mb-4">Compare Evaluations</h3>

        <div className="flex items-center gap-3 mb-5">
          <select
            value={versionA}
            onChange={(e) => setVersionA(e.target.value)}
            className="flex-1 h-9 rounded-lg border border-border bg-surface px-3 text-sm text-text focus:outline-none focus:border-accent"
          >
            <option value="">Select version A</option>
            {evals.map((e) => (
              <option key={e.eval_id} value={e.version}>
                v{e.version} — {e.status}
              </option>
            ))}
          </select>
          <GitCompare className="h-5 w-5 text-fg-tertiary shrink-0" />
          <select
            value={versionB}
            onChange={(e) => setVersionB(e.target.value)}
            className="flex-1 h-9 rounded-lg border border-border bg-surface px-3 text-sm text-text focus:outline-none focus:border-accent"
          >
            <option value="">Select version B</option>
            {evals.map((e) => (
              <option key={e.eval_id} value={e.version}>
                v{e.version} — {e.status}
              </option>
            ))}
          </select>
          <Button onClick={handleCompare} disabled={!versionA || !versionB || loading}>
            {loading ? <Loader2 className="h-4 w-4 animate-spin" /> : 'Compare'}
          </Button>
        </div>

        {result && (
          <div className="rounded-xl border border-border overflow-hidden">
            <table className="w-full">
              <thead>
                <tr className="border-b border-border bg-sunken">
                  <th className="px-4 py-3 text-left text-xs font-medium text-fg-secondary">Metric</th>
                  <th className="px-4 py-3 text-right text-xs font-medium text-fg-secondary">
                    v{result.a.version}
                  </th>
                  <th className="px-4 py-3 text-right text-xs font-medium text-fg-secondary">
                    v{result.b.version}
                  </th>
                  <th className="px-4 py-3 text-right text-xs font-medium text-fg-secondary">Delta</th>
                </tr>
              </thead>
              <tbody>
                {Object.entries(result.deltas).map(([key, { a, b, delta }]) => (
                  <tr key={key} className="border-b border-border last:border-0">
                    <td className="px-4 py-2.5 text-sm text-fg-secondary">{key}</td>
                    <td className="px-4 py-2.5 text-sm font-mono text-right text-text">
                      {a.toFixed(4)}
                    </td>
                    <td className="px-4 py-2.5 text-sm font-mono text-right text-text">
                      {b.toFixed(4)}
                    </td>
                    <td
                      className={cn(
                        'px-4 py-2.5 text-sm font-mono text-right font-medium',
                        delta > 0 ? 'text-pos' : delta < 0 ? 'text-neg' : 'text-fg-secondary',
                      )}
                    >
                      {delta > 0 ? '+' : ''}
                      {delta.toFixed(4)}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}

        <div className="mt-4 flex justify-end">
          <Button variant="outline" onClick={onClose}>
            Close
          </Button>
        </div>
      </motion.div>
    </div>
  )
}

export function ModelEvalsTab({ modelId }: Props) {
  const { data: evals, isLoading } = useModelEvals(modelId)
  const [showCompare, setShowCompare] = useState(false)

  if (isLoading) {
    return (
      <div className="flex items-center justify-center py-12 text-fg-tertiary">
        <Loader2 className="h-5 w-5 animate-spin" />
      </div>
    )
  }

  if (!evals || evals.length === 0) {
    return (
      <div className="flex flex-col items-center justify-center py-16 text-center">
        <p className="text-sm font-medium text-text">No evaluations yet</p>
        <p className="text-xs text-fg-secondary mt-1">
          Run an evaluation from the Versions tab.
        </p>
      </div>
    )
  }

  return (
    <>
      <div className="flex items-center justify-between mb-4">
        <h3 className="text-sm font-medium text-text">Evaluation runs</h3>
        {evals.length >= 2 && (
          <Button size="sm" variant="outline" onClick={() => setShowCompare(true)}>
            <GitCompare className="h-3.5 w-3.5" />
            Compare versions
          </Button>
        )}
      </div>

      <div className="rounded-xl border border-border bg-surface overflow-hidden">
        <table className="w-full">
          <thead>
            <tr className="border-b border-border bg-sunken">
              <th className="px-4 py-3 text-left text-xs font-medium text-fg-secondary">Version</th>
              <th className="px-4 py-3 text-left text-xs font-medium text-fg-secondary">Dataset</th>
              <th className="px-4 py-3 text-left text-xs font-medium text-fg-secondary">Status</th>
              <th className="px-4 py-3 text-left text-xs font-medium text-fg-secondary">Primary metric</th>
              <th className="px-4 py-3 text-left text-xs font-medium text-fg-secondary">Regression</th>
              <th className="px-4 py-3 text-left text-xs font-medium text-fg-secondary">Date</th>
            </tr>
          </thead>
          <tbody>
            {evals.map((ev) => {
              const primaryMetric = ev.metrics
                ? Object.entries(ev.metrics)[0]
                : null

              return (
                <tr key={ev.eval_id} className="border-b border-border last:border-0">
                  <td className="px-4 py-3 text-sm font-mono font-medium text-text">
                    v{ev.version}
                  </td>
                  <td className="px-4 py-3 text-xs text-fg-secondary font-mono">
                    {ev.dataset_id ?? '—'}
                  </td>
                  <td className="px-4 py-3">
                    <span
                      className={cn(
                        'inline-flex items-center rounded-full px-2 py-0.5 text-xs font-medium',
                        ev.status === 'completed'
                          ? 'bg-pos/10 text-pos'
                          : ev.status === 'failed'
                            ? 'bg-neg/10 text-neg'
                            : 'bg-sunken text-fg-secondary',
                      )}
                    >
                      {ev.status}
                    </span>
                  </td>
                  <td className="px-4 py-3 text-sm font-mono text-text">
                    {primaryMetric ? (
                      <>
                        <span className="text-fg-secondary text-xs">{primaryMetric[0]}: </span>
                        {primaryMetric[1].toFixed(4)}
                      </>
                    ) : (
                      <span className="text-fg-tertiary">—</span>
                    )}
                  </td>
                  <td className="px-4 py-3">
                    {ev.regression_ok === true ? (
                      <CheckCircle2 className="h-4 w-4 text-pos" />
                    ) : ev.regression_ok === false ? (
                      <XCircle className="h-4 w-4 text-neg" />
                    ) : (
                      <span className="text-fg-tertiary text-xs">—</span>
                    )}
                  </td>
                  <td className="px-4 py-3 text-xs text-fg-tertiary font-mono">
                    {format(new Date(ev.created_at), 'MMM d, HH:mm')}
                  </td>
                </tr>
              )
            })}
          </tbody>
        </table>
      </div>

      <AnimatePresence>
        {showCompare && (
          <CompareModal
            modelId={modelId}
            evals={evals}
            onClose={() => setShowCompare(false)}
          />
        )}
      </AnimatePresence>
    </>
  )
}
