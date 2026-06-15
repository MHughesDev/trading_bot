// Scanner panel — instruments split into Triggered / Watching sections.
// Condition evaluation runs client-side against incoming WS market.bars.1m frames.

import { useState, useCallback, useEffect, useRef } from 'react'
import { useQuery } from '@tanstack/react-query'
import { strategiesApi } from '@/lib/api'
import { WatchTile, type WatchTileData } from './WatchTile'
import { cn } from '@/lib/utils'
import { ChevronDown, ChevronRight } from 'lucide-react'
import { wsBus, getWsClient } from '@/api/ws'
import type { WsOutMessage } from '@/lib/types'
import { calcEMA, calcRSI } from '@/utils/indicators'

// ── Strategy condition evaluator ──────────────────────────────────────────────

type Operand =
  | { kind: 'feature'; fn: 'ema' | 'rsi' | 'sma'; period: number }
  | { kind: 'bar'; field: string }
  | { kind: 'literal'; value: number }

interface EvalCond {
  left: Operand
  op: '>' | '<'
  right: Operand
}

interface ParsedStrategy {
  condById: Map<string, EvalCond>
  signals: Array<{ when: string; emit: string }>
  minBars: number
}

function parseOperand(s: string): Operand | null {
  s = s.trim()
  const fm = s.match(/^feature\('([a-z]+)_(\d+)'\)$/)
  if (fm) return { kind: 'feature', fn: fm[1] as 'ema' | 'rsi' | 'sma', period: parseInt(fm[2]) }
  const bm = s.match(/^bar\('([a-z]+)'\)$/)
  if (bm) return { kind: 'bar', field: bm[1] }
  const n = parseFloat(s)
  if (!isNaN(n)) return { kind: 'literal', value: n }
  return null
}

function parseExpr(expr: string): EvalCond | null {
  for (const op of [' > ', ' < '] as const) {
    const idx = expr.indexOf(op)
    if (idx === -1) continue
    const left = parseOperand(expr.slice(0, idx))
    const right = parseOperand(expr.slice(idx + op.length))
    if (left && right) return { left, op: op.trim() as '>' | '<', right }
  }
  return null
}

function evalOp(op: Operand, closes: number[]): number {
  const last = closes[closes.length - 1]
  if (op.kind === 'literal') return op.value
  if (op.kind === 'bar') return last
  if (op.fn === 'ema') { const v = calcEMA(closes, op.period); return v[v.length - 1] }
  if (op.fn === 'rsi') { const v = calcRSI(closes, op.period); return v[v.length - 1] }
  return NaN
}

function evalCond(c: EvalCond, closes: number[]): boolean {
  const l = evalOp(c.left, closes)
  const r = evalOp(c.right, closes)
  if (isNaN(l) || isNaN(r)) return false
  return c.op === '>' ? l > r : l < r
}

function parseDefinition(def: {
  nodes: Array<
    | { id: string; type: 'condition'; expr: string }
    | { id: string; type: 'signal'; when: string; emit: string }
  >
}): ParsedStrategy {
  const condById = new Map<string, EvalCond>()
  const signals: Array<{ when: string; emit: string }> = []
  let maxPeriod = 2

  for (const node of def.nodes) {
    if (node.type === 'condition') {
      const c = parseExpr(node.expr)
      if (c) {
        condById.set(node.id, c)
        for (const op of [c.left, c.right]) {
          if (op.kind === 'feature') maxPeriod = Math.max(maxPeriod, op.period)
        }
      }
    } else {
      signals.push({ when: node.when, emit: node.emit })
    }
  }

  return { condById, signals, minBars: maxPeriod }
}

function checkTriggered(strat: ParsedStrategy, closes: number[]): boolean {
  if (closes.length < strat.minBars) return false
  for (const sig of strat.signals) {
    if (sig.emit !== 'scanner_signal') continue
    const cond = strat.condById.get(sig.when)
    if (cond && evalCond(cond, closes)) return true
  }
  return false
}

// ── Sub-components ────────────────────────────────────────────────────────────

interface ScannerPanelProps {
  initialInstruments?: string[]
  initialStrategyId?: string
  initialTimeframe?: string
}

interface SectionHeaderProps {
  label: string
  count: number
  open: boolean
  onToggle: () => void
  accent?: boolean
}

function SectionHeader({ label, count, open, onToggle, accent }: SectionHeaderProps) {
  return (
    <button
      onClick={onToggle}
      className={cn(
        'w-full flex items-center gap-2 px-3 py-1.5 text-left select-none transition-colors',
        'border-b border-border hover:bg-surface-2',
      )}
    >
      {open
        ? <ChevronDown className="h-3 w-3 shrink-0 text-text-dim" />
        : <ChevronRight className="h-3 w-3 shrink-0 text-text-dim" />
      }
      <span
        className={cn(
          'text-[10px] font-semibold uppercase tracking-widest',
          accent ? 'text-emerald-400' : 'text-text-dim',
        )}
      >
        {label}
      </span>
      <span
        className={cn(
          'ml-auto text-[10px] font-mono tabular-nums rounded-full px-1.5 py-px',
          accent && count > 0
            ? 'bg-emerald-500/20 text-emerald-400'
            : 'bg-surface-2 text-text-dim',
        )}
      >
        {count}
      </span>
    </button>
  )
}

// ── Main component ────────────────────────────────────────────────────────────

export function ScannerPanel({
  initialInstruments = [],
  initialStrategyId = '',
  initialTimeframe = '1h',
}: ScannerPanelProps) {
  const evalPanelId = useRef(`scanner-eval-${Math.random().toString(36).slice(2)}`).current

  const [selectedStrategyId, setSelectedStrategyId] = useState<string>(initialStrategyId)
  const [timeframe, setTimeframe] = useState<string>(initialTimeframe)
  const [tiles, setTiles] = useState<WatchTileData[]>(
    initialInstruments.map((id) => ({ instrumentId: id })),
  )
  const [triggeredIds, setTriggeredIds] = useState<Set<string>>(new Set())
  const [triggeredOpen, setTriggeredOpen] = useState(true)
  const [watchingOpen, setWatchingOpen] = useState(true)

  // Mutable refs shared between render cycles without causing re-renders
  const barBuffers = useRef<Map<string, number[]>>(new Map())
  const parsedStrategy = useRef<ParsedStrategy | null>(null)

  // ── Discovery strategy list ───────────────────────────────────────────────

  const { data: discoveryStrategies = [] } = useQuery({
    queryKey: ['strategies', 'apply-list', 'discovery'],
    queryFn: () =>
      strategiesApi.applyList().then((r) =>
        (
          (r.data as { strategies?: Array<{ id: string; strategy_id: string; strategy_kind?: string }> })
            .strategies ?? []
        ).filter((s) => s.strategy_kind === 'discovery'),
      ),
  })

  // ── Fetch and parse strategy definition ──────────────────────────────────

  const { data: strategyDef } = useQuery({
    queryKey: ['strategy-def', selectedStrategyId],
    queryFn: () =>
      strategiesApi.get(selectedStrategyId).then((r) => {
        type ApiResp = {
          definition: {
            nodes: Array<
              | { id: string; type: 'condition'; expr: string }
              | { id: string; type: 'signal'; when: string; emit: string }
            >
          }
        }
        return (r.data as ApiResp).definition
      }),
    enabled: !!selectedStrategyId,
  })

  useEffect(() => {
    if (strategyDef) {
      parsedStrategy.current = parseDefinition(strategyDef)
    } else {
      parsedStrategy.current = null
    }
    barBuffers.current = new Map()
    setTriggeredIds(new Set())
  }, [strategyDef])

  // ── WS subscription + live evaluation ────────────────────────────────────

  useEffect(() => {
    if (tiles.length === 0) return

    const instruments = tiles.map((t) => t.instrumentId)
    const client = getWsClient()
    client?.subscribe(
      evalPanelId,
      instruments.map((inst) => ({ lane: 'market.bars.1m', instrument: inst })),
    )

    const unsub = wsBus.on((msg: WsOutMessage) => {
      if (msg.type !== 'frame' || msg.lane !== 'market.bars.1m') return
      const instrument = msg.instrument
      if (!instruments.includes(instrument)) return

      const payload = msg.payload as Record<string, unknown> | null
      if (!payload || typeof payload.close !== 'string') return
      const close = parseFloat(payload.close)
      if (isNaN(close)) return

      // Maintain rolling 300-bar close buffer per instrument
      const buf = barBuffers.current.get(instrument) ?? []
      const newBuf = buf.length >= 300 ? [...buf.slice(1), close] : [...buf, close]
      barBuffers.current.set(instrument, newBuf)

      const strat = parsedStrategy.current
      if (!strat) return

      const triggered = checkTriggered(strat, newBuf)
      setTriggeredIds((prev) => {
        const was = prev.has(instrument)
        if (triggered === was) return prev
        const next = new Set(prev)
        triggered ? next.add(instrument) : next.delete(instrument)
        return next
      })
    })

    return () => {
      unsub()
      client?.unsubscribe(evalPanelId)
    }
  }, [tiles, evalPanelId])

  // ── Handlers ──────────────────────────────────────────────────────────────

  const handleStrategyChange = useCallback((strategyId: string) => {
    setSelectedStrategyId(strategyId)
    setTriggeredIds(new Set())
    barBuffers.current = new Map()
  }, [])

  const handleRemove = useCallback((instrumentId: string) => {
    setTiles((prev) => prev.filter((t) => t.instrumentId !== instrumentId))
    setTriggeredIds((prev) => { const n = new Set(prev); n.delete(instrumentId); return n })
    barBuffers.current.delete(instrumentId)
  }, [])

  const TIMEFRAMES = ['1m', '5m', '15m', '30m', '1h', '4h', '1d']

  const triggered = tiles.filter((t) => triggeredIds.has(t.instrumentId))
  const watching  = tiles.filter((t) => !triggeredIds.has(t.instrumentId))

  // ── Render ────────────────────────────────────────────────────────────────

  return (
    <div className="flex flex-col h-full overflow-hidden">
      {/* Controls */}
      <div className="shrink-0 px-3 py-2 border-b border-border flex flex-col gap-1.5">
        <div className="relative">
          <select
            value={selectedStrategyId}
            onChange={(e) => handleStrategyChange(e.target.value)}
            className={cn(
              'w-full appearance-none rounded-lg px-3 py-1.5 pr-8 text-sm',
              'bg-surface-2 border border-border text-text',
              'focus:outline-none focus:ring-1 focus:ring-accent',
            )}
          >
            <option value="">Select discovery strategy…</option>
            {discoveryStrategies.map((s) => (
              <option key={s.id} value={s.id}>
                {s.strategy_id}
              </option>
            ))}
          </select>
          <ChevronDown className="pointer-events-none absolute right-2.5 top-1/2 -translate-y-1/2 h-3.5 w-3.5 text-text-dim" />
        </div>
        {/* Timeframe pills */}
        <div className="flex gap-1">
          {TIMEFRAMES.map((tf) => (
            <button
              key={tf}
              onClick={() => setTimeframe(tf)}
              className={cn(
                'flex-1 rounded px-1 py-0.5 text-[10px] font-mono font-medium transition-colors',
                tf === timeframe
                  ? 'bg-blue-500/20 text-blue-300 border border-blue-500/40'
                  : 'text-text-dim hover:text-text border border-transparent hover:border-border',
              )}
            >
              {tf}
            </button>
          ))}
        </div>
      </div>

      {/* Sections */}
      <div className="flex-1 overflow-y-auto">

        {/* ── Triggered ─────────────────────────────────────────────── */}
        <SectionHeader
          label="Triggered"
          count={triggered.length}
          open={triggeredOpen}
          onToggle={() => setTriggeredOpen((v) => !v)}
          accent
        />
        {triggeredOpen && (
          <div className="p-2 space-y-1.5">
            {triggered.length === 0 ? (
              <p className="text-[11px] text-text-dim text-center py-3">
                {selectedStrategyId
                  ? 'No signals yet — waiting for strategy to fire'
                  : 'Select a strategy above'}
              </p>
            ) : (
              triggered.map((tile) => (
                <WatchTile key={tile.instrumentId} data={tile} onRemove={handleRemove} />
              ))
            )}
          </div>
        )}

        {/* ── Watching ──────────────────────────────────────────────── */}
        <SectionHeader
          label="Watching"
          count={watching.length}
          open={watchingOpen}
          onToggle={() => setWatchingOpen((v) => !v)}
        />
        {watchingOpen && (
          <div className="p-2 space-y-1.5">
            {watching.length === 0 ? (
              <p className="text-[11px] text-text-dim text-center py-3">
                No instruments — add a scanner to watch
              </p>
            ) : (
              watching.map((tile) => (
                <WatchTile key={tile.instrumentId} data={tile} onRemove={handleRemove} />
              ))
            )}
          </div>
        )}
      </div>
    </div>
  )
}
