import { Handle, Position, useReactFlow } from '@xyflow/react'
import type { NodeProps, Node } from '@xyflow/react'
import type { ExitType } from '@/types/spec'
import { EXIT_LABELS } from '@/types/spec'

export type ExitNodeData = { exitType: ExitType; value: number; disabled?: boolean }
export type ExitNodeType = Node<ExitNodeData, 'exit'>

export function ExitNode({ data, id, selected }: NodeProps<ExitNodeType>) {
  const { updateNodeData } = useReactFlow()
  const displayValue = +(data.value * 100).toFixed(4)

  return (
    <div className={`tb-node${selected ? ' selected' : ''}${data.disabled ? ' disabled' : ''}`} style={{ minWidth: 210 }}>
      <div className="tb-node-header" style={{ background: '#9F1239' }}>
        <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5"><polygon points="7.86 2 16.14 2 22 7.86 22 16.14 16.14 22 7.86 22 2 16.14 2 7.86 7.86 2"/><line x1="15" y1="9" x2="9" y2="15"/><line x1="9" y1="9" x2="15" y2="15"/></svg>
        Exit Rule
      </div>
      <div className="tb-node-body">
        <div className="tb-node-row">
          <span className="tb-node-label">Type</span>
          <select className="tb-select" value={data.exitType} onChange={e => updateNodeData(id, { exitType: e.target.value as ExitType })}>
            {(Object.entries(EXIT_LABELS) as [ExitType, string][]).map(([k, v]) => (
              <option key={k} value={k}>{v}</option>
            ))}
          </select>
        </div>
        <div className="tb-node-row">
          <span className="tb-node-label">At</span>
          <input className="tb-input" type="number" min={0.01} max={100} step={0.1} value={displayValue}
            onChange={e => updateNodeData(id, { value: (parseFloat(e.target.value) || 0) / 100 })} />
          <span className="tb-unit">%</span>
        </div>
      </div>
      <Handle type="target" position={Position.Left} id="exit-in" />
    </div>
  )
}
