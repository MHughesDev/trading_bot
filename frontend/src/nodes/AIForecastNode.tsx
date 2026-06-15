import { Handle, Position, useReactFlow } from '@xyflow/react'
import type { NodeProps, Node } from '@xyflow/react'
import { useQuery } from '@tanstack/react-query'
import type { ForecastDirection } from '@/types/spec'
import { modelsApi } from '@/api/models'

export type AIForecastNodeData = { model: string; direction: ForecastDirection; minConfidence: number; disabled?: boolean }
export type AIForecastNodeType = Node<AIForecastNodeData, 'ai_forecast'>

export function AIForecastNode({ data, id, selected }: NodeProps<AIForecastNodeType>) {
  const { updateNodeData } = useReactFlow()

  const { data: availableModels } = useQuery({
    queryKey: ['models', 'for-node', 'forecaster'],
    queryFn: () => modelsApi.forNode('forecaster').then((r) => r.data.models),
    staleTime: 30_000,
  })

  return (
    <div className={`tb-node${selected ? ' selected' : ''}${data.disabled ? ' disabled' : ''}`} style={{ minWidth: 230 }}>
      <div className="tb-node-header" style={{ background: '#0E7490' }}>
        <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5"><path d="M12 2a10 10 0 1 0 0 20 10 10 0 0 0 0-20z"/><path d="M12 6v6l4 2"/></svg>
        AI Forecast
        <span className="tb-node-header-badge">BETA</span>
      </div>
      <div className="tb-node-body">
        <div className="tb-node-row">
          <span className="tb-node-label">Model</span>
          <select className="tb-select" value={data.model} onChange={e => updateNodeData(id, { model: e.target.value })}>
            {availableModels && availableModels.length > 0 ? (
              availableModels.map((m) => (
                <option
                  key={m.id}
                  value={m.slug}
                  disabled={m.status !== 'active' || !m.has_production}
                >
                  {m.display_name}
                  {(m.status !== 'active' || !m.has_production) ? ' (unavailable)' : ''}
                </option>
              ))
            ) : (
              <option value="" disabled>
                No models available
              </option>
            )}
          </select>
        </div>
        <div className="tb-node-row">
          <span className="tb-node-label">Expects</span>
          <select className="tb-select" value={data.direction} onChange={e => updateNodeData(id, { direction: e.target.value as ForecastDirection })}>
            <option value="bullish">Bullish (price up)</option>
            <option value="bearish">Bearish (price down)</option>
            <option value="any">Any direction</option>
          </select>
        </div>
        <div className="tb-node-row">
          <span className="tb-node-label">Min conf.</span>
          <input className="tb-input" type="number" min={0} max={1} step={0.05} value={data.minConfidence}
            onChange={e => updateNodeData(id, { minConfidence: Math.min(1, Math.max(0, parseFloat(e.target.value) || 0)) })} />
          <span className="tb-unit">0–1</span>
        </div>
      </div>
      <Handle type="source" position={Position.Left} id="forecast-out-l" />
      <Handle type="source" position={Position.Right} id="forecast-out" />
    </div>
  )
}
