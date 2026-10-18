// Version Timeline (Set H H-5.8): newest first on a vertical rail, with the
// aliases each version holds, its headline metrics, and an expandable drawer
// for the artifact refs. Promotion goes through PromotionGateModal.
import { useState } from 'react'
import { ChevronDown, ChevronUp } from 'lucide-react'
import { Button } from '@/components/primitives/Button'
import { Badge } from '@/components/primitives/Badge'
import { Panel } from '@/components/primitives/Panel'
import { EmptyState, ErrorState, PanelLoading } from '@/components/primitives/States'
import {
  useModelVersions,
  useModelAliases,
  usePromoteVersion,
  useEvaluateVersion,
} from '@/hooks/useMlOps'
import { ModelStatusPill } from './ModelStatusPill'
import { PromotionGateModal } from './PromotionGateModal'
import {
  formatMetric,
  metricLabel,
  numericMetrics,
  sortMetricEntries,
  versionTone,
} from './studio.helpers'
import type { AliasMap, ModelVersion } from '@/api/mlops'
import { format } from 'date-fns'

interface Props {
  modelId: string
}

function aliasesOf(version: number, aliases: AliasMap | undefined): string[] {
  return Object.entries(aliases ?? {})
    .filter(([, v]) => v === version)
    .map(([a]) => a)
}

function VersionItem({
  version,
  modelId,
  aliases,
  hasProduction,
}: {
  version: ModelVersion
  modelId: string
  aliases: AliasMap | undefined
  hasProduction: boolean
}) {
  const [open, setOpen] = useState(false)
  const [gate, setGate] = useState(false)
  const evaluate = useEvaluateVersion(modelId)
  const promote = usePromoteVersion(modelId)

  const held = aliasesOf(version.version, aliases)
  const isProduction = held.includes('production')
  const metrics = sortMetricEntries(numericMetrics(version.metrics))
  const tone = versionTone(version.status, isProduction)

  return (
    <li style={{ display: 'flex', gap: 'var(--s-3)' }} data-testid="version-item">
      {/* rail */}
      <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'center', flex: 'none' }} aria-hidden>
        <span
          style={{
            width: 10,
            height: 10,
            borderRadius: '50%',
            marginTop: 6,
            background: tone === 'neutral' ? 'var(--fg-tertiary)' : `var(--fg-${tone === 'accent' ? 'accent' : tone})`,
          }}
        />
        <span style={{ flex: 1, width: 1, background: 'var(--line-hairline)' }} />
      </div>

      <Panel style={{ flex: 1, marginBottom: 'var(--s-3)' }}>
        <div style={{ padding: 'var(--s-3) var(--s-4)', display: 'flex', flexDirection: 'column', gap: 'var(--s-2)' }}>
          <div style={{ display: 'flex', gap: 'var(--s-2)', alignItems: 'center', flexWrap: 'wrap' }}>
            <span className="h2 num">v{version.version}</span>
            <ModelStatusPill status={version.status} />
            {held.map((a) => (
              <Badge key={a} tone={a === 'production' ? 'pos' : 'outline'}>{a}</Badge>
            ))}
            <span className="spacer" />
            <span className="mut num" style={{ fontSize: 'var(--t-11)' }}>
              {format(new Date(version.created_at), 'MMM d, HH:mm')}
            </span>
          </div>

          {version.version_note && <div className="mut">{version.version_note}</div>}

          {metrics.length > 0 ? (
            <div style={{ display: 'flex', gap: 'var(--s-2)', flexWrap: 'wrap' }}>
              {metrics.slice(0, 3).map(([k, v]) => (
                <span key={k} className="chip" title={k}>
                  {metricLabel(k)} <span className="num">{formatMetric(k, v)}</span>
                </span>
              ))}
            </div>
          ) : (
            <span className="mut" style={{ fontSize: 'var(--t-11)' }}>no metrics recorded</span>
          )}

          <div style={{ display: 'flex', gap: 'var(--s-2)', flexWrap: 'wrap' }}>
            {version.status !== 'failed' && version.status !== 'archived' && (
              <Button
                size="sm"
                loading={evaluate.isPending}
                onClick={() => evaluate.mutate(version.version)}
              >
                Evaluate
              </Button>
            )}
            {!isProduction && version.status !== 'failed' && version.status !== 'archived' && (
              <Button size="sm" variant="primary" onClick={() => setGate(true)}>
                Promote…
              </Button>
            )}
            <Button
              size="sm"
              variant="ghost"
              aria-expanded={open}
              trailing={open ? <ChevronUp size={13} aria-hidden /> : <ChevronDown size={13} aria-hidden />}
              onClick={() => setOpen((o) => !o)}
            >
              Details
            </Button>
          </div>

          {evaluate.isError && (
            <span style={{ color: 'var(--fg-neg)', fontSize: 'var(--t-12)' }} role="alert">
              The evaluation could not be started.
            </span>
          )}

          {open && (
            <dl style={{ display: 'grid', gridTemplateColumns: 'max-content 1fr', gap: '4px var(--s-4)', margin: 0, fontSize: 'var(--t-12)' }}>
              <dt className="mut">Artifact</dt>
              <dd style={{ margin: 0 }} className="truncate-1">{version.artifact_uri ?? '—'}</dd>
              <dt className="mut">Hash</dt>
              <dd style={{ margin: 0 }} className="num truncate-1">{version.artifact_hash ?? 'not recorded'}</dd>
              <dt className="mut">Size</dt>
              <dd style={{ margin: 0 }} className="num">{version.size_bytes != null ? `${(version.size_bytes / 1024).toFixed(1)} KB` : '—'}</dd>
              <dt className="mut">Framework</dt>
              <dd style={{ margin: 0 }}>{version.framework_version ?? '—'}</dd>
              {metrics.map(([k, v]) => (
                <div key={k} style={{ display: 'contents' }}>
                  <dt className="mut">{k}</dt>
                  <dd style={{ margin: 0 }} className="num">{v.toFixed(4)}</dd>
                </div>
              ))}
            </dl>
          )}
        </div>
      </Panel>

      {gate && (
        <PromotionGateModal
          open
          version={version}
          hasProduction={hasProduction}
          onClose={() => setGate(false)}
          onPromote={({ environment, overrideReason }) =>
            promote.mutateAsync({ version: version.version, environment, overrideReason })
          }
        />
      )}
    </li>
  )
}

export function ModelVersionsTab({ modelId }: Props) {
  const versions = useModelVersions(modelId)
  const aliases = useModelAliases(modelId)

  if (versions.isLoading) return <PanelLoading lines={5} />
  if (versions.isError) {
    return (
      <ErrorState
        message="Versions could not be loaded."
        detail="This is a failed request, not a model with no versions."
        onRetry={() => void versions.refetch()}
      />
    )
  }
  if (!versions.data || versions.data.length === 0) {
    return (
      <EmptyState
        message="No versions yet"
        detail="Run a training job from the Train tab to create the first version."
      />
    )
  }

  const sorted = [...versions.data].sort((a, b) => b.version - a.version)
  const hasProduction = typeof aliases.data?.production === 'number'

  return (
    <ol style={{ listStyle: 'none', margin: 0, padding: 0 }} aria-label="Version timeline">
      {sorted.map((v) => (
        <VersionItem
          key={v.version}
          version={v}
          modelId={modelId}
          aliases={aliases.data}
          hasProduction={hasProduction}
        />
      ))}
    </ol>
  )
}
