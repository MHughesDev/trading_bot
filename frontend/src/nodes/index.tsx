import { Handle, Position } from '@xyflow/react'
import type { Node, NodeProps } from '@xyflow/react'
import {
  CONDITION_LABELS,
  EXIT_LABELS,
  INDICATOR_LABELS,
  type ConditionType,
  type ExitType,
  type ForecastDirection,
  type IndicatorKind,
  type InferenceTargetKind,
  type Side,
  type SizeType,
} from '@/types/spec'
import { NodeField, NodeShell } from './NodeShell'

/* =============================================================================
   Node renderers.

   Spec §4.3: nodes show a SUMMARY; the inspector holds the full form. Nodes
   stay small so the graph stays readable. Nothing here edits in place except
   through the inspector — that is the whole point of the redesign.

   Handle ids are unchanged from the previous builder so saved graphs, the
   compiler and `fromDefinition` keep working.
   ============================================================================= */

/* --- Market data (family: data) -------------------------------------------- */

export type MarketDataSource = 'price' | 'volume' | 'orderbook' | 'funding'

export const MARKET_DATA_LABELS: Record<MarketDataSource, string> = {
  price: 'Price series',
  volume: 'Volume',
  orderbook: 'Order book',
  funding: 'Funding rate',
}

export const MARKET_DATA_LANES: Record<MarketDataSource, string> = {
  price: 'market.bars.1m',
  volume: 'market.bars.1m',
  orderbook: 'market.orderbook.l2',
  funding: 'market.funding_rate',
}

export type MarketDataNodeData = {
  source: MarketDataSource
  /** Empty means "bound when the strategy is deployed to an instrument". */
  symbol: string
  timeframe: string
  field: 'close' | 'open' | 'high' | 'low'
  disabled?: boolean
}
export type MarketDataNodeType = Node<MarketDataNodeData, 'market_data'>

export function MarketDataNode({ data, selected }: NodeProps<MarketDataNodeType>) {
  return (
    <>
      <NodeShell
        family="data"
        selected={selected}
        disabled={data.disabled}
        title={data.symbol || 'Bound at deploy'}
      >
        <NodeField k="Source" v={MARKET_DATA_LABELS[data.source]} />
        <NodeField k="Timeframe" v={data.timeframe} />
        {data.source === 'price' && <NodeField k="Field" v={data.field} />}
      </NodeShell>
      <Handle type="source" position={Position.Left} id="data-out-l" />
      <Handle type="source" position={Position.Right} id="data-out" />
    </>
  )
}

/* --- Indicator -------------------------------------------------------------- */

export type IndicatorNodeData = {
  indicatorId: string
  kind: IndicatorKind
  period: number
  disabled?: boolean
}
export type IndicatorNodeType = Node<IndicatorNodeData, 'indicator'>

export function IndicatorNode({ data, selected }: NodeProps<IndicatorNodeType>) {
  return (
    <>
      <NodeShell
        family="indicator"
        selected={selected}
        disabled={data.disabled}
        invalid={!data.indicatorId}
        warning={!data.indicatorId ? 'This indicator needs a name before it can be referenced.' : undefined}
        title={data.indicatorId || 'Unnamed indicator'}
      >
        <NodeField k="Type" v={data.kind.toUpperCase()} />
        <NodeField k="Period" v={data.period} />
      </NodeShell>
      <Handle type="source" position={Position.Left} id="value-out-l" />
      <Handle type="source" position={Position.Right} id="value-out" />
    </>
  )
}

/* --- Condition / signal ----------------------------------------------------- */

export type ConditionNodeData = {
  conditionType: ConditionType
  rightMode: 'indicator' | 'value'
  rightValue: number
  disabled?: boolean
}
export type ConditionNodeType = Node<ConditionNodeData, 'condition'>

const UNARY: ConditionType[] = ['rising', 'falling']

export function ConditionNode({ data, selected }: NodeProps<ConditionNodeType>) {
  const isUnary = UNARY.includes(data.conditionType)
  return (
    <>
      <NodeShell
        family="signal"
        selected={selected}
        disabled={data.disabled}
        invalid={isUnary}
        warning={
          isUnary
            ? 'Rising and falling cannot be saved to the v1.0 definition format yet — it has no access to prior bars.'
            : undefined
        }
        title={CONDITION_LABELS[data.conditionType]}
      >
        <NodeField k="Left" v="input" />
        <NodeField k="Right" v={isUnary ? '—' : data.rightMode === 'value' ? data.rightValue : 'input'} />
      </NodeShell>
      <Handle type="target" position={Position.Left} id="left-in" style={{ top: isUnary ? '50%' : '35%' }} />
      {!isUnary && data.rightMode === 'indicator' && (
        <Handle type="target" position={Position.Left} id="right-in" style={{ top: '65%' }} />
      )}
      <Handle type="target" position={Position.Right} id="left-in-r" style={{ top: isUnary ? '70%' : '35%' }} />
      {!isUnary && data.rightMode === 'indicator' && (
        <Handle type="target" position={Position.Right} id="right-in-r" style={{ top: '65%' }} />
      )}
      <Handle type="source" position={Position.Left} id="cond-out-l" style={{ top: isUnary ? '20%' : undefined }} />
      <Handle type="source" position={Position.Right} id="cond-out" />
    </>
  )
}

/* --- AI inference ----------------------------------------------------------- */

export type AIInferenceNodeData = {
  targetKind: InferenceTargetKind
  targetRef: string
  alias: string
  direction: ForecastDirection
  minConfidence: number
  featureSet?: string
  timeframe: string
  lookback: number
  disabled?: boolean
}
export type AIInferenceNodeType = Node<AIInferenceNodeData, 'ai_inference'>

export function AIInferenceNode({ data, selected }: NodeProps<AIInferenceNodeType>) {
  return (
    <>
      <NodeShell
        family="ai"
        selected={selected}
        disabled={data.disabled}
        invalid={!data.targetRef}
        warning={!data.targetRef ? 'Choose a model in the inspector before saving.' : undefined}
        title="AI forecast"
      >
        <NodeField k="Model" v={data.targetRef || 'none'} />
        <NodeField k="Expects" v={data.direction} />
        <NodeField k="Min conf." v={data.minConfidence} />
      </NodeShell>
      <Handle type="source" position={Position.Left} id="forecast-out-l" />
      <Handle type="source" position={Position.Right} id="forecast-out" />
    </>
  )
}

/* --- Logic ------------------------------------------------------------------ */

export type LogicNodeData = { op: 'and' | 'or'; inputCount: number; disabled?: boolean }
export type LogicNodeType = Node<LogicNodeData, 'logic'>

export function LogicNode({ data, selected }: NodeProps<LogicNodeType>) {
  const n = Math.max(2, Math.min(6, data.inputCount || 2))
  return (
    <>
      <NodeShell family="logic" selected={selected} disabled={data.disabled} title={`${data.op.toUpperCase()} gate`}>
        <NodeField k="Inputs" v={n} />
        <NodeField k="Mode" v={data.op === 'and' ? 'all true' : 'any true'} />
      </NodeShell>
      {Array.from({ length: n }).map((_, i) => (
        <Handle
          key={`in-${i}`}
          type="target"
          position={Position.Left}
          id={`logic-in-${i}`}
          style={{ top: `${((i + 1) / (n + 1)) * 100}%` }}
        />
      ))}
      {Array.from({ length: n }).map((_, i) => (
        <Handle
          key={`in-r-${i}`}
          type="target"
          position={Position.Right}
          id={`logic-in-r-${i}`}
          style={{ top: `${((i + 1) / (n + 1)) * 100}%` }}
        />
      ))}
      <Handle type="source" position={Position.Left} id="logic-out-l" style={{ top: '15%' }} />
      <Handle type="source" position={Position.Right} id="logic-out" style={{ top: '15%' }} />
    </>
  )
}

/* --- Action (intent) -------------------------------------------------------- */

export type ActionNodeData = { side: Side; disabled?: boolean }
export type ActionNodeType = Node<ActionNodeData, 'action'>

export function ActionNode({ data, selected }: NodeProps<ActionNodeType>) {
  return (
    <>
      <NodeShell
        family="intent"
        selected={selected}
        disabled={data.disabled}
        title={data.side === 'buy' ? 'Buy / Long' : 'Sell / Short'}
      >
        <NodeField k="Side" v={data.side === 'buy' ? 'Long' : 'Short'} />
        <NodeField k="Order" v="Market" />
      </NodeShell>
      <Handle type="target" position={Position.Left} id="action-in" />
      <Handle type="target" position={Position.Right} id="action-in-r" />
      <Handle type="source" position={Position.Right} id="size-out" style={{ top: '35%' }} />
      <Handle type="source" position={Position.Right} id="exit-out" style={{ top: '65%' }} />
      <Handle type="source" position={Position.Left} id="size-out-l" style={{ top: '35%' }} />
      <Handle type="source" position={Position.Left} id="exit-out-l" style={{ top: '65%' }} />
    </>
  )
}

/* --- Size (intent) ---------------------------------------------------------- */

export type SizeNodeData = { sizeType: SizeType; value: number; disabled?: boolean }
export type SizeNodeType = Node<SizeNodeData, 'size'>

export function SizeNode({ data, selected }: NodeProps<SizeNodeType>) {
  const isPct = data.sizeType === 'percent_of_equity'
  return (
    <>
      <NodeShell family="intent" selected={selected} disabled={data.disabled} title="Position size" familyLabel="Position size">
        <NodeField k="Sized by" v={isPct ? '% equity' : 'fixed qty'} />
        <NodeField k={isPct ? 'Percent' : 'Quantity'} v={isPct ? (data.value * 100).toFixed(1) : data.value} />
      </NodeShell>
      <Handle type="target" position={Position.Left} id="size-in" />
      <Handle type="target" position={Position.Right} id="size-in-r" />
    </>
  )
}

/* --- Exit (risk) ------------------------------------------------------------ */

export type ExitNodeData = { exitType: ExitType; value: number; disabled?: boolean }
export type ExitNodeType = Node<ExitNodeData, 'exit'>

export function ExitNode({ data, selected }: NodeProps<ExitNodeType>) {
  return (
    <>
      <NodeShell
        family="risk"
        selected={selected}
        disabled={data.disabled}
        title={EXIT_LABELS[data.exitType]}
        warning="Exit rules are not represented in the saved v1.0 format yet and will not be backtested."
      >
        <NodeField k="Type" v={data.exitType.replace(/_/g, ' ')} />
        <NodeField k="Value" v={`${(data.value * 100).toFixed(1)}%`} />
      </NodeShell>
      <Handle type="target" position={Position.Left} id="exit-in" />
      <Handle type="target" position={Position.Right} id="exit-in-r" />
    </>
  )
}

export { INDICATOR_LABELS, CONDITION_LABELS, EXIT_LABELS }
export { NodeShell, NodeField } from './NodeShell'
export type { NodeFamily, FamilyInfo } from './NodeShell'
export { FAMILIES, TYPE_FAMILY, familyOf } from './NodeShell'
