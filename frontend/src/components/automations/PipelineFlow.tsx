// Pipeline automation creation flow.
// Steps: asset class + universe → ordered filter stages → final execution action → trigger → arm.

import { useState, useCallback } from 'react'
import { useQuery, useMutation } from '@tanstack/react-query'
import { strategiesApi, api } from '@/lib/api'
import { useModeStore } from '@/store/mode'
import { cn } from '@/lib/utils'
import { Plus, X, ChevronDown } from 'lucide-react'
import { TriggerStep, DEFAULT_TRIGGER, type TriggerSpec } from './TriggerStep'

const ASSET_CLASSES = [
  { value: 'crypto_spot_cex', label: 'Crypto Spot (CEX)' },
  { value: 'equity', label: 'Equity' },
  { value: 'fx', label: 'FX' },
  { value: 'prediction_market', label: 'Prediction Market' },
  { value: 'option', label: 'Options' },
  { value: 'crypto_spot_dex', label: 'DEX/AMM' },
  { value: 'perpetual_swap', label: 'Perpetual Swap' },
  { value: 'futures_expiring', label: 'Futures (Expiring)' },
]

interface FilterStage {
  stage_id: string
  strategy_id: string
  label: string
}

interface PipelineFlowProps {
  onArmed?: () => void
}

export function PipelineFlow({ onArmed }: PipelineFlowProps) {
  const { mode } = useModeStore()
  const [assetClass, setAssetClass] = useState('')
  const [universeInput, setUniverseInput] = useState('')
  const [stages, setStages] = useState<FilterStage[]>([])
  const [execStrategyId, setExecStrategyId] = useState('')
  const [newStageStrategyId, setNewStageStrategyId] = useState('')
  const [trigger, setTrigger] = useState<TriggerSpec>(DEFAULT_TRIGGER)

  const { data: strategiesResp } = useQuery({
    queryKey: ['strategies', 'apply-list', assetClass],
    queryFn: () => strategiesApi.list().then((r) => r.data),
    enabled: !!assetClass,
  })

  const allStrategies = (
    Array.isArray(strategiesResp)
      ? strategiesResp
      : (strategiesResp as { strategies?: unknown[] })?.strategies ?? []
  ) as Array<{ id: string; strategy_id: string; strategy_kind?: string }>

  const discoveryStrategies = allStrategies.filter(
    (s) => !s.strategy_kind || s.strategy_kind === 'discovery',
  )
  const executionStrategies = allStrategies.filter(
    (s) => s.strategy_kind === 'execution',
  )

  const universe = universeInput
    .split(/[\n,]+/)
    .map((s) => s.trim())
    .filter(Boolean)

  const addStage = useCallback(() => {
    if (!newStageStrategyId) return
    const strat = discoveryStrategies.find((s) => s.id === newStageStrategyId)
    if (!strat) return
    const stageId = `stage_${Date.now()}`
    setStages((prev) => [
      ...prev,
      { stage_id: stageId, strategy_id: strat.id, label: strat.strategy_id },
    ])
    setNewStageStrategyId('')
  }, [newStageStrategyId, discoveryStrategies])

  const removeStage = useCallback((id: string) => {
    setStages((prev) => prev.filter((s) => s.stage_id !== id))
  }, [])

  const mutation = useMutation({
    mutationFn: () =>
      api.post('/api/automations', {
        kind: 'pipeline',
        account_mode: mode.toLowerCase(),
        spec: {
          asset_class: assetClass,
          universe,
          stages: stages.map((s) => ({
            stage_id: s.stage_id,
            strategy_id: s.strategy_id,
            label: s.label,
          })),
          execution_action: {
            execution_strategy_id: execStrategyId,
          },
          trigger,
        },
        armed: true,
      }),
    onSuccess: () => onArmed?.(),
  })

  const canArm = !!assetClass && universe.length > 0 && stages.length > 0 && !!execStrategyId

  return (
    <div className="space-y-5 p-4 max-w-lg">
      {/* Asset class */}
      <div>
        <label className="block text-xs font-semibold text-text-muted uppercase tracking-wider mb-2">
          Asset class
        </label>
        <div className="relative">
          <select
            value={assetClass}
            onChange={(e) => setAssetClass(e.target.value)}
            className="w-full appearance-none rounded-lg px-3 py-2 pr-8 text-sm bg-surface-2 border border-border text-text focus:outline-none focus:ring-1 focus:ring-accent"
          >
            <option value="">Select asset class…</option>
            {ASSET_CLASSES.map((ac) => (
              <option key={ac.value} value={ac.value}>{ac.label}</option>
            ))}
          </select>
          <ChevronDown className="pointer-events-none absolute right-2.5 top-1/2 -translate-y-1/2 h-3.5 w-3.5 text-text-dim" />
        </div>
      </div>

      {/* Universe */}
      <div>
        <label className="block text-xs font-semibold text-text-muted uppercase tracking-wider mb-2">
          Universe (one instrument per line or comma-separated)
        </label>
        <textarea
          value={universeInput}
          onChange={(e) => setUniverseInput(e.target.value)}
          placeholder="BTC-USD&#10;ETH-USD&#10;SOL-USD"
          rows={4}
          className="w-full rounded-lg px-3 py-2 text-sm bg-surface-2 border border-border text-text placeholder:text-text-dim focus:outline-none focus:ring-1 focus:ring-accent resize-none font-mono"
        />
        <p className="mt-1 text-xs text-text-dim">{universe.length} instruments</p>
      </div>

      {/* Filter stages */}
      <div>
        <label className="block text-xs font-semibold text-text-muted uppercase tracking-wider mb-2">
          Filter stages (ordered)
        </label>
        <div className="space-y-2">
          {stages.map((stage, i) => (
            <div
              key={stage.stage_id}
              className="flex items-center gap-2 rounded-lg border border-border bg-surface-2 px-3 py-2"
            >
              <span className="w-5 text-xs text-text-dim font-mono">{i + 1}.</span>
              <span className="flex-1 text-sm text-text truncate">{stage.label}</span>
              <button
                onClick={() => removeStage(stage.stage_id)}
                className="rounded p-0.5 text-text-dim hover:text-red-400 transition-colors"
              >
                <X className="h-3.5 w-3.5" />
              </button>
            </div>
          ))}
        </div>

        {/* Add stage */}
        <div className="flex gap-2 mt-2">
          <div className="relative flex-1">
            <select
              value={newStageStrategyId}
              onChange={(e) => setNewStageStrategyId(e.target.value)}
              disabled={!assetClass}
              className="w-full appearance-none rounded-lg px-3 py-1.5 pr-8 text-sm bg-surface-2 border border-border text-text focus:outline-none focus:ring-1 focus:ring-accent disabled:opacity-50"
            >
              <option value="">Add discovery stage…</option>
              {discoveryStrategies.map((s) => (
                <option key={s.id} value={s.id}>{s.strategy_id}</option>
              ))}
            </select>
            <ChevronDown className="pointer-events-none absolute right-2.5 top-1/2 -translate-y-1/2 h-3.5 w-3.5 text-text-dim" />
          </div>
          <button
            onClick={addStage}
            disabled={!newStageStrategyId}
            className="flex items-center gap-1 rounded-lg px-3 py-1.5 text-sm border border-border text-text-muted hover:text-text hover:bg-border disabled:opacity-40 transition-colors"
          >
            <Plus className="h-3.5 w-3.5" />
            Add
          </button>
        </div>
      </div>

      {/* Final execution action */}
      <div>
        <label className="block text-xs font-semibold text-text-muted uppercase tracking-wider mb-2">
          Final execution action
        </label>
        <div className="relative">
          <select
            value={execStrategyId}
            onChange={(e) => setExecStrategyId(e.target.value)}
            disabled={!assetClass}
            className="w-full appearance-none rounded-lg px-3 py-2 pr-8 text-sm bg-surface-2 border border-border text-text focus:outline-none focus:ring-1 focus:ring-accent disabled:opacity-50"
          >
            <option value="">Select execution strategy…</option>
            {executionStrategies.map((s) => (
              <option key={s.id} value={s.id}>{s.strategy_id}</option>
            ))}
          </select>
          <ChevronDown className="pointer-events-none absolute right-2.5 top-1/2 -translate-y-1/2 h-3.5 w-3.5 text-text-dim" />
        </div>
      </div>

      {/* Trigger */}
      <div>
        <label className="block text-xs font-semibold text-text-muted uppercase tracking-wider mb-2">
          Trigger
        </label>
        <TriggerStep value={trigger} onChange={setTrigger} />
      </div>

      {mutation.isError && (
        <p className="text-xs text-red-400">Failed to arm automation.</p>
      )}

      <button
        disabled={!canArm || mutation.isPending}
        onClick={() => mutation.mutate()}
        className={cn(
          'w-full rounded-lg py-2 text-sm font-semibold transition-colors',
          'bg-accent text-white hover:bg-accent/80 disabled:opacity-40',
        )}
      >
        {mutation.isPending ? 'Arming…' : 'Arm pipeline'}
      </button>
    </div>
  )
}
