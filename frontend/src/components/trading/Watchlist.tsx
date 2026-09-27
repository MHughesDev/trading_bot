import { useEffect, useMemo, useRef, useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { Search, Star } from 'lucide-react'
import { assetApi } from '@/lib/api'
import { getWsClient, wsBus } from '@/api/ws'
import { useWatchlistStore } from '@/store/watchlist'
import { assetClassChartColor, assetClassInfo, inferAssetClass, type PlatformAssetClass } from '@/lib/assetClass'
import { DASH, price as fmtPrice, signedPct } from '@/lib/format'
import { Panel, PanelBody, PanelHeader } from '@/components/primitives/Panel'
import { IconButton } from '@/components/primitives/Button'
import { Swatch } from '@/components/primitives/Badge'
import { SearchInput } from '@/components/primitives/Field'
import { EmptyState, NoResults } from '@/components/primitives/States'
import { cn } from '@/lib/utils'
import { useLiveQuotes, useInitializedAssets } from '@/hooks/useLiveQuotes'

/* Spec §3.15 — watchlist row: identity on the left, price over change on the
   right. Grouped by asset class with a swatch + label header. Active row is
   --bg-selected with a 1px --line-accent inset. */

export interface Quote {
  last: number | null
  prevClose: number | null
  changePct: number | null
  updatedAt: number | null
}

export function WatchlistPanel({
  active,
  onSelect,
  className,
  compact,
}: {
  active?: string
  onSelect: (symbol: string, assetClass: PlatformAssetClass) => void
  className?: string
  compact?: boolean
}) {
  const [q, setQ] = useState('')
  const assets = useInitializedAssets()
  const watch = useWatchlistStore()
  const searchRef = useRef<HTMLDivElement>(null)

  // `/` focuses the watchlist search (spec §5.6).
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      const el = e.target as HTMLElement | null
      if (el?.tagName === 'INPUT' || el?.tagName === 'TEXTAREA' || el?.isContentEditable) return
      if (e.key === '/') {
        e.preventDefault()
        searchRef.current?.querySelector('input')?.focus()
      }
    }
    document.addEventListener('keydown', onKey)
    return () => document.removeEventListener('keydown', onKey)
  }, [])

  const rows = useMemo(() => {
    const list = assets.data ?? []
    const merged = new Map<string, PlatformAssetClass>()
    for (const a of list) merged.set(a.symbol, (a.asset_class as PlatformAssetClass) ?? inferAssetClass(a.symbol))
    for (const s of watch.symbols) if (!merged.has(s)) merged.set(s, inferAssetClass(s))
    return [...merged.entries()].map(([symbol, assetClass]) => ({ symbol, assetClass }))
  }, [assets.data, watch.symbols])

  const filtered = useMemo(() => {
    const t = q.trim().toLowerCase()
    return t ? rows.filter((r) => r.symbol.toLowerCase().includes(t)) : rows
  }, [rows, q])

  const quotes = useLiveQuotes(filtered.slice(0, 40).map((r) => r.symbol))

  const grouped = useMemo(() => {
    const g = new Map<PlatformAssetClass, typeof filtered>()
    for (const r of filtered) {
      const arr = g.get(r.assetClass) ?? []
      arr.push(r)
      g.set(r.assetClass, arr)
    }
    return [...g.entries()].sort((a, b) => {
      const sa = assetClassInfo(a[0]).slot ?? 99
      const sb = assetClassInfo(b[0]).slot ?? 99
      return sa - sb
    })
  }, [filtered])

  return (
    <Panel className={className}>
      <PanelHeader
        title="Watchlist"
        actions={
          <span className="lbl">
            {filtered.length} {filtered.length === 1 ? 'market' : 'markets'}
          </span>
        }
      />
      <div ref={searchRef} style={{ padding: 'var(--s-3) var(--s-3) 0' }}>
        <SearchInput value={q} onValueChange={setQ} placeholder="Search markets  /" aria-label="Search markets" />
      </div>
      <PanelBody flush style={{ overflow: 'auto', paddingTop: 'var(--s-1)' }}>
        {assets.isLoading ? (
          <div style={{ padding: 'var(--s-3)' }}>
            {Array.from({ length: 6 }).map((_, i) => (
              <div key={i} className="skel" style={{ height: 30, margin: '6px 0' }} />
            ))}
          </div>
        ) : filtered.length === 0 ? (
          q ? (
            <NoResults query={q} onClear={() => setQ('')} />
          ) : (
            <EmptyState
              icon={<Search size={20} aria-hidden />}
              message="No markets initialised"
              detail="Initialise an instrument to start streaming its bars, then it appears here."
            />
          )
        ) : (
          grouped.map(([cls, items]) => (
            <div key={cls}>
              <div className="wl-grp">
                <Swatch color={assetClassChartColor(cls)} />
                <span className="lbl">{assetClassInfo(cls).label}</span>
              </div>
              {items.map((r) => {
                const quote = quotes[r.symbol]
                const on = active === r.symbol
                return (
                  <button
                    key={r.symbol}
                    type="button"
                    className={cn('wl-row', on && 'on')}
                    onClick={() => onSelect(r.symbol, r.assetClass)}
                    aria-current={on || undefined}
                  >
                    <span className="sym-stack">
                      <span style={{ fontSize: 'var(--t-13)', fontWeight: 'var(--w-semibold)' }} className="truncate-1">
                        {r.symbol.split('-')[0]}
                      </span>
                      {!compact && (
                        <span className="mut truncate-1" style={{ fontSize: 'var(--t-11)' }}>
                          {r.symbol}
                        </span>
                      )}
                    </span>
                    <span className="r">
                      <span style={{ fontSize: 'var(--t-12)', fontWeight: 'var(--w-medium)', display: 'block' }}>
                        {quote?.last != null ? fmtPrice(r.symbol, quote.last) : DASH}
                      </span>
                      <span
                        className={cn(
                          'truncate-1',
                          quote?.changePct == null ? 'mut' : quote.changePct >= 0 ? 'pos' : 'neg',
                        )}
                        style={{ fontSize: 'var(--t-11)', display: 'block' }}
                      >
                        {quote?.changePct == null ? DASH : signedPct(quote.changePct)}
                      </span>
                    </span>
                  </button>
                )
              })}
            </div>
          ))
        )}
      </PanelBody>
    </Panel>
  )
}

/** The star used on the instrument bar to pin a market to the watchlist. */
export function WatchStar({ symbol }: { symbol: string }) {
  const watch = useWatchlistStore()
  const on = watch.has(symbol)
  return (
    <IconButton
      label={on ? `Remove ${ symbol } from watchlist` : `Add ${symbol} to watchlist`}
      active={on}
      bare
      onClick={() => watch.toggle(symbol)}
    >
      <Star size={14} aria-hidden fill={on ? 'currentColor' : 'none'} />
    </IconButton>
  )
}

