import { useState } from 'react'
import { motion, AnimatePresence } from 'framer-motion'
import { CheckCircle2, Circle, AlertCircle, Loader2, ChevronDown, ChevronUp } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import {
  useModelVersions,
  useModelAliases,
  usePromoteVersion,
  useEvaluateVersion,
} from '@/hooks/useMlOps'
import { ModelStatusPill } from './ModelStatusPill'
import type { ModelVersion } from '@/api/mlops'
import { format } from 'date-fns'

interface Props {
  modelId: string
}

// Headline quality metrics shown first; everything else (n_train, …) follows.
const METRIC_PRIORITY = [
  'val_auc',
  'accuracy',
  'val_accuracy',
  'val_logloss',
  'rmse',
  'mae',
  'val_rmse',
  'val_mae',
  'val_r2',
  'val_directional_accuracy',
  'test_auc',
  'test_accuracy',
  'test_rmse',
  'test_mae',
]

function PromoteModal({
  version,
  modelId,
  hasProductionAlias,
  onClose,
}: {
  version: ModelVersion
  modelId: string
  hasProductionAlias: boolean
  onClose: () => void
}) {
  const [reason, setReason] = useState('')
  const promoteMut = usePromoteVersion(modelId)

  const checks = [
    {
      label: 'Has passed evaluation',
      ok: version.status === 'candidate',
    },
    {
      label: 'Has artifact hash',
      ok: !!version.artifact_hash,
    },
    {
      label: 'Rollback available (prior production exists)',
      ok: hasProductionAlias,
    },
  ]
  const allPass = checks.every((c) => c.ok)

  async function handlePromote() {
    await promoteMut.mutateAsync({ version: version.version, reason: reason.trim() || 'Promoted via UI' })
    onClose()
  }

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center">
      <div
        className="absolute inset-0 bg-black/50 backdrop-blur-sm"
        onClick={onClose}
      />
      <motion.div
        initial={{ opacity: 0, scale: 0.95 }}
        animate={{ opacity: 1, scale: 1 }}
        exit={{ opacity: 0, scale: 0.95 }}
        className="relative z-10 w-full max-w-md rounded-xl border border-border bg-surface p-6 shadow-xl mx-4"
      >
        <h3 className="text-base font-semibold text-text mb-1">
          Promote v{version.version} to Production
        </h3>
        <p className="text-sm text-text-muted mb-5">
          This will set the production alias to version {version.version}.
        </p>

        {/* Checklist */}
        <div className="space-y-2 mb-5">
          {checks.map((check) => (
            <div key={check.label} className="flex items-center gap-2.5">
              {check.ok ? (
                <CheckCircle2 className="h-4 w-4 text-pnl-up shrink-0" />
              ) : (
                <Circle className="h-4 w-4 text-text-dim shrink-0" />
              )}
              <span
                className={cn(
                  'text-sm',
                  check.ok ? 'text-text' : 'text-text-muted line-through',
                )}
              >
                {check.label}
              </span>
            </div>
          ))}
        </div>

        {!allPass && (
          <div className="flex items-start gap-2 rounded-lg bg-warning/10 border border-warning/20 px-3 py-2.5 mb-4">
            <AlertCircle className="h-4 w-4 text-warning shrink-0 mt-0.5" />
            <p className="text-xs text-warning">
              Some checks failed. Provide a reason to override.
            </p>
          </div>
        )}

        <div className="mb-5">
          <label className="block text-sm font-medium text-text mb-1.5">
            Reason {!allPass && <span className="text-pnl-down">*</span>}
          </label>
          <input
            type="text"
            value={reason}
            onChange={(e) => setReason(e.target.value)}
            placeholder="Why is this version being promoted?"
            className="w-full h-9 rounded-lg border border-border bg-surface px-3 text-sm text-text placeholder:text-text-dim focus:outline-none focus:border-accent"
          />
        </div>

        <div className="flex gap-3 justify-end">
          <Button variant="outline" onClick={onClose}>
            Cancel
          </Button>
          <Button
            onClick={handlePromote}
            disabled={(!allPass && !reason.trim()) || promoteMut.isPending}
          >
            {promoteMut.isPending ? (
              <Loader2 className="h-4 w-4 animate-spin" />
            ) : (
              'Promote'
            )}
          </Button>
        </div>
      </motion.div>
    </div>
  )
}

function VersionRow({
  version,
  modelId,
  productionVersion,
}: {
  version: ModelVersion
  modelId: string
  productionVersion?: number
}) {
  const [expanded, setExpanded] = useState(false)
  const [showPromote, setShowPromote] = useState(false)
  const evaluateMut = useEvaluateVersion(modelId)

  const isProduction = version.version === productionVersion
  // Metrics carry mixed types now (objective string, feature_importance object,
  // best_iteration null, …). Only numeric ones are displayable; surface the
  // headline quality metrics first.
  const metricEntries: Array<[string, number]> = (
    version.metrics ? Object.entries(version.metrics) : []
  )
    .filter(
      (e): e is [string, number] =>
        typeof e[1] === 'number' && Number.isFinite(e[1]),
    )
    .sort((a, b) => {
      const rank = (k: string) => {
        const i = METRIC_PRIORITY.indexOf(k)
        return i === -1 ? METRIC_PRIORITY.length : i
      }
      return rank(a[0]) - rank(b[0])
    })

  return (
    <>
      <div
        className={cn(
          'border border-border rounded-xl bg-surface p-4 transition-shadow',
          isProduction && 'border-pnl-up/40 bg-pnl-up/5',
        )}
      >
        <div className="flex items-center gap-3">
          {/* Version badge */}
          <div className="flex h-8 w-8 items-center justify-center rounded-lg bg-surface-2 font-mono text-sm font-bold text-text shrink-0">
            v{version.version}
          </div>

          <div className="flex-1 min-w-0">
            <div className="flex items-center gap-2 flex-wrap">
              <ModelStatusPill status={version.status} />
              {isProduction && (
                <span className="inline-flex items-center rounded-full bg-pnl-up/15 px-2 py-0.5 text-xs font-medium text-pnl-up border border-pnl-up/30">
                  production
                </span>
              )}
            </div>
            {version.version_note && (
              <p className="text-xs text-text-muted mt-1 truncate">{version.version_note}</p>
            )}
          </div>

          <div className="text-xs text-text-dim font-mono shrink-0">
            {format(new Date(version.created_at), 'MMM d, HH:mm')}
          </div>

          {/* Quick metrics */}
          {metricEntries.length > 0 && (
            <div className="flex gap-2 shrink-0">
              {metricEntries.slice(0, 2).map(([k, v]) => (
                <span key={k} className="text-xs font-mono text-text-muted">
                  {k}: <span className="text-text">{v.toFixed(3)}</span>
                </span>
              ))}
            </div>
          )}

          {/* Actions */}
          <div className="flex items-center gap-2 shrink-0">
            {version.status === 'candidate' && (
              <Button
                size="sm"
                variant="outline"
                className="text-xs h-7"
                onClick={() => setShowPromote(true)}
              >
                Promote
              </Button>
            )}
            {(version.status === 'draft' || version.status === 'active') && (
              <Button
                size="sm"
                variant="outline"
                className="text-xs h-7"
                onClick={() => evaluateMut.mutate(version.version)}
                disabled={evaluateMut.isPending}
              >
                {evaluateMut.isPending ? (
                  <Loader2 className="h-3 w-3 animate-spin" />
                ) : (
                  'Evaluate'
                )}
              </Button>
            )}
            {metricEntries.length > 2 && (
              <button
                onClick={() => setExpanded(!expanded)}
                className="text-text-dim hover:text-text"
              >
                {expanded ? (
                  <ChevronUp className="h-4 w-4" />
                ) : (
                  <ChevronDown className="h-4 w-4" />
                )}
              </button>
            )}
          </div>
        </div>

        {/* Expanded metrics */}
        <AnimatePresence>
          {expanded && metricEntries.length > 0 && (
            <motion.div
              initial={{ height: 0, opacity: 0 }}
              animate={{ height: 'auto', opacity: 1 }}
              exit={{ height: 0, opacity: 0 }}
              className="overflow-hidden"
            >
              <div className="mt-3 pt-3 border-t border-border grid grid-cols-2 sm:grid-cols-4 gap-2">
                {metricEntries.map(([k, v]) => (
                  <div key={k} className="rounded-lg bg-surface-2 p-2">
                    <p className="text-xs text-text-muted">{k}</p>
                    <p className="text-sm font-mono font-medium text-text mt-0.5">
                      {v.toFixed(4)}
                    </p>
                  </div>
                ))}
              </div>
            </motion.div>
          )}
        </AnimatePresence>
      </div>

      {/* Promote modal */}
      <AnimatePresence>
        {showPromote && (
          <PromoteModal
            version={version}
            modelId={modelId}
            hasProductionAlias={productionVersion !== undefined}
            onClose={() => setShowPromote(false)}
          />
        )}
      </AnimatePresence>
    </>
  )
}

export function ModelVersionsTab({ modelId }: Props) {
  const { data: versions, isLoading } = useModelVersions(modelId)
  const { data: aliases } = useModelAliases(modelId)

  const productionVersion = aliases?.production

  if (isLoading) {
    return (
      <div className="flex items-center justify-center py-12 text-text-dim">
        <Loader2 className="h-5 w-5 animate-spin" />
      </div>
    )
  }

  if (!versions || versions.length === 0) {
    return (
      <div className="flex flex-col items-center justify-center py-16 text-center">
        <p className="text-sm font-medium text-text">No versions yet</p>
        <p className="text-xs text-text-muted mt-1">
          Run a training job to create the first version.
        </p>
      </div>
    )
  }

  const sorted = [...versions].sort((a, b) => b.version - a.version)

  return (
    <div className="space-y-3">
      {sorted.map((v) => (
        <VersionRow
          key={v.version}
          version={v}
          modelId={modelId}
          productionVersion={productionVersion}
        />
      ))}
    </div>
  )
}
