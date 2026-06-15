import type { Node, Edge } from '@xyflow/react'
import type { RuleStrategySpec, IndicatorSpec, Condition, ExitRule, SizeRule } from '@/types/spec'
import type { IndicatorNodeData } from '@/nodes/IndicatorNode'
import type { ConditionNodeData } from '@/nodes/ConditionNode'
import type { AIForecastNodeData } from '@/nodes/AIForecastNode'
import type { LogicNodeData } from '@/nodes/LogicNode'
import type { ActionNodeData } from '@/nodes/ActionNode'
import type { SizeNodeData } from '@/nodes/SizeNode'
import type { ExitNodeData } from '@/nodes/ExitNode'

function byId(nodes: Node[], id: string): Node | undefined {
  return nodes.find(n => n.id === id)
}

function edgesInto(edges: Edge[], nodeId: string, handle?: string): Edge[] {
  return edges.filter(e => e.target === nodeId && (!handle || e.targetHandle === handle))
}

function edgesFrom(edges: Edge[], nodeId: string, handle?: string): Edge[] {
  return edges.filter(e => e.source === nodeId && (!handle || e.sourceHandle === handle))
}

function collectIndicator(node: Node, map: Map<string, IndicatorSpec>): void {
  if (node.type !== 'indicator') return
  const d = node.data as IndicatorNodeData
  const id = d.indicatorId?.trim()
  if (id && !map.has(id)) map.set(id, { id, kind: d.kind, period: d.period })
}

function resolveConditions(
  node: Node,
  nodes: Node[],
  edges: Edge[],
  indicators: Map<string, IndicatorSpec>,
): { allOf: Condition[]; anyOf: Condition[] } {
  if (node.type === 'condition') {
    const d = node.data as ConditionNodeData
    const cond: Condition = { type: d.conditionType, left: 'price' }

    const leftEdges = edgesInto(edges, node.id, 'left-in')
    if (leftEdges.length > 0) {
      const leftNode = byId(nodes, leftEdges[0].source)
      if (leftNode?.type === 'indicator') {
        const indId = (leftNode.data as IndicatorNodeData).indicatorId?.trim()
        if (indId) { cond.left = indId; collectIndicator(leftNode, indicators) }
      }
    }

    const unary = ['rising', 'falling'].includes(d.conditionType)
    if (!unary) {
      if (d.rightMode === 'indicator') {
        const rightEdges = edgesInto(edges, node.id, 'right-in')
        if (rightEdges.length > 0) {
          const rightNode = byId(nodes, rightEdges[0].source)
          if (rightNode?.type === 'indicator') {
            const indId = (rightNode.data as IndicatorNodeData).indicatorId?.trim()
            if (indId) { cond.right_id = indId; collectIndicator(rightNode, indicators) }
          }
        }
      } else {
        cond.right_value = d.rightValue
      }
    }

    return { allOf: [cond], anyOf: [] }
  }

  if (node.type === 'ai_forecast') {
    const d = node.data as AIForecastNodeData
    const cond: Condition = { type: 'model_forecast', left: 'price', right_value: d.minConfidence }
    ;(cond as unknown as Record<string, unknown>).model = d.model
    ;(cond as unknown as Record<string, unknown>).direction = d.direction
    return { allOf: [cond], anyOf: [] }
  }

  if (node.type === 'logic') {
    const d = node.data as LogicNodeData
    const allOf: Condition[] = []
    const anyOf: Condition[] = []
    edgesInto(edges, node.id).forEach(e => {
      const src = byId(nodes, e.source)
      if (!src) return
      const { allOf: a, anyOf: b } = resolveConditions(src, nodes, edges, indicators)
      if (d.op === 'and') { allOf.push(...a, ...b) } else { anyOf.push(...a, ...b) }
    })
    return { allOf, anyOf }
  }

  return { allOf: [], anyOf: [] }
}

export interface CompileResult {
  spec: RuleStrategySpec | null
  errors: string[]
  warnings: string[]
}

export interface ScannerCompileResult {
  indicators: IndicatorSpec[]
  allOf: Condition[]
  anyOf: Condition[]
  errors: string[]
  warnings: string[]
}

/** Compile a Discovery (scanner) strategy — no action/size/exit nodes required. */
export function compileScanner(nodes: Node[], edges: Edge[], name: string): ScannerCompileResult {
  const errors: string[] = []
  const warnings: string[] = []
  const indicators = new Map<string, IndicatorSpec>()
  const allOf: Condition[] = []
  const anyOf: Condition[] = []

  // Root signal nodes = condition/logic/ai_forecast nodes whose output does NOT
  // feed into another condition/logic node.  These are the ones that would
  // normally wire into a Trade Action node.
  const condOrLogicIds = new Set(
    nodes
      .filter(n => n.type === 'condition' || n.type === 'logic' || n.type === 'ai_forecast')
      .map(n => n.id),
  )
  const sourcesOfCondEdges = new Set(
    edges
      .filter(e => condOrLogicIds.has(e.source) && condOrLogicIds.has(e.target))
      .map(e => e.source),
  )
  const rootNodes = nodes.filter(n => condOrLogicIds.has(n.id) && !sourcesOfCondEdges.has(n.id))

  if (rootNodes.length === 0) {
    errors.push('Add at least one Condition node to your scanner strategy.')
  }

  rootNodes.forEach(node => {
    const { allOf: a, anyOf: b } = resolveConditions(node, nodes, edges, indicators)
    allOf.push(...a)
    anyOf.push(...b)
  })

  if ([...allOf, ...anyOf].some(c => c.type === 'model_forecast')) {
    warnings.push('AI Forecast conditions require the forecaster service to be running.')
  }

  if (!name.trim()) errors.push('Give your scanner strategy a name.')

  return {
    indicators: Array.from(indicators.values()),
    allOf,
    anyOf,
    errors,
    warnings,
  }
}

export function compile(nodes: Node[], edges: Edge[], name: string): CompileResult {
  const errors: string[] = []
  const warnings: string[] = []

  const actionNode = nodes.find(n => n.type === 'action')
  if (!actionNode) return { spec: null, errors: ['Add a Trade Action node to your strategy.'], warnings }

  const actionData = actionNode.data as ActionNodeData

  const condInEdges = edgesInto(edges, actionNode.id, 'action-in')
  if (condInEdges.length === 0)
    errors.push('Connect at least one Condition (or Logic) node to the Trade Action input.')

  const indicators = new Map<string, IndicatorSpec>()
  const allOf: Condition[] = []
  const anyOf: Condition[] = []

  condInEdges.forEach(e => {
    const src = byId(nodes, e.source)
    if (!src) return
    const { allOf: a, anyOf: b } = resolveConditions(src, nodes, edges, indicators)
    allOf.push(...a); anyOf.push(...b)
  })

  if ([...allOf, ...anyOf].some(c => c.type === 'model_forecast'))
    warnings.push('AI Forecast conditions require the forecaster service to be running.')

  const sizeEdges = edgesFrom(edges, actionNode.id, 'size-out')
  if (sizeEdges.length === 0)
    errors.push('Connect a Position Size node to the size handle on the Trade Action.')

  let sizeSpec: SizeRule = { type: 'percent_of_equity', value: 0.02 }
  if (sizeEdges.length > 0) {
    const sn = byId(nodes, sizeEdges[0].target)
    if (sn?.type === 'size') {
      const d = sn.data as SizeNodeData
      sizeSpec = { type: d.sizeType, value: d.value }
    }
  }

  const exitEdges = edgesFrom(edges, actionNode.id, 'exit-out')
  if (exitEdges.length === 0)
    errors.push('Connect at least one Exit Rule node to the exits handle on the Trade Action.')

  const exits: ExitRule[] = exitEdges
    .map(e => byId(nodes, e.target))
    .filter((n): n is Node => n?.type === 'exit')
    .map(n => { const d = n.data as ExitNodeData; return { type: d.exitType, value: d.value } })

  if (!name.trim()) errors.push('Give your strategy a name.')

  const spec: RuleStrategySpec = {
    name: name.trim() || 'Untitled',
    indicators: Array.from(indicators.values()),
    entry: { side: actionData.side, all: allOf, any: anyOf },
    size: sizeSpec,
    exits,
  }

  return { spec, errors, warnings }
}
