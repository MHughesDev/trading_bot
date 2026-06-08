import { Handle, Position, useReactFlow } from '@xyflow/react'
import type { NodeProps, Node } from '@xyflow/react'
import type { IndicatorKind } from '@/types/spec'
import { INDICATOR_LABELS } from '@/types/spec'

export type IndicatorNodeData = { indicatorId: string; kind: IndicatorKind; period: number; disabled?: boolean }
export type IndicatorNodeType = Node<IndicatorNodeData, 'indicator'>

export function IndicatorNode({ data, id, selected }: NodeProps<IndicatorNodeType>) {
  const { updateNodeData } = useReactFlow()

  return (
    <div className={`tb-node${selected ? ' selected' : ''}${data.disabled ? ' disabled' : ''}`}>
      <div className="tb-node-header" style={{ background: '#7C3AED' }}>
        <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5"><polyline points="22 12 18 12 15 21 9 3 6 12 2 12"/></svg>
        Indicator
      </div>
      <div className="tb-node-body">
        <div className="tb-node-row">
          <span className="tb-node-label">Name</span>
          <input className="tb-input" value={data.indicatorId} onChange={e => updateNodeData(id, { indicatorId: e.target.value })} placeholder="e.g. ema_fast" spellCheck={false} />
        </div>
        <div className="tb-node-row">
          <span className="tb-node-label">Type</span>
          <select className="tb-select" value={data.kind} onChange={e => updateNodeData(id, { kind: e.target.value as IndicatorKind })}>
            {(Object.keys(INDICATOR_LABELS) as IndicatorKind[]).map(k => (
              <option key={k} value={k}>{k.toUpperCase()}</option>
            ))}
          </select>
        </div>
        <div className="tb-node-row">
          <span className="tb-node-label">Period</span>
          <input className="tb-input" type="number" min={1} max={500} value={data.period} onChange={e => updateNodeData(id, { period: Math.max(1, parseInt(e.target.value) || 1) })} />
        </div>
      </div>
      <Handle type="source" position={Position.Right} id="value-out" />
    </div>
  )
}
