// Command Center (N-5.2) -- /mlops/studio
//
// The fleet: what is training, what is stale, what is serving. One read,
// `GET /api/models/studio/fleet`, and nothing is computed client-side that the
// server did not say: "stale" and "serving" are the server's flags, and a stale
// model shows the server's reason.
import { useMemo, useState } from 'react'
import { Link, useNavigate } from 'react-router-dom'
import { Plus } from 'lucide-react'
import { Button } from '@/components/primitives/Button'
import { Badge } from '@/components/primitives/Badge'
import { Panel, PanelBody, PanelHeader } from '@/components/primitives/Panel'
import { Segmented } from '@/components/primitives/Segmented'
import { StatTile } from '@/components/primitives/Num'
import { EmptyState, ErrorState, PanelLoading } from '@/components/primitives/States'
import { useFleet } from '@/lib/models/queries'
import type { FleetModel } from '@/api/modelsStudio'

type Filter = 'all' | 'training' | 'stale' | 'serving'

function matches(m: FleetModel, f: Filter): boolean {
  switch (f) {
    case 'training':
      return m.active_run !== null
    case 'stale':
      return m.stale
    case 'serving':
      return m.serving
    default:
      return true
  }
}

function FleetRow({ m }: { m: FleetModel }) {
  const run = m.active_run
  return (
    <tr data-testid="fleet-row" data-model={m.model_id}>
      <td>
        <Link to={`/mlops/studio/${m.model_id}`}>{m.display_name}</Link>
        <div className="mut" style={{ fontSize: 'var(--t-11)' }}>
          {m.model_kind}
          {m.family ? ` · ${m.family}` : ''}
        </div>
      </td>
      <td>
        <Badge tone="neutral">{m.status}</Badge>
      </td>
      <td>
        {run ? (
          <span data-testid="fleet-run">
            {run.kind} · {run.phase} · <span className="mono">{Math.round(run.progress)}%</span>
          </span>
        ) : (
          <span className="mut">idle</span>
        )}
      </td>
      <td>
        {m.stale ? (
          <Badge tone="warn" data-testid="fleet-stale" title={m.stale_reason ?? undefined}>
            stale{m.stale_reason ? `: ${m.stale_reason}` : ''}
          </Badge>
        ) : (
          <span className="mut">current</span>
        )}
      </td>
      <td>
        {m.serving ? (
          <Badge tone="pos">serving v{m.production_version ?? '?'}</Badge>
        ) : (
          <span className="mut">not serving</span>
        )}
      </td>
      <td className="mono">{m.latest_version === null ? 'none' : `v${m.latest_version}`}</td>
      <td className="mut">{m.last_trained_at ? m.last_trained_at.slice(0, 10) : 'never'}</td>
    </tr>
  )
}

export function ModelCommandCenterPage() {
  const fleet = useFleet()
  const navigate = useNavigate()
  const [filter, setFilter] = useState<Filter>('all')

  const rows = useMemo(
    () => (fleet.data?.models ?? []).filter((m) => matches(m, filter)),
    [fleet.data, filter],
  )

  const counts = fleet.data?.counts

  return (
    <>
      <div className="pagehead">
        <h1 className="h1">Model Studio</h1>
        <span className="lbl">What is training, what is stale, what is serving</span>
        <div className="spacer" />
        <Button size="sm" variant="primary" icon={<Plus size={13} aria-hidden />} onClick={() => navigate('/mlops/studio/new')}>
          New model
        </Button>
        <Button size="sm" onClick={() => navigate('/mlops')}>
          Classic view
        </Button>
      </div>
      <div className="page-body" style={{ display: 'flex', flexDirection: 'column', gap: 'var(--s-3)' }}>
        {counts && (
          <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fit, minmax(140px, 1fr))', gap: 'var(--s-3)' }} data-testid="fleet-counts">
            <StatTile label="Models" value={counts.models} />
            <StatTile label="Training" value={counts.training} />
            <StatTile label="Evaluating" value={counts.evaluating} />
            <StatTile label="Stale" value={counts.stale} tone={counts.stale > 0 ? 'warn' : undefined} />
            <StatTile label="Serving" value={counts.serving} />
          </div>
        )}
        <Panel>
          <PanelHeader
            title="Fleet"
            actions={
              <Segmented<Filter>
                ariaLabel="Fleet filter"
                value={filter}
                onChange={setFilter}
                options={[
                  { value: 'all', label: 'All' },
                  { value: 'training', label: 'Training' },
                  { value: 'stale', label: 'Stale' },
                  { value: 'serving', label: 'Serving' },
                ]}
              />
            }
          />
          <PanelBody flush={!fleet.isLoading && !fleet.isError && rows.length > 0}>
            {fleet.isLoading ? (
              <PanelLoading lines={6} />
            ) : fleet.isError ? (
              <ErrorState message="The fleet could not be read." onRetry={() => void fleet.refetch()} />
            ) : (fleet.data?.models.length ?? 0) === 0 ? (
              <EmptyState
                message="No models yet"
                detail="Declare a dataset and train a first model."
                action={
                  <Button size="sm" variant="primary" onClick={() => navigate('/mlops/studio/new')}>
                    New model
                  </Button>
                }
              />
            ) : rows.length === 0 ? (
              <EmptyState message={`No ${filter} models`} detail="Nothing in the fleet matches this filter." />
            ) : (
              <table className="tbl" aria-label="Model fleet">
                <thead>
                  <tr>
                    <th>Model</th>
                    <th>Status</th>
                    <th>Run</th>
                    <th>Freshness</th>
                    <th>Serving</th>
                    <th>Latest</th>
                    <th>Last trained</th>
                  </tr>
                </thead>
                <tbody>
                  {rows.map((m) => (
                    <FleetRow key={m.model_id} m={m} />
                  ))}
                </tbody>
              </table>
            )}
          </PanelBody>
        </Panel>
      </div>
    </>
  )
}
