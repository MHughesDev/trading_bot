// AI Inference block — rebuilt from scratch.
//
// One block runs incoming market data through an inference *target* (a model)
// and produces a single forecast that gates entries (fires when the forecast
// direction matches and confidence ≥ the threshold). The target resolves to a
// forecast through the inference gateway.
//
// The block also declares the *input-data contract* — which feature set, at
// what timeframe, and how many bars of lookback — so automations, scanners, and
// backtests each feed the target a window it actually supports. When a model is
// chosen we read its feature-vector contract to auto-fill the feature set and a
// sensible default lookback, and surface what the model expects.

import { Handle, Position, useReactFlow } from '@xyflow/react'
import type { NodeProps, Node } from '@xyflow/react'
import { useEffect } from 'react'
import { useQuery } from '@tanstack/react-query'
import type { ForecastDirection, InferenceTargetKind } from '@/types/spec'
import { INFERENCE_TARGET_LABELS } from '@/types/spec'
import { modelsApi, inferenceTargetsApi } from '@/api/models'

// A plain type literal (not an interface intersection) so the data satisfies
// ReactFlow's `Record<string, unknown>` node-data constraint. Mirrors the
// `AiInputContract` fields (featureSet/timeframe/lookback) inline.
export type AIInferenceNodeData = {
  targetKind: InferenceTargetKind
  /** Model slug. */
  targetRef: string
  alias: string
  direction: ForecastDirection
  minConfidence: number
  /** Feature set the target consumes (auto-filled from the target). */
  featureSet?: string
  /** Timeframe of the bars/features fed in. */
  timeframe: string
  /** Lookback window — number of bars provided each run. */
  lookback: number
  disabled?: boolean
}

export type AIInferenceNodeType = Node<AIInferenceNodeData, 'ai_inference'>

const TARGET_KINDS: InferenceTargetKind[] = ['model']

const TIMEFRAMES = ['1m', '5m', '15m', '1h', '4h', '1d'] as const

export function AIInferenceNode({ data, id, selected }: NodeProps<AIInferenceNodeType>) {
  const { updateNodeData } = useReactFlow()
  const set = (patch: Partial<AIInferenceNodeData>) => updateNodeData(id, patch)

  // ── Target options per kind ────────────────────────────────────────────────
  const { data: models } = useQuery({
    queryKey: ['inference-targets', 'model'],
    queryFn: () => inferenceTargetsApi.models().then((r) => r.data.models),
    enabled: data.targetKind === 'model',
    staleTime: 30_000,
  })

  // ── Model input contract (feature set + bars the model expects) ─────────────
  const selectedModelId =
    data.targetKind === 'model'
      ? models?.find((m) => m.slug === data.targetRef)?.id
      : undefined

  const { data: contract } = useQuery({
    queryKey: ['model-contract', selectedModelId],
    queryFn: () => modelsApi.featureVector(selectedModelId!).then((r) => r.data),
    enabled: !!selectedModelId,
    staleTime: 60_000,
  })

  // Auto-fill the input contract from the chosen model's declared requirement so
  // the block defaults to a window the model supports.
  useEffect(() => {
    if (!contract) return
    const patch: Partial<AIInferenceNodeData> = {}
    if (contract.feature_set && data.featureSet !== contract.feature_set) {
      patch.featureSet = contract.feature_set
    }
    if (contract.timeframe && data.timeframe !== contract.timeframe) {
      patch.timeframe = contract.timeframe
    }
    if (contract.bars_used && contract.bars_used > data.lookback) {
      patch.lookback = contract.bars_used
    }
    if (Object.keys(patch).length > 0) set(patch)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [contract])

  const minLookback = contract?.bars_used
  const lookbackTooSmall = minLookback != null && data.lookback < minLookback

  const switchKind = (kind: InferenceTargetKind) => {
    // Changing target kind clears the ref + the model-derived feature set.
    set({ targetKind: kind, targetRef: '', featureSet: undefined })
  }

  return (
    <div className={`tb-node${selected ? ' selected' : ''}${data.disabled ? ' disabled' : ''}`} style={{ minWidth: 250 }}>
      <div className="tb-node-header" style={{ background: '#0E7490' }}>
        <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5"><path d="M12 2a10 10 0 1 0 0 20 10 10 0 0 0 0-20z"/><path d="M12 6v6l4 2"/></svg>
        AI Inference
      </div>
      <div className="tb-node-body">
        {/* Target kind */}
        <div className="tb-node-row">
          <span className="tb-node-label">Run through</span>
          <select className="tb-select" value={data.targetKind} onChange={(e) => switchKind(e.target.value as InferenceTargetKind)}>
            {TARGET_KINDS.map((k) => (
              <option key={k} value={k}>{INFERENCE_TARGET_LABELS[k]}</option>
            ))}
          </select>
        </div>

        {/* Target picker */}
        <div className="tb-node-row">
          <span className="tb-node-label">{INFERENCE_TARGET_LABELS[data.targetKind]}</span>
          {data.targetKind === 'model' && (
            <select className="tb-select" value={data.targetRef} onChange={(e) => set({ targetRef: e.target.value, featureSet: undefined })}>
              <option value="" disabled>Select a model…</option>
              {(models ?? []).map((m) => (
                <option key={m.id} value={m.slug} disabled={m.status !== 'active' || !m.has_production}>
                  {m.display_name}{(m.status !== 'active' || !m.has_production) ? ' (unavailable)' : ''}
                </option>
              ))}
            </select>
          )}
        </div>

        {/* Alias */}
        <div className="tb-node-row">
          <span className="tb-node-label">Version</span>
          <select className="tb-select" value={data.alias} onChange={(e) => set({ alias: e.target.value })}>
            <option value="production">Production</option>
            <option value="candidate">Candidate</option>
          </select>
        </div>

        <div className="tb-node-divider" />

        {/* Input-data contract */}
        <div className="tb-node-row">
          <span className="tb-node-label">Features</span>
          <span className="tb-node-readonly" title="Feature set the target consumes">
            {data.featureSet ?? (data.targetKind === 'model' ? '— pick a model —' : 'target default')}
          </span>
        </div>
        <div className="tb-node-row">
          <span className="tb-node-label">Timeframe</span>
          <select className="tb-select" value={data.timeframe} onChange={(e) => set({ timeframe: e.target.value })}>
            {TIMEFRAMES.map((t) => <option key={t} value={t}>{t}</option>)}
          </select>
        </div>
        <div className="tb-node-row">
          <span className="tb-node-label">Lookback</span>
          <input
            className="tb-input"
            type="number"
            min={1}
            step={1}
            value={data.lookback}
            onChange={(e) => set({ lookback: Math.max(1, parseInt(e.target.value) || 1) })}
          />
          <span className="tb-unit">bars</span>
        </div>
        {lookbackTooSmall && (
          <div className="tb-node-warn">
            Model needs ≥ {minLookback} bars.
          </div>
        )}

        <div className="tb-node-divider" />

        {/* Output gating */}
        <div className="tb-node-row">
          <span className="tb-node-label">Expects</span>
          <select className="tb-select" value={data.direction} onChange={(e) => set({ direction: e.target.value as ForecastDirection })}>
            <option value="bullish">Bullish (price up)</option>
            <option value="bearish">Bearish (price down)</option>
            <option value="any">Any direction</option>
          </select>
        </div>
        <div className="tb-node-row">
          <span className="tb-node-label">Min conf.</span>
          <input
            className="tb-input"
            type="number"
            min={0}
            max={1}
            step={0.05}
            value={data.minConfidence}
            onChange={(e) => set({ minConfidence: Math.min(1, Math.max(0, parseFloat(e.target.value) || 0)) })}
          />
          <span className="tb-unit">0–1</span>
        </div>
      </div>
      <Handle type="source" position={Position.Left} id="forecast-out-l" />
      <Handle type="source" position={Position.Right} id="forecast-out" />
    </div>
  )
}
