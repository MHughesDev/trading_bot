import { Handle, Position, useReactFlow } from '@xyflow/react'
import type { NodeProps, Node } from '@xyflow/react'
import type { ConditionType } from '@/types/spec'
import { CONDITION_LABELS } from '@/types/spec'

export type ConditionNodeData = { conditionType: ConditionType; rightMode: 'indicator' | 'value'; rightValue: number; disabled?: boolean }
export type ConditionNodeType = Node<ConditionNodeData, 'condition'>

const UNARY: ConditionType[] = ['rising', 'falling']

export function ConditionNode({ data, id, selected }: NodeProps<ConditionNodeType>) {
  const { updateNodeData } = useReactFlow()
  const isUnary = UNARY.includes(data.conditionType)

  return (
    <div className={`tb-node${selected ? ' selected' : ''}${data.disabled ? ' disabled' : ''}`} style={{ minWidth: 230 }}>
      <div className="tb-node-header" style={{ background: '#B45309' }}>
        <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5"><circle cx="12" cy="12" r="10"/><line x1="12" y1="8" x2="12" y2="12"/><line x1="12" y1="16" x2="12.01" y2="16"/></svg>
        Condition
      </div>
      <div className="tb-node-body">
        <div className="tb-node-row">
          <span className="tb-node-label">Rule</span>
          <select className="tb-select" value={data.conditionType} onChange={e => updateNodeData(id, { conditionType: e.target.value as ConditionType })}>
            {(Object.entries(CONDITION_LABELS) as [ConditionType, string][]).map(([k, v]) => (
              <option key={k} value={k}>{v}</option>
            ))}
          </select>
        </div>
        {!isUnary && (
          <div className="tb-node-row">
            <span className="tb-node-label">Compare</span>
            <select className="tb-select" value={data.rightMode} onChange={e => updateNodeData(id, { rightMode: e.target.value as 'indicator' | 'value' })}>
              <option value="indicator">Indicator (connect →)</option>
              <option value="value">Fixed number</option>
            </select>
          </div>
        )}
        {!isUnary && data.rightMode === 'value' && (
          <div className="tb-node-row">
            <span className="tb-node-label">Value</span>
            <input className="tb-input" type="number" value={data.rightValue} step="any" onChange={e => updateNodeData(id, { rightValue: parseFloat(e.target.value) || 0 })} />
          </div>
        )}
        {!isUnary && data.rightMode === 'indicator' && (
          <div className="tb-node-row">
            <span style={{ color: 'var(--tb-text-dim)', fontSize: 10 }}>connect an indicator to either side</span>
          </div>
        )}
      </div>
      <Handle type="target" position={Position.Left} id="left-in" style={{ top: isUnary ? '50%' : '35%' }} />
      <span className="tb-handle-label" style={{ left: 14, top: isUnary ? 'calc(50% - 14px)' : 'calc(35% - 14px)' }}>left</span>
      {!isUnary && data.rightMode === 'indicator' && (
        <>
          <Handle type="target" position={Position.Left} id="right-in" style={{ top: '65%' }} />
          <span className="tb-handle-label" style={{ left: 14, top: 'calc(65% - 14px)' }}>right</span>
        </>
      )}
      <Handle type="target" position={Position.Right} id="left-in-r" style={{ top: isUnary ? '70%' : '35%' }} />
      <span className="tb-handle-label" style={{ right: 14, top: isUnary ? 'calc(70% - 14px)' : 'calc(35% - 14px)' }}>left</span>
      {!isUnary && data.rightMode === 'indicator' && (
        <>
          <Handle type="target" position={Position.Right} id="right-in-r" style={{ top: '65%' }} />
          <span className="tb-handle-label" style={{ right: 14, top: 'calc(65% - 14px)' }}>right</span>
        </>
      )}
      <Handle type="source" position={Position.Left} id="cond-out-l" style={{ top: isUnary ? '20%' : undefined }} />
      <Handle type="source" position={Position.Right} id="cond-out" />
    </div>
  )
}
