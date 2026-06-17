import { useState, useCallback } from 'react'
import { useQuery } from '@tanstack/react-query'
import { WorkspaceScroll } from '@/components/trading/WorkspaceScroll'
import { Panel } from '@/components/trading/Panel'
import { ScannerPanel } from '@/components/trading/ScannerPanel'
import { TerminalPanel, type TerminalAssetClass } from '@/components/trading/TerminalPanel'
import { ChartPanel } from '@/panels/ChartPanel'
import type { PanelSpec } from '@/config/layoutTemplates'
import { useWorkspaceStore } from '@/store/workspace'
import { useModeStore } from '@/store/mode'
import { assetApi, strategiesApi } from '@/lib/api'
import { cn } from '@/lib/utils'
import { Plus, X, ChevronLeft } from 'lucide-react'

const AC_LABEL: Record<string, string> = {
  crypto_spot_cex: 'Crypto CEX',
  crypto_spot_dex: 'Crypto DEX',
  perpetual_swap:  'Perp Swap',
  equity:          'Equity',
  etf:             'ETF',
  bond:            'Bond',
  fx:              'FX',
  option:          'Option',
}

function panelTitle(spec: PanelSpec): string {
  if (spec.kind === 'scanner') return 'Scanner'
  const ac = spec.assetClass ? (AC_LABEL[spec.assetClass] ?? spec.assetClass) : ''
  const prefix = spec.kind === 'chart' ? 'Chart' : 'Terminal'
  const label = spec.instrument ?? prefix
  return ac ? `${label} · ${ac}` : label
}

function panelWidth(spec: PanelSpec): number {
  if (spec.kind === 'scanner') return 360
  if (spec.kind === 'chart') return 580
  return 360
}

const ASSET_CLASS_OPTIONS = [
  { value: 'crypto_spot_cex', label: 'Crypto (CEX)' },
  { value: 'equity',          label: 'Equity' },
  { value: 'etf',             label: 'ETF' },
  { value: 'perpetual_swap',  label: 'Perp Swap' },
]

function inferAssetClass(symbol: string): string {
  const u = symbol.toUpperCase()
  if (u.endsWith('-USD') || u.endsWith('-USDT') || u.endsWith('-USDC') || u.endsWith('-BTC')) {
    return 'crypto_spot_cex'
  }
  return 'equity'
}

function AddPanelPicker({ kind, onAdd, onCancel }: {
  kind: 'chart' | 'terminal'
  onAdd: (instrument: string, assetClass: string) => void
  onCancel: () => void
}) {
  const [query, setQuery] = useState('')
  const [assetClass, setAssetClass] = useState('crypto_spot_cex')

  const { data } = useQuery({
    queryKey: ['initialized-assets'],
    queryFn: () => assetApi.initialized().then((r) => r.data.assets),
    staleTime: 30_000,
  })

  const initialized = data ?? []

  const filtered = query.trim()
    ? initialized.filter((a) =>
        a.symbol.toLowerCase().includes(query.toLowerCase()),
      )
    : initialized

  const customNotInList =
    query.trim() !== '' &&
    !initialized.some((a) => a.symbol.toLowerCase() === query.trim().toLowerCase())

  const title = kind === 'chart' ? 'Add Chart' : 'Add Terminal'

  return (
    <div className="flex flex-col gap-2 p-3 w-56">
      <div className="flex items-center justify-between">
        <span className="text-xs font-semibold text-text">{title}</span>
        <button onClick={onCancel} className="text-text-dim hover:text-text">
          <X className="h-3.5 w-3.5" />
        </button>
      </div>

      <input
        autoFocus
        value={query}
        onChange={(e) => {
          setQuery(e.target.value)
          setAssetClass(inferAssetClass(e.target.value))
        }}
        onKeyDown={(e) => {
          if (e.key === 'Enter' && customNotInList) {
            const sym = query.trim().toUpperCase()
            if (sym) onAdd(sym, assetClass)
          }
        }}
        placeholder="Symbol, e.g. ETH-USD"
        className="w-full rounded-md px-2.5 py-1.5 text-xs bg-surface-2 border border-border text-text placeholder:text-text-dim focus:outline-none focus:border-blue-500/60"
      />

      {filtered.length > 0 && (
        <div className="flex flex-col gap-0.5 max-h-40 overflow-y-auto">
          {filtered.map((a) => (
            <button
              key={a.symbol}
              onClick={() => onAdd(a.symbol, a.asset_class)}
              className="flex items-center justify-between rounded px-2 py-1.5 text-xs hover:bg-surface-2 text-text transition-colors text-left"
            >
              <span className="font-mono font-medium">{a.symbol}</span>
              <span className="text-text-dim text-[10px]">
                {ASSET_CLASS_OPTIONS.find((o) => o.value === a.asset_class)?.label ?? a.asset_class}
              </span>
            </button>
          ))}
        </div>
      )}

      {customNotInList && (
        <>
          <select
            value={assetClass}
            onChange={(e) => setAssetClass(e.target.value)}
            className="w-full rounded-md px-2.5 py-1.5 text-xs bg-surface-2 border border-border text-text focus:outline-none"
          >
            {ASSET_CLASS_OPTIONS.map((o) => (
              <option key={o.value} value={o.value}>{o.label}</option>
            ))}
          </select>
          <button
            onClick={() => onAdd(query.trim().toUpperCase(), assetClass)}
            className={cn(
              'w-full rounded-lg py-1.5 text-xs font-semibold transition-colors',
              'bg-blue-600 hover:bg-blue-500 text-white',
            )}
          >
            Open {query.trim().toUpperCase()}
          </button>
          {kind === 'chart' && (
            <p className="text-[10px] text-text-dim leading-tight">
              Asset hasn't been initialized yet. The chart will offer to seed it.
            </p>
          )}
        </>
      )}

      {initialized.length === 0 && !query.trim() && (
        <p className="text-xs text-text-dim text-center py-2">
          No initialized assets yet.
        </p>
      )}
    </div>
  )
}

// ── Scanner picker (4-step) ───────────────────────────────────────────────────

type ScannerStep = 'asset_class' | 'strategy' | 'timeframe' | 'instruments'
const SCANNER_STEPS: ScannerStep[] = ['asset_class', 'strategy', 'timeframe', 'instruments']

const TIMEFRAME_OPTIONS = [
  { value: '1m',  label: '1 min' },
  { value: '5m',  label: '5 min' },
  { value: '15m', label: '15 min' },
  { value: '30m', label: '30 min' },
  { value: '1h',  label: '1 hour' },
  { value: '4h',  label: '4 hour' },
  { value: '1d',  label: '1 day' },
]

interface ScannerConfig {
  assetClass: string
  strategyId: string
  timeframe: string
  instruments: string[]
}

function ScannerPicker({
  onAdd,
  onCancel,
}: {
  onAdd: (cfg: ScannerConfig) => void
  onCancel: () => void
}) {
  const [step, setStep] = useState<ScannerStep>('asset_class')
  const [assetClass, setAssetClass] = useState('crypto_spot_cex')
  const [strategyId, setStrategyId] = useState('')
  const [timeframe, setTimeframe] = useState('1h')
  const [selected, setSelected] = useState<Set<string>>(new Set())

  const { data: assetsData } = useQuery({
    queryKey: ['initialized-assets'],
    queryFn: () => assetApi.initialized().then((r) => r.data.assets),
    staleTime: 30_000,
  })
  const { data: strategiesData } = useQuery({
    queryKey: ['scanner-strategies', assetClass],
    queryFn: () => strategiesApi.applyList(assetClass).then((r) =>
      ((r.data as { strategies?: { id: string; strategy_id: string; strategy_kind?: string }[] }).strategies ?? [])
        .filter((s) => s.strategy_kind === 'discovery')
    ),
    staleTime: 30_000,
  })

  const allAssets = assetsData ?? []
  const strategies = strategiesData ?? []
  const filteredAssets = allAssets.filter((a) => a.asset_class === assetClass)

  const toggleInstrument = (sym: string) =>
    setSelected((prev) => {
      const next = new Set(prev)
      next.has(sym) ? next.delete(sym) : next.add(sym)
      return next
    })

  const stepLabel: Record<ScannerStep, string> = {
    asset_class: 'Asset class',
    strategy: 'Strategy',
    timeframe: 'Timeframe',
    instruments: 'Instruments',
  }

  const stepIdx = SCANNER_STEPS.indexOf(step)
  const goBack = () => { if (stepIdx > 0) setStep(SCANNER_STEPS[stepIdx - 1]) }
  const goNext = (next: ScannerStep) => setStep(next)

  return (
    <div className="flex flex-col gap-2 p-3 w-64">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-1.5">
          {step !== 'asset_class' && (
            <button onClick={goBack} className="text-text-dim hover:text-text">
              <ChevronLeft className="h-3.5 w-3.5" />
            </button>
          )}
          <span className="text-xs font-semibold text-text">Add Scanner — {stepLabel[step]}</span>
        </div>
        <button onClick={onCancel} className="text-text-dim hover:text-text">
          <X className="h-3.5 w-3.5" />
        </button>
      </div>

      {/* Step indicator */}
      <div className="flex gap-1">
        {SCANNER_STEPS.map((s, i) => (
          <div
            key={s}
            className={cn(
              'h-0.5 flex-1 rounded-full transition-colors',
              step === s ? 'bg-blue-500' : i < stepIdx ? 'bg-blue-500/40' : 'bg-border',
            )}
          />
        ))}
      </div>

      {/* Step 1 — Asset class */}
      {step === 'asset_class' && (
        <>
          <div className="flex flex-col gap-1.5 mt-1">
            {ASSET_CLASS_OPTIONS.map((o) => (
              <button
                key={o.value}
                onClick={() => setAssetClass(o.value)}
                className={cn(
                  'w-full text-left rounded-lg px-3 py-2 text-xs font-medium border transition-colors',
                  assetClass === o.value
                    ? 'bg-blue-500/20 border-blue-500/60 text-blue-300'
                    : 'border-border text-text-muted hover:text-text hover:border-border-2',
                )}
              >
                {o.label}
              </button>
            ))}
          </div>
          <button
            onClick={() => goNext('strategy')}
            className="mt-1 w-full rounded-lg py-1.5 text-xs font-semibold bg-blue-600 hover:bg-blue-500 text-white transition-colors"
          >
            Next
          </button>
        </>
      )}

      {/* Step 2 — Strategy */}
      {step === 'strategy' && (
        <>
          <div className="flex flex-col gap-1 mt-1 max-h-48 overflow-y-auto">
            <button
              onClick={() => setStrategyId('')}
              className={cn(
                'w-full text-left rounded-lg px-3 py-2 text-xs border transition-colors',
                strategyId === ''
                  ? 'bg-blue-500/20 border-blue-500/60 text-blue-300'
                  : 'border-border text-text-muted hover:text-text',
              )}
            >
              None (watch only)
            </button>
            {strategies.map((s) => (
              <button
                key={s.id}
                onClick={() => setStrategyId(s.id)}
                className={cn(
                  'w-full text-left rounded-lg px-3 py-2 text-xs font-medium border transition-colors',
                  strategyId === s.id
                    ? 'bg-blue-500/20 border-blue-500/60 text-blue-300'
                    : 'border-border text-text-muted hover:text-text',
                )}
              >
                {s.strategy_id}
              </button>
            ))}
            {strategies.length === 0 && (
              <p className="text-xs text-text-dim px-1 py-2">No scanner strategies yet. Build one in the Strategy Builder using Scanner mode.</p>
            )}
          </div>
          <button
            onClick={() => goNext('timeframe')}
            className="mt-1 w-full rounded-lg py-1.5 text-xs font-semibold bg-blue-600 hover:bg-blue-500 text-white transition-colors"
          >
            Next
          </button>
        </>
      )}

      {/* Step 3 — Timeframe */}
      {step === 'timeframe' && (
        <>
          <div className="grid grid-cols-2 gap-1.5 mt-1">
            {TIMEFRAME_OPTIONS.map((o) => (
              <button
                key={o.value}
                onClick={() => setTimeframe(o.value)}
                className={cn(
                  'rounded-lg px-3 py-2 text-xs font-medium border transition-colors text-center',
                  timeframe === o.value
                    ? 'bg-blue-500/20 border-blue-500/60 text-blue-300'
                    : 'border-border text-text-muted hover:text-text hover:border-border-2',
                )}
              >
                <span className="font-mono font-semibold">{o.value}</span>
                <span className="block text-[10px] text-text-dim mt-0.5">{o.label}</span>
              </button>
            ))}
          </div>
          <button
            onClick={() => goNext('instruments')}
            className="mt-1 w-full rounded-lg py-1.5 text-xs font-semibold bg-blue-600 hover:bg-blue-500 text-white transition-colors"
          >
            Next
          </button>
        </>
      )}

      {/* Step 4 — Instruments */}
      {step === 'instruments' && (
        <>
          {filteredAssets.length === 0 ? (
            <p className="text-xs text-text-dim py-2">
              No initialized {ASSET_CLASS_OPTIONS.find((o) => o.value === assetClass)?.label ?? assetClass} assets yet.
            </p>
          ) : (
            <div className="flex flex-col gap-1 mt-1 max-h-52 overflow-y-auto">
              {filteredAssets.map((a) => (
                <button
                  key={a.symbol}
                  onClick={() => toggleInstrument(a.symbol)}
                  className={cn(
                    'w-full text-left rounded-lg px-3 py-2 text-xs font-mono font-medium border transition-colors flex items-center gap-2',
                    selected.has(a.symbol)
                      ? 'bg-blue-500/20 border-blue-500/60 text-blue-300'
                      : 'border-border text-text-muted hover:text-text',
                  )}
                >
                  <span className={cn(
                    'h-3 w-3 rounded border shrink-0 flex items-center justify-center text-[9px]',
                    selected.has(a.symbol) ? 'bg-blue-500 border-blue-500 text-white' : 'border-border-2',
                  )}>
                    {selected.has(a.symbol) ? '✓' : ''}
                  </span>
                  {a.symbol}
                </button>
              ))}
            </div>
          )}
          <button
            onClick={() => onAdd({ assetClass, strategyId, timeframe, instruments: [...selected] })}
            disabled={selected.size === 0 && filteredAssets.length > 0}
            className="mt-1 w-full rounded-lg py-1.5 text-xs font-semibold bg-blue-600 hover:bg-blue-500 text-white transition-colors disabled:opacity-40 disabled:cursor-not-allowed"
          >
            Add Scanner{selected.size > 0 ? ` (${selected.size})` : ''}
          </button>
        </>
      )}
    </div>
  )
}

type PickerKind = 'chart' | 'terminal' | 'scanner' | null

export function TradingPage() {
  // Panel layout is persisted per trading mode (localStorage via the workspace
  // store) so a refresh keeps every open chart / terminal / scanner, and a
  // PAPER window keeps a different layout from a LIVE window.
  const mode = useModeStore((s) => s.mode)
  const panels = useWorkspaceStore((s) => s.byMode[mode].panels)
  const setPanelsForMode = useWorkspaceStore((s) => s.setPanels)
  const removeChartSettings = useWorkspaceStore((s) => s.removeChartSettings)
  const [pickerKind, setPickerKind] = useState<PickerKind>(null)
  const [dragIndex, setDragIndex] = useState<number | null>(null)
  const [dragOverIndex, setDragOverIndex] = useState<number | null>(null)

  // Bind the panel mutators to the current mode so every edit lands in this
  // window's mode slice.
  const setPanels = useCallback(
    (updater: (prev: PanelSpec[]) => PanelSpec[]) => setPanelsForMode(mode, updater),
    [setPanelsForMode, mode],
  )

  const removePanel = useCallback((id: string) => {
    setPanels((prev) => prev.filter((p) => p.id !== id))
    // Drop the closed chart's persisted view settings so storage doesn't grow.
    removeChartSettings(mode, id)
  }, [setPanels, removeChartSettings, mode])

  const addPanel = useCallback((instrument: string, assetClass: string, kind: 'chart' | 'terminal') => {
    const id = `${kind}-${Date.now()}`
    setPanels((prev) => [...prev, { id, kind, instrument, venue: 'kraken', assetClass }])
    setPickerKind(null)
  }, [setPanels])

  const addScannerFromPicker = useCallback((cfg: ScannerConfig) => {
    const id = `scanner-${Date.now()}`
    setPanels((prev) => [...prev, {
      id, kind: 'scanner',
      assetClass: cfg.assetClass,
      strategyId: cfg.strategyId || undefined,
      timeframe: cfg.timeframe,
      instruments: cfg.instruments,
    }])
    setPickerKind(null)
  }, [setPanels])

  const handleDragStart = useCallback((index: number, e: React.DragEvent) => {
    setDragIndex(index)
    e.dataTransfer.effectAllowed = 'move'
  }, [])

  const handleDragOver = useCallback((index: number, e: React.DragEvent) => {
    e.preventDefault()
    e.dataTransfer.dropEffect = 'move'
    setDragOverIndex(index)
  }, [])

  const handleDrop = useCallback((index: number) => {
    setDragIndex((src) => {
      if (src !== null && src !== index) {
        setPanels((prev) => {
          const next = [...prev]
          const [moved] = next.splice(src, 1)
          next.splice(index, 0, moved)
          return next
        })
      }
      return null
    })
    setDragOverIndex(null)
  }, [setPanels])

  const handleDragEnd = useCallback(() => {
    setDragIndex(null)
    setDragOverIndex(null)
  }, [])

  return (
    <div className="flex flex-col h-full overflow-hidden">
      <WorkspaceScroll className="flex-1">
        {panels.map((spec, i) => (
          <Panel
            key={`${mode}:${spec.id}`}
            title={panelTitle(spec)}
            width={panelWidth(spec)}
            onClose={() => removePanel(spec.id)}
            isDragging={dragIndex === i}
            isDragOver={dragOverIndex === i && dragIndex !== i}
            onDragStart={(e) => handleDragStart(i, e)}
            onDragOver={(e) => handleDragOver(i, e)}
            onDrop={() => handleDrop(i)}
            onDragEnd={handleDragEnd}
          >
            {spec.kind === 'scanner' ? (
              <ScannerPanel
                initialInstruments={spec.instruments ?? []}
                initialStrategyId={spec.strategyId}
                initialTimeframe={spec.timeframe ?? '1h'}
              />
            ) : spec.kind === 'chart' ? (
              <div className="h-full overflow-y-auto">
                <ChartPanel
                  persistKey={spec.id}
                  instrument={spec.instrument ?? 'BTC-USD'}
                  assetClass={spec.assetClass ?? 'crypto_spot_cex'}
                />
              </div>
            ) : (
              <TerminalPanel
                instrument={spec.instrument ?? 'BTC-USD'}
                assetClass={(spec.assetClass as TerminalAssetClass) ?? 'crypto_spot_cex'}
              />
            )}
          </Panel>
        ))}

        {/* Add panel column */}
        <div
          className="flex flex-col items-center justify-center gap-4 shrink-0 border-l border-dashed border-border p-4"
          style={{ width: pickerKind ? 'auto' : '10rem' }}
        >
          {pickerKind === 'scanner' ? (
            <ScannerPicker
              onAdd={addScannerFromPicker}
              onCancel={() => setPickerKind(null)}
            />
          ) : pickerKind ? (
            <AddPanelPicker
              kind={pickerKind}
              onAdd={(instrument, assetClass) => addPanel(instrument, assetClass, pickerKind)}
              onCancel={() => setPickerKind(null)}
            />
          ) : (
            <>
              <button
                onClick={() => setPickerKind('scanner')}
                className="flex flex-col items-center gap-2 rounded-xl border border-dashed border-border p-4 text-xs text-text-dim hover:text-text hover:border-border-2 transition-colors w-full"
              >
                <Plus className="h-4 w-4" />
                Scanner
              </button>
              <button
                onClick={() => setPickerKind('chart')}
                className="flex flex-col items-center gap-2 rounded-xl border border-dashed border-border p-4 text-xs text-text-dim hover:text-text hover:border-border-2 transition-colors w-full"
              >
                <Plus className="h-4 w-4" />
                Chart
              </button>
              <button
                onClick={() => setPickerKind('terminal')}
                className="flex flex-col items-center gap-2 rounded-xl border border-dashed border-border p-4 text-xs text-text-dim hover:text-text hover:border-border-2 transition-colors w-full"
              >
                <Plus className="h-4 w-4" />
                Terminal
              </button>
            </>
          )}
        </div>
      </WorkspaceScroll>
    </div>
  )
}
