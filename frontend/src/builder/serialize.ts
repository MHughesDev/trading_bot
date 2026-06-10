/**
 * serialize.ts — bidirectional mapping between the v1.0 canonical strategy
 * definition JSON and a React Flow node graph.
 *
 * All three front doors (visual builder, JSON API, MCP server) produce and
 * consume the same StrategyDefinition format. This module is the visual
 * builder's side of that contract.
 */

import type { Node, Edge } from '@xyflow/react'

// ── Canonical v1.0 types (mirrors domain::strategy_def) ──────────────────────

export interface InputDeclaration {
  lane: string
  instrument: string
  features?: string[]
}

export interface ConditionNode {
  id: string
  type: 'condition'
  expr: string
}

export interface SignalNode {
  id: string
  type: 'signal'
  when: string
  emit: string
}

export type StrategyNode = ConditionNode | SignalNode

export interface OrderSpec {
  side: 'buy' | 'sell'
  size_mode: 'fixed' | 'percent_of_balance' | 'risk_unit'
  size: string
}

export interface StrategyAction {
  on_signal: string
  type: 'place_order'
  order: OrderSpec
}

export interface RiskOverrides {
  max_position?: string
  max_order_rate_per_minute?: number
  max_order_rate_per_second?: number
}

export interface StrategyDefinition {
  strategy_id: string
  definition_version: '1.0'
  asset_class: string
  min_trust_tier?: string
  inputs: InputDeclaration[]
  nodes: StrategyNode[]
  actions: StrategyAction[]
  risk_overrides?: RiskOverrides
}

// ── React Flow node data types ────────────────────────────────────────────────

export interface ConditionNodeData {
  nodeId: string
  expr: string
}

export interface SignalNodeData {
  nodeId: string
  emit: string
}

export interface ActionNodeData {
  on_signal: string
  side: 'buy' | 'sell'
  size_mode: 'fixed' | 'percent_of_balance' | 'risk_unit'
  size: string
}

// ── Serialize: React Flow graph → StrategyDefinition ─────────────────────────

export interface SerializeInput {
  strategyId: string
  assetClass: string
  nodes: Node[]
  edges: Edge[]
}

/** Extract all `feature('name')` identifiers from an expression string. */
function extractFeatureNames(expr: string): string[] {
  const re = /feature\(\s*'([^']+)'\s*\)/g
  const names: string[] = []
  let match: RegExpExecArray | null
  while ((match = re.exec(expr)) !== null) {
    if (!names.includes(match[1])) names.push(match[1])
  }
  return names
}

/** Serialize a React Flow v1.0 builder graph to canonical StrategyDefinition JSON. */
export function serialize(input: SerializeInput): StrategyDefinition {
  const { strategyId, assetClass, nodes, edges } = input

  const conditionNodes = nodes.filter(n => n.type === 'condition_v1')
  const signalNodes = nodes.filter(n => n.type === 'signal_v1')
  const actionNodes = nodes.filter(n => n.type === 'action_v1')

  // Build the node graph.
  const strategyNodes: StrategyNode[] = []
  for (const n of conditionNodes) {
    const d = n.data as unknown as ConditionNodeData
    strategyNodes.push({ id: d.nodeId ?? n.id, type: 'condition', expr: d.expr })
  }

  // Signal nodes get their `when` from the edge connecting them to a condition.
  for (const n of signalNodes) {
    const d = n.data as unknown as SignalNodeData
    const inEdge = edges.find(e => e.target === n.id)
    const condNode = inEdge ? conditionNodes.find(c => c.id === inEdge.source) : null
    const when = condNode ? ((condNode.data as unknown as ConditionNodeData).nodeId ?? condNode.id) : ''
    strategyNodes.push({ id: d.nodeId ?? n.id, type: 'signal', when, emit: d.emit })
  }

  // Actions.
  const actions: StrategyAction[] = actionNodes.map(n => {
    const d = n.data as unknown as ActionNodeData
    return {
      on_signal: d.on_signal,
      type: 'place_order',
      order: { side: d.side, size_mode: d.size_mode, size: d.size },
    }
  })

  // Derive inputs: always include market.bars.1m, plus features.technical if any
  // condition uses feature('…').
  const allFeatures = conditionNodes.flatMap(n =>
    extractFeatureNames((n.data as unknown as ConditionNodeData).expr)
  )
  const inputs: InputDeclaration[] = [
    { lane: 'market.bars.1m', instrument: '$bound_at_init' },
  ]
  if (allFeatures.length > 0) {
    inputs.push({
      lane: 'features.technical',
      instrument: '$bound_at_init',
      features: allFeatures,
    })
  }

  return {
    strategy_id: strategyId,
    definition_version: '1.0',
    asset_class: assetClass,
    inputs,
    nodes: strategyNodes,
    actions,
  }
}

// ── Deserialize: StrategyDefinition → React Flow graph ───────────────────────

export interface DeserializeResult {
  nodes: Node[]
  edges: Edge[]
}

/** Auto-layout: column positions for a left-to-right flow. */
const COL_X = { condition: 80, signal: 380, action: 680 }
const ROW_GAP = 160

/** Deserialize a canonical StrategyDefinition back into a React Flow graph. */
export function deserialize(def: StrategyDefinition): DeserializeResult {
  const nodes: Node[] = []
  const edges: Edge[] = []

  let condRow = 0
  let signalRow = 0
  let actionRow = 0

  const condIdToFlowId = new Map<string, string>()

  for (const sn of def.nodes) {
    if (sn.type === 'condition') {
      const flowId = `flow-cond-${sn.id}`
      condIdToFlowId.set(sn.id, flowId)
      nodes.push({
        id: flowId,
        type: 'condition_v1',
        position: { x: COL_X.condition, y: condRow * ROW_GAP + 60 },
        data: { nodeId: sn.id, expr: sn.expr } satisfies ConditionNodeData,
      })
      condRow++
    } else if (sn.type === 'signal') {
      const flowId = `flow-signal-${sn.id}`
      nodes.push({
        id: flowId,
        type: 'signal_v1',
        position: { x: COL_X.signal, y: signalRow * ROW_GAP + 60 },
        data: { nodeId: sn.id, emit: sn.emit } satisfies SignalNodeData,
      })
      // Edge: condition → signal
      const condFlowId = condIdToFlowId.get(sn.when)
      if (condFlowId) {
        edges.push({
          id: `e-${condFlowId}-${flowId}`,
          source: condFlowId,
          target: flowId,
          sourceHandle: 'cond-out',
          targetHandle: 'signal-in',
        })
      }
      signalRow++
    }
  }

  // Signal flow IDs keyed by emit name.
  const emitToFlowId = new Map<string, string>()
  for (const sn of def.nodes) {
    if (sn.type === 'signal') {
      emitToFlowId.set(sn.emit, `flow-signal-${sn.id}`)
    }
  }

  for (const action of def.actions) {
    const flowId = `flow-action-${actionRow}`
    nodes.push({
      id: flowId,
      type: 'action_v1',
      position: { x: COL_X.action, y: actionRow * ROW_GAP + 60 },
      data: {
        on_signal: action.on_signal,
        side: action.order.side,
        size_mode: action.order.size_mode,
        size: action.order.size,
      } satisfies ActionNodeData,
    })
    const sigFlowId = emitToFlowId.get(action.on_signal)
    if (sigFlowId) {
      edges.push({
        id: `e-${sigFlowId}-${flowId}`,
        source: sigFlowId,
        target: flowId,
        sourceHandle: 'signal-out',
        targetHandle: 'action-in',
      })
    }
    actionRow++
  }

  return { nodes, edges }
}

/** Round-trip: serialize then deserialize should preserve strategy structure. */
export function roundTrip(
  input: SerializeInput
): { definition: StrategyDefinition; graph: DeserializeResult } {
  const definition = serialize(input)
  const graph = deserialize(definition)
  return { definition, graph }
}
