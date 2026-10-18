// Cockpit shell (N-5.4) -- /mlops/studio/:id/(overview|training|test|evaluations|versions|lineage)
//
// One page per model. The shell owns the model header and the tab bar; each tab
// is a component in components/models and owns its own loading / empty / error
// states. The tab is the first segment after the model id, so a deep link to
// `/mlops/studio/abc/evaluations?v=3` lands on that tab and version.
import { useNavigate, useParams, useSearchParams } from 'react-router-dom'
import { Badge } from '@/components/primitives/Badge'
import { Button } from '@/components/primitives/Button'
import { Tabs } from '@/components/primitives/Segmented'
import { EmptyState, ErrorState, PanelLoading } from '@/components/primitives/States'
import { useModel } from '@/hooks/useMlOps'
import { CockpitOverview } from '@/components/models/CockpitOverview'
import { TrainingConsole } from '@/components/models/TrainingConsole'
import { TestLab } from '@/components/models/TestLab'
import { EvaluationsArena } from '@/components/models/EvaluationsArena'
import { PromotionPanel } from '@/components/models/PromotionPanel'
import { LineageGraphView } from '@/components/models/LineageGraphView'

const TABS = [
  { value: 'overview', label: 'Overview' },
  { value: 'training', label: 'Training' },
  { value: 'test', label: 'Test lab' },
  { value: 'evaluations', label: 'Evaluations' },
  { value: 'versions', label: 'Versions and promotion' },
  { value: 'lineage', label: 'Lineage' },
] as const

type TabId = (typeof TABS)[number]['value']

function tabOf(segment: string | undefined): TabId {
  const first = (segment ?? '').split('/')[0]
  return (TABS.find((t) => t.value === first)?.value ?? 'overview') as TabId
}

export function ModelCockpitPage() {
  const params = useParams()
  const id = params.id ?? ''
  const tab = tabOf(params['*'])
  const navigate = useNavigate()
  const [search] = useSearchParams()
  const model = useModel(id)

  const go = (t: TabId) => {
    const q = search.get('v')
    navigate(`/mlops/studio/${id}/${t}${q ? `?v=${q}` : ''}`)
  }

  if (model.isLoading) {
    return (
      <div className="page-body">
        <PanelLoading lines={6} />
      </div>
    )
  }
  if (model.isError || !model.data) {
    return (
      <div className="page-body">
        <ErrorState
          message="This model could not be read."
          detail="It may not exist, or the registry is unreachable."
          onRetry={() => void model.refetch()}
        />
      </div>
    )
  }
  if (!id) return <EmptyState message="No model chosen" />

  const m = model.data
  return (
    <>
      <div className="pagehead">
        <h1 className="h1">{m.display_name}</h1>
        <Badge tone="neutral">{m.status}</Badge>
        <span className="lbl">{m.model_kind}</span>
        <div className="spacer" />
        <Button size="sm" onClick={() => navigate('/mlops/studio')}>
          Command Center
        </Button>
      </div>
      <div className="page-body" style={{ display: 'flex', flexDirection: 'column', gap: 'var(--s-3)' }}>
        <Tabs<TabId>
          ariaLabel="Model cockpit"
          value={tab}
          onChange={go}
          tabs={TABS.map((t) => ({ value: t.value, label: t.label }))}
        />
        {tab === 'overview' && <CockpitOverview modelId={id} />}
        {tab === 'training' && <TrainingConsole modelId={id} />}
        {tab === 'test' && <TestLab modelId={id} />}
        {tab === 'evaluations' && <EvaluationsArena modelId={id} />}
        {tab === 'versions' && <PromotionPanel modelId={id} />}
        {tab === 'lineage' && <LineageGraphView modelId={id} />}
      </div>
    </>
  )
}
