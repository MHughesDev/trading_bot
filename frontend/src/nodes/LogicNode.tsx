import { Handle, Position, useReactFlow } from '@xyflow/react'
import type { NodeProps, Node } from '@xyflow/react'

export type LogicNodeData = { op: 'and' | 'or'; inputCount: number; disabled?: boolean }
export type LogicNodeType = Node<LogicNodeData, 'logic'>

const COLORS = { and: '#1E40AF', or: '#6D28D9' }

export function LogicNode({ data, id, selected }: NodeProps<LogicNodeType>) {
  const { updateNodeData } = useReactFlow()

  return (
    <div className={`tb-node${selected ? ' selected' : ''}${data.disabled ? ' disabled' : ''}`} style={{ minWidth: 180 }}>
      <div className="tb-node-header" style={{ background: COLORS[data.op] }}>
        <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5"><circle cx="18" cy="18" r="3"/><circle cx="6" cy="6" r="3"/><path d="M13 6h3a2 2 0 0 1 2 2v7"/><line x1="6" y1="9" x2="6" y2="21"/></svg>
        Logic Gate
      </div>
      <div className="tb-node-body">
        <div className="tb-node-row">
          <span className="tb-node-label">Mode</span>
          <select className="tb-select" value={data.op} onChange={e => updateNodeData(id, { op: e.target.value as 'and' | 'or' })}>
            <option value="and">AND — all must match</option>
            <option value="or">OR — any can match</option>
          </select>
        </div>
        <div className="tb-node-row">
          <span className="tb-node-label">Inputs</span>
          <input className="tb-input" type="number" min={2} max={6} value={data.inputCount}
            onChange={e => updateNodeData(id, { inputCount: Math.min(6, Math.max(2, parseInt(e.target.value) || 2)) })} />
        </div>
      </div>
      {Array.from({ length: data.inputCount }, (_, i) => (
        <Handle key={`in-${i}`} type="target" position={Position.Left} id={`logic-in-${i}`}
          style={{ top: `${((i + 1) / (data.inputCount + 1)) * 100}%` }} />
      ))}
      {Array.from({ length: data.inputCount }, (_, i) => (
        <Handle key={`in-r-${i}`} type="target" position={Position.Right} id={`logic-in-r-${i}`}
          style={{ top: `${((i + 1) / (data.inputCount + 1)) * 100}%` }} />
      ))}
      <Handle type="source" position={Position.Left} id="logic-out-l" style={{ top: '15%' }} />
      <Handle type="source" position={Position.Right} id="logic-out" style={{ top: '15%' }} />
    </div>
  )
}
