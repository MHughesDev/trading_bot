import { Handle, Position, useReactFlow } from '@xyflow/react'
import type { NodeProps, Node } from '@xyflow/react'
import type { Side } from '@/types/spec'

export type ActionNodeData = { side: Side; disabled?: boolean }
export type ActionNodeType = Node<ActionNodeData, 'action'>

const COLORS: Record<Side, string> = { buy: '#15803D', sell: '#9F1239' }

export function ActionNode({ data, id, selected }: NodeProps<ActionNodeType>) {
  const { updateNodeData } = useReactFlow()

  return (
    <div className={`tb-node${selected ? ' selected' : ''}${data.disabled ? ' disabled' : ''}`} style={{ minWidth: 190 }}>
      <div className="tb-node-header" style={{ background: COLORS[data.side] }}>
        <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5"><circle cx="12" cy="12" r="10"/><line x1="12" y1="8" x2="12" y2="16"/><line x1="8" y1="12" x2="16" y2="12"/></svg>
        Trade Action
      </div>
      <div className="tb-node-body">
        <div className="tb-node-row">
          <span className="tb-node-label">Action</span>
          <select className="tb-select" value={data.side} onChange={e => updateNodeData(id, { side: e.target.value as Side })}>
            <option value="buy">Buy (Long)</option>
            <option value="sell">Sell / Short</option>
          </select>
        </div>
        <div style={{ fontSize: 9, color: 'var(--tb-text-dim)', lineHeight: 1.4, paddingTop: 2 }}>
          Condition in → left or right.<br />Size and Exit out → left or right.
        </div>
      </div>
      <Handle type="target" position={Position.Left} id="action-in" />
      <Handle type="target" position={Position.Right} id="action-in-r" />
      <Handle type="source" position={Position.Right} id="size-out" style={{ top: '35%' }} />
      <span className="tb-handle-label" style={{ right: 14, top: 'calc(35% - 14px)' }}>size</span>
      <Handle type="source" position={Position.Right} id="exit-out" style={{ top: '65%' }} />
      <span className="tb-handle-label" style={{ right: 14, top: 'calc(65% - 14px)' }}>exits</span>
      <Handle type="source" position={Position.Left} id="size-out-l" style={{ top: '35%' }} />
      <span className="tb-handle-label" style={{ left: 14, top: 'calc(35% - 14px)' }}>size</span>
      <Handle type="source" position={Position.Left} id="exit-out-l" style={{ top: '65%' }} />
      <span className="tb-handle-label" style={{ left: 14, top: 'calc(65% - 14px)' }}>exits</span>
    </div>
  )
}
