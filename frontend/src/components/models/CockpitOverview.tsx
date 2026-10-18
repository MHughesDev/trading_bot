// Cockpit Overview (N-5.4): lineage summary, versions, the current alias, and the
// scorecard WITH its baselines. Each panel carries its own loading / empty /
// error state so one failed read does not blank the page.
import { Badge, KeyValue } from '@/components/primitives/Badge'
import { Panel, PanelBody, PanelHeader } from '@/components/primitives/Panel'
import { EmptyState, ErrorState, PanelLoading } from '@/components/primitives/States'
import { useModelAliases, useModelDeployments, useModelRuns, useModelUsedBy } from '@/hooks/useMlOps'
import { useImportance, useVerdict } from '@/lib/models/queries'
import { useSelectedVersion } from '@/lib/models/useSelectedVersion'
import { VerdictPanelBody } from './VerdictPanel'
import { VersionPicker } from './VersionPicker'

function ImportancePanel({ modelId, version }: { modelId: string; version: number | null }) {
  const q = useImportance(modelId, version)
  if (version === null) return null
  return (
    <Panel>
      <PanelHeader title="Feature importance" />
      <PanelBody>
        {q.isLoading ? (
          <PanelLoading lines={3} />
        ) : q.isError || !q.data ? (
          <ErrorState message="Importance could not be read." onRetry={() => void q.refetch()} />
        ) : !q.data.available ? (
          <EmptyState message="Not available for this version" detail={q.data.reason ?? 'The server gave no reason.'} />
        ) : (
          <table className="tbl dense" aria-label="Clustered permutation importance">
            <thead>
              <tr>
                <th>Cluster</th>
                <th className="n">Importance</th>
                <th className="n">Interval</th>
              </tr>
            </thead>
            <tbody>
              {(q.data.importance?.clusters ?? []).map((c) => (
                <tr key={c.members.join('|')} data-testid="importance-cluster">
                  <td className="mono">{c.members.join(', ')}</td>
                  <td className="n">{c.mean.toFixed(4)}</td>
                  <td className="n">
                    {c.interval
                      ? `${c.interval[0].toFixed(4)} to ${c.interval[1].toFixed(4)}`
                      : `not estimable from ${q.data?.importance?.folds ?? 0} fold(s)`}
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

export function CockpitOverview({ modelId }: { modelId: string }) {
  const sel = useSelectedVersion(modelId)
  const aliases = useModelAliases(modelId)
  const runs = useModelRuns(modelId)
  const deployments = useModelDeployments(modelId)
  const usedBy = useModelUsedBy(modelId)
  const verdict = useVerdict(modelId, sel.version)

  const held = Object.entries(aliases.data ?? {})

  return (
    <div style={{ display: 'grid', gap: 'var(--s-3)', gridTemplateColumns: 'repeat(auto-fit, minmax(340px, 1fr))' }}>
      <Panel>
        <PanelHeader title="Lineage summary" />
        <PanelBody>
          {sel.query.isLoading ? (
            <PanelLoading lines={4} />
          ) : sel.query.isError ? (
            <ErrorState message="Versions could not be read." onRetry={() => void sel.query.refetch()} />
          ) : (
            <div data-testid="lineage-summary">
              <KeyValue k="Versions" v={sel.versions.length} />
              <KeyValue k="Training runs" v={runs.isLoading ? '…' : runs.isError ? 'unread' : (runs.data ?? []).filter((r) => r.kind === 'training').length} />
              <KeyValue k="Deployments" v={deployments.isLoading ? '…' : deployments.isError ? 'unread' : (deployments.data ?? []).length} />
              <KeyValue k="Used by" v={usedBy.isLoading ? '…' : usedBy.isError ? 'unread' : `${(usedBy.data ?? []).length} strateg${(usedBy.data ?? []).length === 1 ? 'y' : 'ies'}`} />
            </div>
          )}
        </PanelBody>
      </Panel>

      <Panel>
        <PanelHeader title="Aliases" />
        <PanelBody>
          {aliases.isLoading ? (
            <PanelLoading lines={2} />
          ) : aliases.isError ? (
            <ErrorState message="Aliases could not be read." onRetry={() => void aliases.refetch()} />
          ) : held.length === 0 ? (
            <EmptyState message="No alias set" detail="Nothing is promoted: no version is production." />
          ) : (
            <div data-testid="aliases">
              {held.map(([a, v]) => (
                <KeyValue key={a} k={a} v={<Badge tone={a === 'production' ? 'pos' : 'neutral'}>v{String(v)}</Badge>} />
              ))}
            </div>
          )}
        </PanelBody>
      </Panel>

      <Panel style={{ gridColumn: '1 / -1' }}>
        <PanelHeader
          title="Scorecard and baselines"
          actions={<VersionPicker versions={sel.versions} aliases={sel.aliases} value={sel.version} onChange={sel.select} />}
        />
        <PanelBody>
          <VerdictPanelBody
            version={sel.query.isLoading ? -1 : sel.version}
            verdict={verdict.data}
            loading={sel.query.isLoading || verdict.isLoading}
            error={verdict.isError}
            onRetry={() => void verdict.refetch()}
          />
        </PanelBody>
      </Panel>

      <ImportancePanel modelId={modelId} version={sel.version} />

      <Panel>
        <PanelHeader title="Versions" />
        <PanelBody flush={sel.versions.length > 0}>
          {sel.query.isLoading ? (
            <PanelLoading lines={3} />
          ) : sel.versions.length === 0 ? (
            <EmptyState message="No versions yet" detail="Train this model to produce its first version." />
          ) : (
            <table className="tbl dense" aria-label="Versions">
              <thead>
                <tr>
                  <th>Version</th>
                  <th>Status</th>
                  <th>Created</th>
                </tr>
              </thead>
              <tbody>
                {[...sel.versions]
                  .sort((a, b) => b.version - a.version)
                  .map((v) => (
                    <tr key={v.version}>
                      <td className="mono">v{v.version}</td>
                      <td>{v.status}</td>
                      <td className="mut">{v.created_at.slice(0, 10)}</td>
                    </tr>
                  ))}
              </tbody>
            </table>
          )}
        </PanelBody>
      </Panel>
    </div>
  )
}
