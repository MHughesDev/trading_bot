import { useEffect, useMemo, useState } from 'react'
import { useNavigate, useParams, useSearchParams } from 'react-router-dom'
import { ChevronDown, LayoutGrid } from 'lucide-react'
import { useModeStore } from '@/store/mode'
import { usePrefs } from '@/store/prefs'
import { usePortfolioSummary } from '@/hooks/usePortfolioSummary'
import { useBars, timeframeLabel } from '@/hooks/useBars'
import { assetClassInfo, inferAssetClass, type PlatformAssetClass } from '@/lib/assetClass'
import { DASH, compact, price as fmtPrice, signedPct, pct } from '@/lib/format'
import { WatchlistPanel, WatchStar, useInitializedAssets, useLiveQuotes } from '@/components/trading/Watchlist'
import { OrderBookPanel } from '@/components/trading/OrderBook'
import { OrderTicket } from '@/components/trading/OrderTicket'
import { TerminalDock } from '@/components/trading/TerminalDock'
import { ChartSurface, type LayoutMode } from '@/components/trading/ChartSurface'
import { AlertDialog } from '@/components/trading/AlertDialog'
import { Chip } from '@/components/primitives/Badge'
import { Button, IconButton } from '@/components/primitives/Button'
import { Label } from '@/components/primitives/Num'
import { MenuItem, MenuLabel, Popover, Tooltip } from '@/components/primitives/Overlay'
import { EmptyState } from '@/components/primitives/States'
import type { IndicatorInstance } from '@/components/charts/MultiPaneChart'
import { cn } from '@/lib/utils'

/* =============================================================================
   Spec §4.2 — the per-asset trading terminal.

   topbar 56
   instrument bar 44  [chip] SYMBOL ▾ | last +chg | bid ask hi lo vol funding
                      ⟶ [timeframe] [Indicators] [Alert]
   ┌───────────┬──────────────────────────────┬────────────────┐
   │ WATCHLIST │ PRIMARY CHART                │ ORDER TICKET   │
   │ 280       ├──────────────────────────────┤ 340            │
   ├───────────┤ DOCK 262                     │ (full height)  │
   │ ORDER BOOK│                              │                │
   └───────────┴──────────────────────────────┴────────────────┘

   This is the single-instrument instrument. The multi-panel workspace lives at
   /trading and is a different product surface with a different job.
   ============================================================================= */

const DEFAULT_SYMBOL = 'BTC-USD'

export function TerminalPage() {
  const { symbol } = useParams<{ symbol: string }>()
  const [params] = useSearchParams()
  const navigate = useNavigate()
  const { mode } = useModeStore()
  const portfolio = usePortfolioSummary()
  const assets = useInitializedAssets()

  const remembered = usePrefs.getState().rememberLastInstrument ? usePrefs.getState().lastInstrument : null
  const instrument = symbol
    ? decodeURIComponent(symbol)
    : (remembered ?? assets.data?.[0]?.symbol ?? DEFAULT_SYMBOL)
  const assetClass: PlatformAssetClass =
    (assets.data?.find((a) => a.symbol === instrument)?.asset_class as PlatformAssetClass) ??
    inferAssetClass(instrument)
  const info = assetClassInfo(assetClass)

  const prefs = usePrefs()
  const [tfSecs, setTfSecs] = useState(prefs.defaultTimeframeSecs)
  const [indicators, setIndicators] = useState<IndicatorInstance[]>([])
  const [layout, setLayout] = useState<LayoutMode>('1')
  const [focusedPane, setFocusedPane] = useState(0)
  const [pickedPrice, setPickedPrice] = useState<number | null>(null)
  const [alertOpen, setAlertOpen] = useState(false)

  // Remember the market so the terminal reopens where the user left it.
  useEffect(() => {
    if (prefs.rememberLastInstrument && instrument) prefs.set('lastInstrument', instrument)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [instrument, prefs.rememberLastInstrument])

  const quotes = useLiveQuotes([instrument])
  const quote = quotes[instrument]
  const { bars } = useBars(instrument, tfSecs, 7)

  // Session stats from the loaded bars — honest, derived, never invented.
  const stats = useMemo(() => {
    if (!bars.length) return null
    const window = bars.slice(-24)
    const high = Math.max(...window.map((b) => b.high))
    const low = Math.min(...window.map((b) => b.low))
    const vol = window.reduce((s, b) => s + (b.volume ?? 0), 0)
    const last = bars[bars.length - 1]
    const first = window[0]
    const change = first ? last.close - first.open : 0
    const changePct = first && first.open ? (change / first.open) * 100 : 0
    return { high, low, vol, last: last.close, change, changePct }
  }, [bars])

  const last = quote?.last ?? stats?.last ?? null
  const changePct = quote?.changePct ?? stats?.changePct ?? null

  // 1-6 switch timeframe (spec §5.6)
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      const el = e.target as HTMLElement | null
      if (el?.tagName === 'INPUT' || el?.tagName === 'TEXTAREA' || el?.isContentEditable) return
      const map: Record<string, number> = { '1': 60, '2': 300, '3': 900, '4': 3600, '5': 14400, '6': 86400 }
      if (map[e.key]) {
        e.preventDefault()
        setTfSecs(map[e.key])
      }
    }
    document.addEventListener('keydown', onKey)
    return () => document.removeEventListener('keydown', onKey)
  }, [])

  const paneInstruments = useMemo(() => {
    const others = (assets.data ?? []).map((a) => a.symbol).filter((s) => s !== instrument)
    return [instrument, ...others]
  }, [instrument, assets.data])

  function selectInstrument(next: string) {
    navigate(`/terminal/${encodeURIComponent(next)}${params.toString() ? `?${params}` : ''}`)
  }

  if (!instrument) {
    return (
      <EmptyState
        message="No market selected"
        detail="Initialise an instrument to start trading it."
        action={<Button variant="primary" onClick={() => navigate('/trading')}>Open the trading dashboard</Button>}
      />
    )
  }

  return (
    <>
      {/* ── INSTRUMENT BAR ─────────────────────────────────────────────────── */}
      <div className="subbar">
        <div className="inst row" style={{ gap: 'var(--s-3)', paddingRight: 'var(--s-4)', borderRight: '1px solid var(--line-hairline)' }}>
          <Chip>{info.short}</Chip>
          <Popover
            ariaLabel="Switch instrument"
            align="start"
            width={280}
            trigger={({ onClick, ref, 'aria-expanded': expanded }) => (
              <button
                ref={ref}
                type="button"
                onClick={onClick}
                aria-expanded={expanded}
                className="row"
                style={{ gap: 6, background: 'transparent', border: 0 }}
              >
                <span style={{ fontSize: 'var(--t-18)', fontWeight: 'var(--w-semibold)', letterSpacing: 'var(--tr-tight)' }}>
                  {instrument}
                </span>
                <ChevronDown size={12} className="mut" aria-hidden />
              </button>
            )}
          >
            {(close) => (
              <>
                <MenuLabel>Markets</MenuLabel>
                {(assets.data ?? []).slice(0, 24).map((a) => (
                  <MenuItem
                    key={a.symbol}
                    selected={a.symbol === instrument}
                    onClick={() => {
                      selectInstrument(a.symbol)
                      close()
                    }}
                  >
                    {a.symbol}
                    <span className="lbl" style={{ marginLeft: 'auto' }}>
                      {assetClassInfo(a.asset_class).short}
                    </span>
                  </MenuItem>
                ))}
              </>
            )}
          </Popover>
          <WatchStar symbol={instrument} />
        </div>

        <div className="row" style={{ gap: 'var(--s-3)' }}>
          <span className={cn('lastprice', changePct != null && (changePct >= 0 ? 'pos' : 'neg'))}>
            {last != null ? fmtPrice(instrument, last) : DASH}
          </span>
          <div style={{ display: 'flex', flexDirection: 'column', lineHeight: 1.2 }}>
            <span className={cn('num', stats && stats.change >= 0 ? 'pos' : 'neg')} style={{ fontSize: 'var(--t-12)' }}>
              {stats ? (stats.change >= 0 ? '+' : '−') + fmtPrice(instrument, Math.abs(stats.change)) : DASH}
            </span>
            <span className={cn('num', changePct != null && (changePct >= 0 ? 'pos' : 'neg'))} style={{ fontSize: 'var(--t-12)' }}>
              {changePct != null ? signedPct(changePct) : DASH}
            </span>
          </div>
        </div>

        <div className="statgrp">
          <div className="s">
            <Label>24h high</Label>
            <b>{stats ? fmtPrice(instrument, stats.high) : DASH}</b>
          </div>
          <div className="s">
            <Label>24h low</Label>
            <b>{stats ? fmtPrice(instrument, stats.low) : DASH}</b>
          </div>
          <div className="s">
            <Label>24h vol</Label>
            <b>{stats ? compact(stats.vol) : DASH}</b>
          </div>
          <div className="s">
            <Label>Margin used</Label>
            <b>{pct(portfolio.marginUsedPct)}</b>
          </div>
        </div>

        <div className="spacer" />

        <Tooltip content="Open the multi-panel trading dashboard">
          <IconButton label="Trading dashboard" onClick={() => navigate('/trading')}>
            <LayoutGrid size={14} aria-hidden />
          </IconButton>
        </Tooltip>
        <Label>{mode === 'LIVE' ? 'Live' : 'Paper'} · {timeframeLabel(tfSecs)}</Label>
      </div>

      {/* ── MAIN ───────────────────────────────────────────────────────────── */}
      <div className="term-main">
        <div className="term-left">
          <WatchlistPanel active={instrument} onSelect={(s) => selectInstrument(s)} />
          <OrderBookPanel
            instrument={instrument}
            lastPrice={last}
            onPickPrice={(p) => setPickedPrice(p)}
          />
        </div>

        <div className="term-center">
          <ChartSurface
            instruments={paneInstruments}
            focusedIndex={focusedPane}
            onFocusIndex={setFocusedPane}
            tfSecs={tfSecs}
            onTimeframe={setTfSecs}
            indicators={indicators}
            onIndicators={setIndicators}
            layout={layout}
            onLayout={(m) => {
              setLayout(m)
              setFocusedPane(0)
            }}
            onAlert={() => setAlertOpen(true)}
          />
          <TerminalDock instrument={instrument} assetClass={assetClass} markPrice={last} />
        </div>

        <div className="term-ticket-rail" style={{ display: 'flex', minHeight: 0 }}>
          <OrderTicket
            instrument={instrument}
            assetClass={assetClass}
            lastPrice={last}
            buyingPower={portfolio.buyingPower}
            equity={portfolio.equity}
            presetPrice={pickedPrice}
            onPresetConsumed={() => setPickedPrice(null)}
            className="flex-1"
          />
        </div>
      </div>

      <AlertDialog
        open={alertOpen}
        onClose={() => setAlertOpen(false)}
        instrument={instrument}
        lastPrice={last}
      />
    </>
  )
}
