// Single-instrument automation creation flow.
// Steps: asset class → instrument → execution strategy → time window → arm.

import { useState, useCallback } from 'react'
import { useQuery, useMutation } from '@tanstack/react-query'
import { strategiesApi, api } from '@/lib/api'
import { useModeStore } from '@/store/mode'
import { cn } from '@/lib/utils'
import { ChevronDown } from 'lucide-react'

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

// Whether the asset class trades 24/7 or has sessions.
function is24_7(assetClass: string): boolean {
  return (
    assetClass === 'crypto_spot_cex' ||
    assetClass === 'crypto_spot_dex' ||
    assetClass === 'perpetual_swap' ||
    assetClass === 'prediction_market'
  )
}

interface SelectProps {
  value: string
  onChange: (v: string) => void
  options: Array<{ value: string; label: string }>
  placeholder?: string
}

function Select({ value, onChange, options, placeholder }: SelectProps) {
  return (
    <div className="relative">
      <select
        value={value}
        onChange={(e) => onChange(e.target.value)}
        className={cn(
          'w-full appearance-none rounded-lg px-3 py-2 pr-8 text-sm',
          'bg-surface-2 border border-border text-text',
          'focus:outline-none focus:ring-1 focus:ring-accent',
        )}
      >
        {placeholder && <option value="">{placeholder}</option>}
        {options.map((o) => (
          <option key={o.value} value={o.value}>
            {o.label}
          </option>
        ))}
      </select>
      <ChevronDown className="pointer-events-none absolute right-2.5 top-1/2 -translate-y-1/2 h-3.5 w-3.5 text-text-dim" />
    </div>
  )
}

function StepLabel({ n, children }: { n: number; children: React.ReactNode }) {
  return (
    <div className="flex items-center gap-2 mb-2">
      <div className="flex h-5 w-5 items-center justify-center rounded-full bg-accent/20 text-xs font-bold text-accent shrink-0">
        {n}
      </div>
      <span className="text-xs font-semibold text-text-muted uppercase tracking-wider">
        {children}
      </span>
    </div>
  )
}

interface SingleInstrumentFlowProps {
  onArmed?: () => void
}

export function SingleInstrumentFlow({ onArmed }: SingleInstrumentFlowProps) {
  const { mode } = useModeStore()
  const [assetClass, setAssetClass] = useState('')
  const [instrument, setInstrument] = useState('')
  const [strategyId, setStrategyId] = useState('')
  const [timeWindow, setTimeWindow] = useState<'24_7' | 'sessioned'>('24_7')

  const { data: strategiesResp } = useQuery({
    queryKey: ['strategies', 'apply-list', assetClass],
    queryFn: () =>
      strategiesApi.list().then((r) => r.data),
    enabled: !!assetClass,
  })

  const strategies = (
    Array.isArray(strategiesResp)
      ? strategiesResp
      : (strategiesResp as { strategies?: unknown[] })?.strategies ?? []
  ) as Array<{ id: string; strategy_id: string; strategy_kind?: string }>

  const executionStrategies = strategies.filter(
    (s) => s.strategy_kind === 'execution',
  )

  const handleAssetClassChange = useCallback((ac: string) => {
    setAssetClass(ac)
    setTimeWindow(is24_7(ac) ? '24_7' : 'sessioned')
  }, [])

  const mutation = useMutation({
    mutationFn: () =>
      api.post('/api/automations', {
        kind: 'single_instrument',
        account_mode: mode.toLowerCase(),
        spec: {
          asset_class: assetClass,
          instrument_id: instrument,
          execution_strategy_id: strategyId,
          time_window: { kind: timeWindow },
        },
        armed: true,
      }),
    onSuccess: () => onArmed?.(),
  })

  const canArm = !!assetClass && !!instrument && !!strategyId

  return (
    <div className="space-y-5 p-4 max-w-sm">
      {/* Step 1: Asset class */}
      <div>
        <StepLabel n={1}>Asset class</StepLabel>
        <Select
          value={assetClass}
          onChange={handleAssetClassChange}
          options={ASSET_CLASSES}
          placeholder="Select asset class…"
        />
      </div>

      {/* Step 2: Instrument */}
      <div>
        <StepLabel n={2}>Instrument</StepLabel>
        <input
          type="text"
          value={instrument}
          onChange={(e) => setInstrument(e.target.value)}
          placeholder="e.g. BTC-USD"
          disabled={!assetClass}
          className={cn(
            'w-full rounded-lg px-3 py-2 text-sm bg-surface-2 border border-border text-text',
            'placeholder:text-text-dim focus:outline-none focus:ring-1 focus:ring-accent',
            'disabled:opacity-50',
          )}
        />
      </div>

      {/* Step 3: Execution strategy */}
      <div>
        <StepLabel n={3}>Execution strategy</StepLabel>
        <Select
          value={strategyId}
          onChange={setStrategyId}
          options={executionStrategies.map((s) => ({
            value: s.id,
            label: s.strategy_id,
          }))}
          placeholder={assetClass ? 'Select strategy…' : 'Select asset class first'}
        />
        {assetClass && executionStrategies.length === 0 && (
          <p className="mt-1 text-xs text-text-dim">
            No compatible execution strategies found.
          </p>
        )}
      </div>

      {/* Step 4: Time window */}
      <div>
        <StepLabel n={4}>Time window</StepLabel>
        <Select
          value={timeWindow}
          onChange={(v) => setTimeWindow(v as '24_7' | 'sessioned')}
          options={[
            { value: '24_7', label: '24/7 (continuous)' },
            { value: 'sessioned', label: 'Sessioned (market hours)' },
          ]}
        />
        {assetClass && (
          <p className="mt-1 text-xs text-text-dim">
            {is24_7(assetClass)
              ? 'This asset class trades 24/7.'
              : 'This asset class has market sessions.'}
          </p>
        )}
      </div>

      {mutation.isError && (
        <p className="text-xs text-red-400">Failed to arm automation.</p>
      )}

      <button
        disabled={!canArm || mutation.isPending}
        onClick={() => mutation.mutate()}
        className="w-full rounded-lg py-2 text-sm font-semibold bg-accent text-white hover:bg-accent/80 disabled:opacity-40 transition-colors"
      >
        {mutation.isPending ? 'Arming…' : 'Arm automation'}
      </button>
    </div>
  )
}
