import { Handle, Position, useReactFlow } from '@xyflow/react'
import type { NodeProps, Node } from '@xyflow/react'
import type { SizeType } from '@/types/spec'

export type SizeNodeData = { sizeType: SizeType; value: number; disabled?: boolean }
export type SizeNodeType = Node<SizeNodeData, 'size'>

export function SizeNode({ data, id, selected }: NodeProps<SizeNodeType>) {
  const { updateNodeData } = useReactFlow()
  const isPct = data.sizeType === 'percent_of_equity'
  const displayValue = isPct ? +(data.value * 100).toFixed(4) : data.value

  return (
    <div className={`tb-node${selected ? ' selected' : ''}${data.disabled ? ' disabled' : ''}`} style={{ minWidth: 210 }}>
      <div className="tb-node-header" style={{ background: '#0D9488' }}>
        <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5"><line x1="12" y1="1" x2="12" y2="23"/><path d="M17 5H9.5a3.5 3.5 0 0 0 0 7h5a3.5 3.5 0 0 1 0 7H6"/></svg>
        Position Size
      </div>
      <div className="tb-node-body">
        <div className="tb-node-row">
          <span className="tb-node-label">Sized by</span>
          <select className="tb-select" value={data.sizeType} onChange={e => updateNodeData(id, { sizeType: e.target.value as SizeType })}>
            <option value="percent_of_equity">% of equity</option>
            <option value="fixed_quantity">Fixed qty</option>
          </select>
        </div>
        <div className="tb-node-row">
          <span className="tb-node-label">{isPct ? 'Percent' : 'Qty'}</span>
          <input className="tb-input" type="number" min={0} step={isPct ? 0.5 : 1} value={displayValue}
            onChange={e => { const v = parseFloat(e.target.value) || 0; updateNodeData(id, { value: isPct ? v / 100 : v }) }} />
          {isPct && <span className="tb-unit">%</span>}
        </div>
      </div>
      <Handle type="target" position={Position.Left} id="size-in" />
      <Handle type="target" position={Position.Right} id="size-in-r" />
    </div>
  )
}
