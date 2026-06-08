/**
 * BuilderCanvas — n8n-style node graph editor for v1.0 strategy definitions.
 *
 * Serializes to/from the canonical StrategyDefinition JSON via serialize.ts,
 * then submits via POST /api/strategies. Validation errors returned by the
 * API are displayed inline so the user can fix them.
 *
 * Three front doors, one room: whatever is built here round-trips through the
 * same validator and runtime as the JSON API and MCP server.
 */

import { useCallback, useState } from 'react'
import {
  ReactFlow,
  Background,
  BackgroundVariant,
  Controls,
  MiniMap,
  addEdge,
  useNodesState,
  useEdgesState,
  Handle,
  Position,
} from '@xyflow/react'
import type { Node, Edge, NodeTypes, Connection } from '@xyflow/react'
import '@xyflow/react/dist/style.css'

import { serialize, deserialize } from './serialize'
import type { ConditionNodeData, SignalNodeData, ActionNodeData, StrategyDefinition } from './serialize'

// ── v1.0 node components ─────────────────────────────────────────────────────

function ConditionNodeV1({ id, data }: { id: string; data: ConditionNodeData }) {
  return (
    <div style={{ background: '#fef9c3', border: '1px solid #ca8a04', borderRadius: 8, padding: '8px 12px', minWidth: 260 }}>
      <div style={{ fontWeight: 600, fontSize: 12, color: '#92400e', marginBottom: 4 }}>CONDITION [{data.nodeId ?? id}]</div>
      <div style={{ fontSize: 11, fontFamily: 'monospace', color: '#1c1917', wordBreak: 'break-all' }}>{data.expr || '(empty)'}</div>
      <Handle type="source" position={Position.Right} id="cond-out" />
    </div>
  )
}

function SignalNodeV1({ id, data }: { id: string; data: SignalNodeData }) {
  return (
    <div style={{ background: '#dbeafe', border: '1px solid #3b82f6', borderRadius: 8, padding: '8px 12px', minWidth: 180 }}>
      <Handle type="target" position={Position.Left} id="signal-in" />
      <div style={{ fontWeight: 600, fontSize: 12, color: '#1e40af', marginBottom: 4 }}>SIGNAL [{data.nodeId ?? id}]</div>
      <div style={{ fontSize: 12, color: '#1e3a8a' }}>emit: <strong>{data.emit || '(none)'}</strong></div>
      <Handle type="source" position={Position.Right} id="signal-out" />
    </div>
  )
}

function ActionNodeV1({ id, data }: { id: string; data: ActionNodeData }) {
  return (
    <div style={{ background: '#dcfce7', border: '1px solid #16a34a', borderRadius: 8, padding: '8px 12px', minWidth: 200 }}>
      <Handle type="target" position={Position.Left} id="action-in" />
      <div style={{ fontWeight: 600, fontSize: 12, color: '#166534', marginBottom: 4 }}>ACTION [{id}]</div>
      <div style={{ fontSize: 12, color: '#14532d' }}>on: <strong>{data.on_signal}</strong></div>
      <div style={{ fontSize: 12, color: '#14532d' }}>{data.side.toUpperCase()} {data.size} ({data.size_mode})</div>
    </div>
  )
}

const nodeTypes: NodeTypes = {
  condition_v1: ConditionNodeV1 as unknown as NodeTypes[string],
  signal_v1: SignalNodeV1 as unknown as NodeTypes[string],
  action_v1: ActionNodeV1 as unknown as NodeTypes[string],
}

// ── Initial example graph (EMA cross strategy) ───────────────────────────────

const INITIAL_NODES: Node[] = [
  {
    id: 'flow-cond-n1',
    type: 'condition_v1',
    position: { x: 80, y: 80 },
    data: { nodeId: 'n1', expr: "feature('ema_7') > feature('ema_21')" } satisfies ConditionNodeData,
  },
  {
    id: 'flow-signal-n2',
    type: 'signal_v1',
    position: { x: 420, y: 80 },
    data: { nodeId: 'n2', emit: 'long' } satisfies SignalNodeData,
  },
  {
    id: 'flow-action-0',
    type: 'action_v1',
    position: { x: 720, y: 80 },
    data: { on_signal: 'long', side: 'buy', size_mode: 'fixed', size: '0.01' } satisfies ActionNodeData,
  },
]

const INITIAL_EDGES: Edge[] = [
  { id: 'e1', source: 'flow-cond-n1', sourceHandle: 'cond-out', target: 'flow-signal-n2', targetHandle: 'signal-in' },
  { id: 'e2', source: 'flow-signal-n2', sourceHandle: 'signal-out', target: 'flow-action-0', targetHandle: 'action-in' },
]

// ── BuilderCanvas ─────────────────────────────────────────────────────────────

interface BuilderCanvasProps {
  onSaved?: (def: StrategyDefinition) => void
}

export function BuilderCanvas({ onSaved }: BuilderCanvasProps) {
  const [nodes, , onNodesChange] = useNodesState(INITIAL_NODES)
  const [edges, setEdges, onEdgesChange] = useEdgesState(INITIAL_EDGES)

  const [strategyId, setStrategyId] = useState('ema_cross_v1')
  const [assetClass, setAssetClass] = useState('crypto_spot_cex')
  const [jsonPreview, setJsonPreview] = useState('')
  const [errors, setErrors] = useState<string[]>([])
  const [saving, setSaving] = useState(false)

  const onConnect = useCallback(
    (connection: Connection) => setEdges(eds => addEdge(connection, eds)),
    [setEdges]
  )

  const handlePreview = () => {
    const def = serialize({ strategyId, assetClass, nodes, edges })
    setJsonPreview(JSON.stringify(def, null, 2))
    setErrors([])
  }

  const handleSave = async () => {
    const def = serialize({ strategyId, assetClass, nodes, edges })
    setSaving(true)
    setErrors([])
    try {
      const resp = await fetch('/api/strategies', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json', Authorization: 'Bearer dev' },
        body: JSON.stringify(def),
      })
      const body = await resp.json()
      if (!resp.ok) {
        const errs: string[] = (body.errors ?? []).map(
          (e: { path: string; message: string }) => `${e.path}: ${e.message}`
        )
        setErrors(errs.length > 0 ? errs : [body.error ?? 'Unknown error'])
      } else {
        onSaved?.(def)
      }
    } catch (err) {
      setErrors([String(err)])
    } finally {
      setSaving(false)
    }
  }

  const handleLoadJson = (json: string) => {
    try {
      const def: StrategyDefinition = JSON.parse(json)
      const { nodes: newNodes, edges: newEdges } = deserialize(def)
      // Replace graph with deserialized nodes/edges
      // (useNodesState/useEdgesState don't expose a reset directly — use setter)
      setEdges(newEdges)
      setStrategyId(def.strategy_id)
      setAssetClass(def.asset_class)
      setErrors([])
    } catch {
      setErrors(['Invalid JSON'])
    }
  }

  return (
    <div style={{ display: 'flex', flexDirection: 'column', height: '100%', gap: 8 }}>
      {/* Toolbar */}
      <div style={{ display: 'flex', gap: 8, padding: '8px 12px', background: 'var(--tb-surface-1, #1e1e2e)', borderRadius: 8, alignItems: 'center', flexWrap: 'wrap' }}>
        <label style={{ fontSize: 12, color: 'var(--tb-text-2, #cdd6f4)' }}>
          Strategy ID
          <input
            value={strategyId}
            onChange={e => setStrategyId(e.target.value)}
            style={{ marginLeft: 6, background: 'var(--tb-surface-2, #313244)', border: '1px solid var(--tb-border-1, #45475a)', borderRadius: 4, color: 'inherit', padding: '2px 6px', fontSize: 12 }}
          />
        </label>
        <label style={{ fontSize: 12, color: 'var(--tb-text-2, #cdd6f4)' }}>
          Asset Class
          <input
            value={assetClass}
            onChange={e => setAssetClass(e.target.value)}
            style={{ marginLeft: 6, background: 'var(--tb-surface-2, #313244)', border: '1px solid var(--tb-border-1, #45475a)', borderRadius: 4, color: 'inherit', padding: '2px 6px', fontSize: 12 }}
          />
        </label>
        <button onClick={handlePreview} style={{ fontSize: 12, padding: '3px 10px', borderRadius: 4, border: '1px solid #4b5563', cursor: 'pointer', background: '#374151', color: '#e5e7eb' }}>
          Preview JSON
        </button>
        <button onClick={handleSave} disabled={saving} style={{ fontSize: 12, padding: '3px 10px', borderRadius: 4, border: 'none', cursor: saving ? 'wait' : 'pointer', background: '#15803d', color: '#fff', fontWeight: 600 }}>
          {saving ? 'Saving…' : 'Save Strategy'}
        </button>
      </div>

      {/* Validation errors */}
      {errors.length > 0 && (
        <div style={{ background: '#fef2f2', border: '1px solid #fca5a5', borderRadius: 6, padding: '8px 12px' }}>
          {errors.map((e, i) => <div key={i} style={{ fontSize: 12, color: '#991b1b' }}>{e}</div>)}
        </div>
      )}

      {/* React Flow canvas */}
      <div style={{ flex: 1, border: '1px solid var(--tb-border-1, #45475a)', borderRadius: 8, overflow: 'hidden' }}>
        <ReactFlow
          nodes={nodes}
          edges={edges}
          nodeTypes={nodeTypes}
          onNodesChange={onNodesChange}
          onEdgesChange={onEdgesChange}
          onConnect={onConnect}
          fitView
        >
          <Background variant={BackgroundVariant.Dots} />
          <Controls />
          <MiniMap />
        </ReactFlow>
      </div>

      {/* JSON preview panel */}
      {jsonPreview && (
        <pre style={{ maxHeight: 220, overflow: 'auto', fontSize: 11, fontFamily: 'monospace', background: 'var(--tb-surface-1, #1e1e2e)', color: '#a6e3a1', padding: 12, borderRadius: 8, margin: 0 }}>
          {jsonPreview}
        </pre>
      )}
    </div>
  )
}
