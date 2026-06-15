import { useEffect, useRef, useState } from 'react'
import { useMutation, useQuery } from '@tanstack/react-query'
import { format } from 'date-fns'
import { RefreshCw, X, BarChart2 } from 'lucide-react'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { MultiPaneChart, type IndicatorKind, type IndicatorInstance } from '@/components/charts/MultiPaneChart'
import type { Bar as OhlcvBar } from '@/components/charts/OhlcvChart'
import { wsBus, getWsClient } from '@/api/ws'
import { assetApi } from '@/lib/api'
import { cn } from '@/lib/utils'
import type { Bar, WsOutMessage } from '@/lib/types'
import type { PriceLineAnnotation } from '@/components/charts/Annotations'

interface ChartPanelProps {
  instrument: string
  initialBars?: Bar[]
  priceLines?: PriceLineAnnotation[]
  assetClass?: string
}

const PANEL_ID_PREFIX = 'chart_'

function toOhlcvBar(b: Bar): OhlcvBar | null {
  const open = parseFloat(b.open)
  const high = parseFloat(b.high)
  const low = parseFloat(b.low)
  const close = parseFloat(b.close)
  const volume = parseFloat(b.volume)
  if (!b.time || isNaN(open) || isNaN(high) || isNaN(low) || isNaN(close)) return null
  return { ts: b.time, open, high, low, close, volume: isNaN(volume) ? 0 : volume }
}

function isBar(payload: unknown): payload is Bar {
  if (!payload || typeof payload !== 'object') return false
  const p = payload as Record<string, unknown>
  return (
    typeof p.time === 'string' &&
    typeof p.open === 'string' &&
    typeof p.high === 'string' &&
    typeof p.low === 'string' &&
    typeof p.close === 'string'
  )
}

type ConnectionState = 'live' | 'reconnecting' | 'error' | 'disconnected'

// ── Inline init dialog ────────────────────────────────────────────────────────

const LOOKBACK_PRESETS = [
  { label: '30d',  days: 30  },
  { label: '90d',  days: 90  },
  { label: '180d', days: 180 },
  { label: '1yr',  days: 365 },
]

const ASSET_CLASS_OPTIONS = [
  { value: 'crypto_spot_cex', label: 'Crypto (CEX)' },
  { value: 'equity',          label: 'Equity' },
  { value: 'etf',             label: 'ETF' },
  { value: 'perpetual_swap',  label: 'Perp Swap' },
]

function InitOverlay({
  instrument,
  defaultAssetClass,
  onDone,
  forceOpen = false,
}: {
  instrument: string
  defaultAssetClass: string
  onDone: () => void
  forceOpen?: boolean
}) {
  const [open, setOpen] = useState(forceOpen)
  const [days, setDays] = useState(90)
  const [assetClass, setAssetClass] = useState(defaultAssetClass)

  const initMut = useMutation({
    mutationFn: () => assetApi.init(instrument, days, assetClass),
    onSuccess: onDone,
  })

  if (!open) {
    return (
      <div className="flex flex-col items-center justify-center gap-3 h-72">
        <p className="text-sm text-text-dim">No bar data for {instrument}</p>
        <button
          onClick={() => setOpen(true)}
          className="flex items-center gap-1.5 rounded-lg bg-blue-500/20 border border-blue-500/40 px-3 py-1.5 text-xs font-medium text-blue-300 hover:bg-blue-500/30 transition-colors"
        >
          <RefreshCw className="h-3.5 w-3.5" />
          Initialize &amp; seed bars
        </button>
      </div>
    )
  }

  return (
    <div className="flex flex-col gap-3 p-4 h-72">
      <div className="flex items-center justify-between">
        <span className="text-xs font-semibold text-text">Initialize <span className="font-mono text-blue-400">{instrument}</span></span>
        <button onClick={() => { setOpen(false); if (forceOpen) onDone() }} className="text-text-dim hover:text-text">
          <X className="h-3.5 w-3.5" />
        </button>
      </div>

      <div className="space-y-1">
        <label className="text-xs text-text-dim">Asset class</label>
        <select
          value={assetClass}
          onChange={e => setAssetClass(e.target.value)}
          className="w-full rounded-md px-2.5 py-1.5 text-xs bg-surface-2 border border-border text-text focus:outline-none"
        >
          {ASSET_CLASS_OPTIONS.map(o => (
            <option key={o.value} value={o.value}>{o.label}</option>
          ))}
        </select>
      </div>

      <div className="space-y-1.5">
        <label className="text-xs text-text-dim">Lookback</label>
        <div className="flex gap-1.5 flex-wrap">
          {LOOKBACK_PRESETS.map(p => (
            <button
              key={p.days}
              onClick={() => setDays(p.days)}
              className={cn(
                'rounded px-2 py-1 text-xs font-medium border transition-colors',
                days === p.days
                  ? 'bg-blue-500/20 border-blue-500/60 text-blue-300'
                  : 'border-border text-text-muted hover:text-text',
              )}
            >
              {p.label}
            </button>
          ))}
        </div>
        <p className="text-xs text-text-dim">~{days * 24} hourly + ~{days * 24 * 60} 1m bars</p>
      </div>

      {initMut.isError && (
        <p className="text-xs text-red-400">Init failed — check asset class and try again.</p>
      )}

      <button
        onClick={() => initMut.mutate()}
        disabled={initMut.isPending}
        className="mt-auto w-full rounded-lg py-1.5 text-xs font-semibold bg-blue-600 hover:bg-blue-500 text-white disabled:opacity-50 transition-colors"
      >
        {initMut.isPending ? 'Seeding…' : 'Start seeding'}
      </button>
    </div>
  )
}

// ── Indicator picker ────────────────────────────────────────────────────────

const OVERLAY_COLORS = [
  '#3b82f6', '#f59e0b', '#10b981', '#ef4444',
  '#8b5cf6', '#ec4899', '#f97316', '#06b6d4',
]

function genUid(kind: string) {
  return `${kind}_${Date.now()}_${Math.random().toString(36).slice(2, 5)}`
}

function instLabel(inst: IndicatorInstance): string {
  switch (inst.kind) {
    case 'ema':    return `EMA(${inst.period})`
    case 'sma':    return `SMA(${inst.period})`
    case 'bb':     return `BB(${inst.period}, ${inst.stddev})`
    case 'rsi':    return `RSI(${inst.period})`
    case 'macd':   return `MACD(${inst.fast}, ${inst.slow}, ${inst.signal})`
    case 'volume': return 'Volume'
  }
}

type KindMeta = { kind: IndicatorKind; label: string; section: 'overlay' | 'subpane' }

const KIND_LIST: KindMeta[] = [
  { kind: 'ema',    label: 'EMA',    section: 'overlay'  },
  { kind: 'sma',    label: 'SMA',    section: 'overlay'  },
  { kind: 'bb',     label: 'BB',     section: 'overlay'  },
  { kind: 'volume', label: 'Volume', section: 'subpane'  },
  { kind: 'rsi',    label: 'RSI',    section: 'subpane'  },
  { kind: 'macd',   label: 'MACD',   section: 'subpane'  },
]

function IndicatorPicker({
  active,
  onAdd,
  onRemove,
  onClear,
  onClose,
}: {
  active: IndicatorInstance[]
  onAdd: (inst: IndicatorInstance) => void
  onRemove: (uid: string) => void
  onClear: () => void
  onClose: () => void
}) {
  const ref = useRef<HTMLDivElement>(null)
  const [selectedKind, setSelectedKind] = useState<IndicatorKind | null>(null)

  // per-type form state
  const [period, setPeriod]   = useState('20')
  const [stddev, setStddev]   = useState('2')
  const [fast,   setFast]     = useState('12')
  const [slow,   setSlow]     = useState('26')
  const [signal, setSignal]   = useState('9')
  const [rsiPer, setRsiPer]   = useState('14')

  useEffect(() => {
    const h = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) onClose()
    }
    document.addEventListener('mousedown', h)
    return () => document.removeEventListener('mousedown', h)
  }, [onClose])

  function handleAdd() {
    if (!selectedKind) return
    const overlayIdx = active.filter((i) => ['ema', 'sma', 'bb'].includes(i.kind)).length
    const color = OVERLAY_COLORS[overlayIdx % OVERLAY_COLORS.length]
    let inst: IndicatorInstance
    switch (selectedKind) {
      case 'ema':    inst = { uid: genUid('ema'),  kind: 'ema',    period: parseInt(period)  || 20, color }; break
      case 'sma':    inst = { uid: genUid('sma'),  kind: 'sma',    period: parseInt(period)  || 20, color }; break
      case 'bb':     inst = { uid: genUid('bb'),   kind: 'bb',     period: parseInt(period)  || 20, stddev: parseFloat(stddev) || 2, color }; break
      case 'rsi':    inst = { uid: genUid('rsi'),  kind: 'rsi',    period: parseInt(rsiPer)  || 14 }; break
      case 'macd':   inst = { uid: genUid('macd'), kind: 'macd',   fast: parseInt(fast) || 12, slow: parseInt(slow) || 26, signal: parseInt(signal) || 9 }; break
      case 'volume': inst = { uid: genUid('vol'),  kind: 'volume' }; break
    }
    onAdd(inst)
    setSelectedKind(null)
  }

  const numInput = (label: string, val: string, set: (v: string) => void) => (
    <div className="flex items-center gap-2">
      <label className="text-xs text-text-dim w-14 shrink-0">{label}</label>
      <input
        type="number"
        value={val}
        onChange={(e) => set(e.target.value)}
        className="w-16 rounded px-2 py-1 text-xs bg-background border border-border text-text focus:outline-none focus:border-blue-500/60"
      />
    </div>
  )

  const TypeBtn = ({ kind, label }: KindMeta) => (
    <button
      onClick={() => setSelectedKind(selectedKind === kind ? null : kind)}
      className={cn(
        'px-2.5 py-1 rounded-md text-xs font-medium border transition-colors',
        selectedKind === kind
          ? 'bg-blue-500/20 border-blue-500/50 text-blue-300'
          : 'border-border text-text-dim hover:text-text hover:border-border-2',
      )}
    >
      {label}
    </button>
  )

  return (
    <div
      ref={ref}
      className="absolute right-0 top-7 z-50 w-56 rounded-xl border border-border-2 bg-surface shadow-xl shadow-black/40"
    >
      <div className="border-b border-border px-3 py-2 text-[11px] font-semibold text-text">
        Indicators
      </div>

      {/* type buttons */}
      <div className="p-2 space-y-2">
        <div>
          <div className="text-[10px] font-semibold uppercase tracking-wider text-text-dim px-1 mb-1">
            Overlays
          </div>
          <div className="flex gap-1 flex-wrap">
            {KIND_LIST.filter((k) => k.section === 'overlay').map((k) => (
              <TypeBtn key={k.kind} {...k} />
            ))}
          </div>
        </div>
        <div>
          <div className="text-[10px] font-semibold uppercase tracking-wider text-text-dim px-1 mb-1">
            Sub-pane
          </div>
          <div className="flex gap-1 flex-wrap">
            {KIND_LIST.filter((k) => k.section === 'subpane').map((k) => (
              <TypeBtn key={k.kind} {...k} />
            ))}
          </div>
        </div>
      </div>

      {/* form for selected type */}
      {selectedKind && (
        <div className="border-t border-border p-3 flex flex-col gap-2">
          {(selectedKind === 'ema' || selectedKind === 'sma') && numInput('Period', period, setPeriod)}
          {selectedKind === 'bb' && (
            <>
              {numInput('Period', period, setPeriod)}
              {numInput('Std Dev', stddev, setStddev)}
            </>
          )}
          {selectedKind === 'rsi' && numInput('Period', rsiPer, setRsiPer)}
          {selectedKind === 'macd' && (
            <>
              {numInput('Fast', fast, setFast)}
              {numInput('Slow', slow, setSlow)}
              {numInput('Signal', signal, setSignal)}
            </>
          )}
          {selectedKind === 'volume' && (
            <p className="text-xs text-text-dim">No parameters needed.</p>
          )}
          <button
            onClick={handleAdd}
            className="mt-1 w-full rounded-lg py-1.5 text-xs font-semibold bg-blue-600 hover:bg-blue-500 text-white transition-colors"
          >
            Add {selectedKind.toUpperCase()}
          </button>
        </div>
      )}

      {/* active indicators */}
      {active.length > 0 && (
        <div className="border-t border-border p-3">
          <div className="flex items-center justify-between mb-1.5">
            <span className="text-[10px] font-semibold uppercase tracking-wider text-text-dim">
              Active
            </span>
            <button
              onClick={() => { onClear(); onClose() }}
              className="text-[10px] text-text-dim hover:text-text transition-colors"
            >
              Clear all
            </button>
          </div>
          <div className="flex flex-col gap-1 max-h-40 overflow-y-auto">
            {active.map((inst) => (
              <div
                key={inst.uid}
                className="flex items-center justify-between rounded px-2 py-1 bg-surface-2"
              >
                <div className="flex items-center gap-1.5 min-w-0">
                  {inst.color ? (
                    <span
                      className="inline-block h-2 w-2 rounded-full shrink-0"
                      style={{ background: inst.color }}
                    />
                  ) : (
                    <span className="inline-block h-2 w-2 rounded-full shrink-0 bg-text-dim/40" />
                  )}
                  <span className="text-xs text-text font-mono truncate">{instLabel(inst)}</span>
                </div>
                <button
                  onClick={() => onRemove(inst.uid)}
                  className="shrink-0 ml-1 text-text-dim hover:text-text transition-colors"
                >
                  <X className="h-3 w-3" />
                </button>
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  )
}

// ── Timeframe selector ──────────────────────────────────────────────────────

const TIMEFRAMES = [
  { label: '1m',  secs: 60   },
  { label: '5m',  secs: 300  },
  { label: '15m', secs: 900  },
  { label: '30m', secs: 1800 },
  { label: '1h',  secs: 3600 },
] as const

// ── ChartPanel ────────────────────────────────────────────────────────────────

export function ChartPanel({
  instrument,
  initialBars = [],
  priceLines = [],
  assetClass = 'crypto_spot_cex',
}: ChartPanelProps) {
  const panelId = useRef(`${PANEL_ID_PREFIX}${instrument}`).current
  const [liveBars, setLiveBars] = useState<Bar[]>(() => initialBars)
  const [connState, setConnState] = useState<ConnectionState>('disconnected')
  const [initDone, setInitDone] = useState(false)
  const [tfSecs, setTfSecs] = useState(3600)
  const [showReseed, setShowReseed] = useState(false)

  const [activeIndicators, setActiveIndicators] = useState<IndicatorInstance[]>([])
  const [showIndicatorPicker, setShowIndicatorPicker] = useState(false)

  const tf = TIMEFRAMES.find((t) => t.secs === tfSecs) ?? TIMEFRAMES[4]

  function addIndicator(inst: IndicatorInstance) {
    setActiveIndicators((prev) => [...prev, inst])
  }
  function removeIndicator(uid: string) {
    setActiveIndicators((prev) => prev.filter((i) => i.uid !== uid))
  }

  // Fetch all available bars for the selected timeframe — no artificial
  // lookback cap. Start from epoch so the query returns everything in the DB.
  const { data: restBars } = useQuery({
    queryKey: ['chart-bars-rest', instrument, tfSecs],
    queryFn: () => {
      const end = new Date()
      const start = new Date(0) // epoch — return all stored bars
      return assetApi
        .chartBars(
          instrument,
          format(start, "yyyy-MM-dd'T'HH:mm:ss"),
          format(end, "yyyy-MM-dd'T'HH:mm:ss"),
          tfSecs,
        )
        .then((r) => r.data?.bars ?? [])
    },
    refetchInterval: 5000,
  })

  // Lifecycle state to know if the asset needs initialization
  const { data: lifecycle, refetch: refetchLifecycle } = useQuery({
    queryKey: ['lifecycle-terminal', instrument],
    queryFn: () => assetApi.lifecycle(instrument).then((r) => r.data),
    refetchInterval: 10000,
  })

  // Convert REST bars { t, o, h, l, c, v } → OhlcvBar
  const restOhlcv: OhlcvBar[] = (restBars ?? []).map(
    (b: { t: number; o: string; h: string; l: string; c: string; v: string }) => ({
      ts: b.t,
      open: parseFloat(b.o),
      high: parseFloat(b.h),
      low: parseFloat(b.l),
      close: parseFloat(b.c),
      volume: parseFloat(b.v),
    }),
  )

  // Live 1m WS bars converted to OhlcvBar
  const liveOhlcv = liveBars.flatMap((b) => {
    const mapped = toOhlcvBar(b)
    return mapped ? [mapped] : []
  })

  // The live WS feed is 1-minute, so it's only appended on the 1m timeframe;
  // coarser views rely on the 5s REST poll of the (server-aggregated) bars.
  const allBars: OhlcvBar[] = (() => {
    if (tfSecs !== 60) return restOhlcv
    if (restOhlcv.length === 0) return liveOhlcv
    const lastTs = restOhlcv[restOhlcv.length - 1].ts
    const newer = liveOhlcv.filter((b) => b.ts > lastTs)
    return [...restOhlcv, ...newer]
  })()

  const isUninitialized =
    lifecycle?.lifecycle === 'uninitialized' || lifecycle?.lifecycle === undefined

  const noData = allBars.length === 0

  useEffect(() => {
    // Subscribe to the WS lane. getWsClient() may be null briefly on first
    // render before auto-init completes — subscribe anyway so the bus listener
    // is always active; pending subscriptions are replayed on WS connect.
    const client = getWsClient()
    client?.subscribe(panelId, [
      { lane: 'market.bars.1m', instrument },
    ])

    const unsub = wsBus.on((msg: WsOutMessage) => {
      if ((msg as { type: string }).type === 'connected') { setConnState('live'); return }
      if ((msg as { type: string }).type === 'disconnected') { setConnState('reconnecting'); return }
      if ((msg as { type: string }).type === 'error') { setConnState('error'); return }

      if (
        msg.type === 'frame' &&
        msg.lane === 'market.bars.1m' &&
        msg.instrument === instrument
      ) {
        if (!isBar(msg.payload)) return
        const bar = msg.payload
        setLiveBars((prev) => {
          if (prev.length > 0 && prev[prev.length - 1].time === bar.time) {
            return [...prev.slice(0, -1), bar]
          }
          return [...prev, bar].slice(-500)
        })
      }
      if (msg.type === 'heartbeat') setConnState('live')
    })

    return () => {
      unsub()
      client?.unsubscribe(panelId)
      setConnState('disconnected')
    }
  }, [instrument, panelId])

  const connLabel =
    connState === 'live' ? 'live'
    : connState === 'reconnecting' ? 'reconnecting'
    : connState === 'error' ? 'error'
    : 'disconnected'

  const connColor =
    connState === 'live' ? 'text-green-400'
    : connState === 'error' ? 'text-red-400'
    : 'text-text-dim'

  return (
    <Card>
      <CardHeader className="pb-2 flex-row items-center justify-between gap-2">
        <CardTitle className="text-sm shrink-0">{instrument} — {tf.label}</CardTitle>
        <div className="flex items-center gap-0.5 rounded-md border border-border p-0.5">
          {TIMEFRAMES.map((t) => (
            <button
              key={t.secs}
              onClick={() => setTfSecs(t.secs)}
              className={cn(
                'px-1.5 py-0.5 rounded text-xs font-medium transition-colors',
                tfSecs === t.secs
                  ? 'bg-blue-500/20 text-blue-300'
                  : 'text-text-dim hover:text-text',
              )}
            >
              {t.label}
            </button>
          ))}
        </div>
        <span className={`text-xs shrink-0 ${connColor}`}>{connLabel}</span>
        {/* Indicator picker */}
        <div className="relative shrink-0">
          <button
            onClick={() => setShowIndicatorPicker((v) => !v)}
            title="Indicators"
            className={cn(
              'flex items-center gap-1 px-1.5 py-0.5 rounded text-xs transition-colors',
              activeIndicators.length > 0
                ? 'text-blue-400 bg-blue-500/10'
                : 'text-text-dim hover:text-text',
            )}
          >
            <BarChart2 className="h-3.5 w-3.5" />
            {activeIndicators.length > 0 && (
              <span className="text-[10px] font-semibold">{activeIndicators.length}</span>
            )}
          </button>
          {showIndicatorPicker && (
            <IndicatorPicker
              active={activeIndicators}
              onAdd={addIndicator}
              onRemove={removeIndicator}
              onClear={() => setActiveIndicators([])}
              onClose={() => setShowIndicatorPicker(false)}
            />
          )}
        </div>
        {!isUninitialized && (
          <button
            onClick={() => setShowReseed((v) => !v)}
            title="Reseed historical bars"
            className="shrink-0 text-text-dim hover:text-text transition-colors"
          >
            <RefreshCw className="h-3.5 w-3.5" />
          </button>
        )}
      </CardHeader>
      <CardContent className="p-0 pb-2">
        {(noData && isUninitialized) || showReseed ? (
          <InitOverlay
            instrument={instrument}
            defaultAssetClass={assetClass}
            forceOpen={showReseed}
            onDone={() => {
              setShowReseed(false)
              setInitDone(true)
              void refetchLifecycle()
            }}
          />
        ) : noData ? (
          <div className="flex h-72 items-center justify-center text-text-dim text-sm">
            Waiting for bars…
          </div>
        ) : (
          <MultiPaneChart
            bars={allBars}
            indicators={activeIndicators}
            markers={[]}
            priceLines={priceLines}
            mainHeight={300}
          />
        )}
      </CardContent>
    </Card>
  )
}
