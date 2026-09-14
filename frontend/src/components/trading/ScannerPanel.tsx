// Scanner panel — a discovery strategy run across a universe, live.
//
// Condition evaluation runs client-side against incoming WS market.bars.1m
// frames: the browser holds a rolling 300-bar close buffer per instrument and
// re-evaluates the strategy's signal conditions on every tick. That keeps the
// scanner responsive without a round trip, at the cost of only supporting the
// comparison grammar the saved v1.0 definition format uses.

import { useState, useCallback, useEffect, useRef, useMemo } from 'react'
import { useQuery } from '@tanstack/react-query'
import { Radar, X } from 'lucide-react'
import { strategiesApi } from '@/lib/api'
import { wsBus, getWsClient } from '@/api/ws'
import type { WsOutMessage } from '@/lib/types'
import { calcEMA, calcRSI } from '@/utils/indicators'
import { DASH, price as fmtPrice, signedPct } from '@/lib/format'
import { Badge, Swatch } from '@/components/primitives/Badge'
import { IconButton } from '@/components/primitives/Button'
import { Select } from '@/components/primitives/Overlay'
import { Segmented } from '@/components/primitives/Segmented'
import { EmptyState } from '@/components/primitives/States'
import { assetClassChartColor, inferAssetClass } from '@/lib/assetClass'
import { useLiveQuotes } from './Watchlist'
import { cn } from '@/lib/utils'

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

// ── Component ─────────────────────────────────────────────────────────────────

export interface ScannerPanelProps {
  initialInstruments?: string[]
  initialStrategyId?: string
  initialTimeframe?: string
}

/** The evaluator consumes 1m bars; coarser timeframes aggregate from them. */
const AGG_WINDOWS: Record<string, number> = { '1m': 1, '5m': 5, '15m': 15, '30m': 30, '1h': 60, '4h': 240 }

export function ScannerPanel({
  initialInstruments = [],
  initialStrategyId = '',
  initialTimeframe = '1h',
}: ScannerPanelProps) {
  const evalPanelId = useRef(`scanner-eval-${Math.random().toString(36).slice(2)}`).current

  const [selectedStrategyId, setSelectedStrategyId] = useState<string>(initialStrategyId)
  const [timeframe, setTimeframe] = useState<string>(initialTimeframe)
  const [instruments, setInstruments] = useState<string[]>(initialInstruments)
  const [triggeredIds, setTriggeredIds] = useState<Set<string>>(new Set())

  const barBuffers = useRef<Map<string, number[]>>(new Map())
  const tickCounters = useRef<Map<string, number>>(new Map())
  const parsedStrategy = useRef<ParsedStrategy | null>(null)

  const quotes = useLiveQuotes(instruments)

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
    parsedStrategy.current = strategyDef ? parseDefinition(strategyDef) : null
    barBuffers.current = new Map()
    tickCounters.current = new Map()
    setTriggeredIds(new Set())
  }, [strategyDef, timeframe])

  useEffect(() => {
    if (instruments.length === 0) return
    const client = getWsClient()
    client?.subscribe(
      evalPanelId,
      instruments.map((inst) => ({ lane: 'market.bars.1m', instrument: inst })),
    )

    const agg = AGG_WINDOWS[timeframe] ?? 1

    const unsub = wsBus.on((msg: WsOutMessage) => {
      if (msg.type !== 'frame' || msg.lane !== 'market.bars.1m') return
      const instrument = msg.instrument
      if (!instruments.includes(instrument)) return

      const payload = msg.payload as Record<string, unknown> | null
      const close = payload ? parseFloat(String(payload.close)) : NaN
      if (isNaN(close)) return

      // Downsample the 1m lane to the selected timeframe so the evaluator sees
      // the same bar cadence the strategy was designed against.
      const n = (tickCounters.current.get(instrument) ?? 0) + 1
      tickCounters.current.set(instrument, n)
      if (agg > 1 && n % agg !== 0) return

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
        if (triggered) next.add(instrument)
        else next.delete(instrument)
        return next
      })
    })

    return () => {
      unsub()
      client?.unsubscribe(evalPanelId)
    }
  }, [instruments, evalPanelId, timeframe])

  const handleStrategyChange = useCallback((strategyId: string) => {
    setSelectedStrategyId(strategyId)
    setTriggeredIds(new Set())
    barBuffers.current = new Map()
  }, [])

  const handleRemove = useCallback((instrumentId: string) => {
    setInstruments((prev) => prev.filter((i) => i !== instrumentId))
    setTriggeredIds((prev) => {
      const n = new Set(prev)
      n.delete(instrumentId)
      return n
    })
    barBuffers.current.delete(instrumentId)
  }, [])

  const triggered = useMemo(() => instruments.filter((i) => triggeredIds.has(i)), [instruments, triggeredIds])
  const watching = useMemo(() => instruments.filter((i) => !triggeredIds.has(i)), [instruments, triggeredIds])

  const strategyOptions = discoveryStrategies.map((s) => ({ value: s.id, label: s.strategy_id, text: s.strategy_id }))

  return (
    <>
      <div
        style={{
          flex: 'none',
          padding: 'var(--s-3)',
          borderBottom: '1px solid var(--line-hairline)',
          display: 'flex',
          flexDirection: 'column',
          gap: 'var(--s-2)',
        }}
      >
        <Select
          ariaLabel="Discovery strategy"
          small
          value={selectedStrategyId || null}
          onChange={handleStrategyChange}
          placeholder="Choose a discovery strategy…"
          options={strategyOptions}
        />
        <Segmented
          ariaLabel="Scan timeframe"
          wide
          value={timeframe}
          onChange={setTimeframe}
          options={Object.keys(AGG_WINDOWS).map((tf) => ({ value: tf, label: tf }))}
        />
      </div>

      <div style={{ flex: 1, minHeight: 0, overflow: 'auto' }}>
        {instruments.length === 0 ? (
          <EmptyState
            icon={<Radar size={20} aria-hidden />}
            message="No universe to scan"
            detail="Add markets to this scanner and pick a discovery strategy; instruments move to Triggered the moment its conditions hold."
          />
        ) : !selectedStrategyId ? (
          <EmptyState
            icon={<Radar size={20} aria-hidden />}
            message="Pick a discovery strategy"
            detail={`${instruments.length} markets are streaming. Choose a strategy above to start evaluating them.`}
          />
        ) : (
          <>
            <ScannerSection label="Triggered" count={triggered.length} accent>
              {triggered.length === 0 ? (
                <div className="mut" style={{ padding: 'var(--s-3) var(--s-4)', fontSize: 'var(--t-12)' }}>
                  Nothing has met the conditions yet.
                </div>
              ) : (
                triggered.map((i) => (
                  <ScannerRow key={i} symbol={i} quote={quotes[i]} triggered onRemove={handleRemove} />
                ))
              )}
            </ScannerSection>

            <ScannerSection label="Watching" count={watching.length}>
              {watching.map((i) => (
                <ScannerRow key={i} symbol={i} quote={quotes[i]} onRemove={handleRemove} />
              ))}
            </ScannerSection>
          </>
        )}
      </div>
    </>
  )
}

function ScannerSection({
  label,
  count,
  accent,
  children,
}: {
  label: string
  count: number
  accent?: boolean
  children: React.ReactNode
}) {
  const [open, setOpen] = useState(true)
  return (
    <div>
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        className="row"
        style={{
          width: '100%',
          padding: '6px var(--s-3)',
          borderBottom: '1px solid var(--line-hairline)',
          background: 'transparent',
          textAlign: 'left',
        }}
      >
        <span className={cn('lbl', accent && count > 0 && 'pos')}>{label}</span>
        <span className="spacer" />
        <Badge tone={accent && count > 0 ? 'pos' : 'neutral'}>{count}</Badge>
      </button>
      {open && children}
    </div>
  )
}

function ScannerRow({
  symbol,
  quote,
  triggered,
  onRemove,
}: {
  symbol: string
  quote?: { last: number | null; changePct: number | null }
  triggered?: boolean
  onRemove: (s: string) => void
}) {
  return (
    <div
      className="row"
      style={{
        padding: '7px var(--s-3)',
        borderBottom: '1px solid var(--line-hairline)',
        gap: 'var(--s-2)',
        background: triggered ? 'var(--bg-pos-subtle)' : undefined,
      }}
    >
      <Swatch color={assetClassChartColor(inferAssetClass(symbol))} />
      <span style={{ fontSize: 'var(--t-12)', fontWeight: 'var(--w-semibold)' }} className="truncate-1">
        {symbol}
      </span>
      <span className="spacer" />
      <span className="num" style={{ fontSize: 'var(--t-12)' }}>
        {quote?.last != null ? fmtPrice(symbol, quote.last) : DASH}
      </span>
      <span
        className={cn('num', quote?.changePct == null ? 'mut' : quote.changePct >= 0 ? 'pos' : 'neg')}
        style={{ fontSize: 'var(--t-11)', width: 58, textAlign: 'right' }}
      >
        {quote?.changePct == null ? DASH : signedPct(quote.changePct)}
      </span>
      <IconButton label={`Remove ${symbol}`} bare onClick={() => onRemove(symbol)}>
        <X size={12} aria-hidden />
      </IconButton>
    </div>
  )
}

