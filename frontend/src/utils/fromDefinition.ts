// Reconstructs a visual ReactFlow graph from a saved v1.0 StrategyDefinition.
// The v1.0 format is lossy (exit rules and AI-forecast nodes are not stored),
// so the reconstituted canvas is a best-effort representation good for editing
// and re-saving.

import type { Node, Edge } from '@xyflow/react'
import type { StrategyDefinition } from './toDefinition'

// ── expression parser ─────────────────────────────────────────────────────────

interface Operand {
  kind: 'feature' | 'price' | 'value'
  featureId?: string
  value?: number
}

function parseOperand(token: string): Operand {
  const fMatch = token.match(/^feature\(['"](.+?)['"]\)$/)
  if (fMatch) return { kind: 'feature', featureId: fMatch[1] }
  if (token === "bar('close')" || token === 'close') return { kind: 'price' }
  const num = parseFloat(token)
  if (!isNaN(num)) return { kind: 'value', value: num }
  return { kind: 'price' }
}

function parseCondExpr(expr: string): {
  left: Operand
  op: '>' | '<'
  right: Operand
} | null {
  const gtIdx = expr.indexOf(' > ')
  const ltIdx = expr.indexOf(' < ')
  if (gtIdx !== -1) {
    return {
      left: parseOperand(expr.slice(0, gtIdx).trim()),
      op: '>',
      right: parseOperand(expr.slice(gtIdx + 3).trim()),
    }
  }
  if (ltIdx !== -1) {
    return {
      left: parseOperand(expr.slice(0, ltIdx).trim()),
      op: '<',
      right: parseOperand(expr.slice(ltIdx + 3).trim()),
    }
  }
  return null
}

// Parse a feature name like "ema_7", "rsi_14", "sma_50" into kind + period.
function parseFeatureName(name: string): { kind: string; period: number } {
  const parts = name.split('_')
  const periodStr = parts[parts.length - 1]
  const period = parseInt(periodStr, 10)
  const kind = parts.slice(0, -1).join('_')
  return { kind: kind || 'ema', period: isNaN(period) ? 14 : period }
}

// ── main export ───────────────────────────────────────────────────────────────

let _idSeed = 1000
const uid = () => `r-${_idSeed++}`

export function fromDefinition(def: StrategyDefinition): {
  nodes: Node[]
  edges: Edge[]
  name: string
} {
  const nodes: Node[] = []
  const edges: Edge[] = []

  // Collect all feature names referenced by condition expressions + inputs.
  const featureIds = new Set<string>()
  for (const inp of def.inputs) {
    if (inp.features) inp.features.forEach((f) => featureIds.add(f))
  }
  // Also scan condition expressions in case inputs are sparse.
  for (const n of def.nodes) {
    if (n.type !== 'condition') continue
    const parsed = parseCondExpr(n.expr)
    if (!parsed) continue
    if (parsed.left.kind === 'feature' && parsed.left.featureId) featureIds.add(parsed.left.featureId)
    if (parsed.right.kind === 'feature' && parsed.right.featureId) featureIds.add(parsed.right.featureId)
  }

  // Create IndicatorNodes — one per unique feature, stacked vertically on the left.
  const featureNodeId = new Map<string, string>() // featureId → reactflow nodeId
  let indY = 60
  for (const featureId of featureIds) {
    const { kind, period } = parseFeatureName(featureId)
    const nodeId = uid()
    featureNodeId.set(featureId, nodeId)
    nodes.push({
      id: nodeId,
      type: 'indicator',
      position: { x: 60, y: indY },
      data: { indicatorId: featureId, kind, period },
    })
    indY += 140
  }

  // Build signal→emit lookup so we know which signal fires on which condition.
  const condEmit = new Map<string, string>() // condId → emit name
  for (const n of def.nodes) {
    if (n.type === 'signal') condEmit.set(n.when, n.emit)
  }

  // Create ConditionNodes.
  const condNodeId = new Map<string, string>() // def condId → reactflow nodeId
  let condY = 60
  for (const n of def.nodes) {
    if (n.type !== 'condition') continue
    const parsed = parseCondExpr(n.expr)
    if (!parsed) continue

    const nodeId = uid()
    condNodeId.set(n.id, nodeId)

    const conditionType = parsed.op === '>' ? 'greater_than' : 'less_than'
    const rightMode = parsed.right.kind === 'feature' ? 'indicator' : 'value'

    nodes.push({
      id: nodeId,
      type: 'condition',
      position: { x: 360, y: condY },
      data: {
        conditionType,
        rightMode,
        rightValue: parsed.right.value ?? 0,
      },
    })

    // Wire left operand
    if (parsed.left.kind === 'feature' && parsed.left.featureId) {
      const src = featureNodeId.get(parsed.left.featureId)
      if (src) {
        edges.push({
          id: uid(),
          source: src,
          sourceHandle: 'value-out',
          target: nodeId,
          targetHandle: 'left-in',
        })
      }
    }

    // Wire right operand (if it's an indicator)
    if (parsed.right.kind === 'feature' && parsed.right.featureId) {
      const src = featureNodeId.get(parsed.right.featureId)
      if (src) {
        edges.push({
          id: uid(),
          source: src,
          sourceHandle: 'value-out',
          target: nodeId,
          targetHandle: 'right-in',
        })
      }
    }

    condY += 180
  }

  // Create AIForecastNodes from v1.1 model_forecast nodes.
  const modelNodeId = new Map<string, string>() // def node id → reactflow nodeId
  let modelY = condY
  for (const n of def.nodes) {
    if (n.type !== 'model_forecast') continue
    const nodeId = uid()
    modelNodeId.set(n.id, nodeId)
    nodes.push({
      id: nodeId,
      type: 'ai_forecast',
      position: { x: 360, y: modelY },
      data: {
        model: n.model_ref,
        direction: (n.direction as 'bullish' | 'bearish' | 'any') ?? 'any',
        minConfidence: n.min_confidence ?? 0,
        alias: n.alias ?? 'production',
      },
    })
    modelY += 180
  }

  // Create ActionNode + SizeNode for each action.
  const actionCenterY = Math.max(condY / 2, 150)
  let actOffset = 0
  for (const action of def.actions) {
    if (action.type !== 'place_order') continue

    const actId = uid()
    nodes.push({
      id: actId,
      type: 'action',
      position: { x: 660, y: actionCenterY + actOffset },
      data: { side: action.order.side },
    })

    // Wire conditions whose signals trigger this action.
    for (const [defCondId, rfNodeId] of condNodeId) {
      if (condEmit.get(defCondId) === action.on_signal) {
        edges.push({
          id: uid(),
          source: rfNodeId,
          sourceHandle: 'cond-out',
          target: actId,
          targetHandle: 'action-in',
          animated: true,
        })
      }
    }

    // Wire AI model nodes whose signals trigger this action.
    for (const [defModelId, rfNodeId] of modelNodeId) {
      if (condEmit.get(defModelId) === action.on_signal) {
        edges.push({
          id: uid(),
          source: rfNodeId,
          sourceHandle: 'forecast-out',
          target: actId,
          targetHandle: 'action-in',
          animated: true,
        })
      }
    }

    // Size node
    const sizeType =
      action.order.size_mode === 'percent_of_balance' ? 'percent_of_equity' : 'fixed'
    const sizeId = uid()
    nodes.push({
      id: sizeId,
      type: 'size',
      position: { x: 940, y: actionCenterY + actOffset - 80 },
      data: { sizeType, value: parseFloat(action.order.size) || 0.02 },
    })
    edges.push({
      id: uid(),
      source: actId,
      sourceHandle: 'size-out',
      target: sizeId,
      targetHandle: 'size-in',
    })

    actOffset += 220
  }

  // Derive a readable display name from the strategy_id slug.
  const name = def.strategy_id
    .replace(/_/g, ' ')
    .replace(/\b\w/g, (c) => c.toUpperCase())

  return { nodes, edges, name }
}
