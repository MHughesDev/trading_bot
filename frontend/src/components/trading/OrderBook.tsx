import { useEffect, useId, useMemo, useState } from 'react'
import { Layers } from 'lucide-react'
import { getWsClient, wsBus } from '@/api/ws'
import { price as fmtPrice, qty as fmtQty, DASH } from '@/lib/format'
import { Panel, PanelBody, PanelHeader } from '@/components/primitives/Panel'
import { Label } from '@/components/primitives/Num'
import { Segmented } from '@/components/primitives/Segmented'
import { EmptyState } from '@/components/primitives/States'
import { cn } from '@/lib/utils'
import { usePrefs } from '@/store/prefs'
import { useOrderBook, type RawLevel } from '@/hooks/useOrderBook'

/* =============================================================================
   Spec §3.14 — order book.

   Three columns (price · size · cumulative total), 19px rows, tabular numerals.
   The depth bar is anchored to the RIGHT edge, width proportional to cumulative
   size. Only the price column is coloured; size and total stay primary.
   Clicking a level writes that price into the ticket.

   Feed: the `ui.orderbook.snapshot` WebSocket lane, which the gateway serves
   from `market.orderbook.l2`. When no venue depth feed is attached to the
   instrument the panel shows its designed "no depth" state rather than an
   empty grid — an empty book and an absent book are different facts.
   ============================================================================= */

export interface BookRow {
  price: number
  size: number
  total: number
}

export type BookGrouping = 1 | 5 | 10 | 50

function aggregate(levels: RawLevel[], grouping: number, dp: number, side: 'bid' | 'ask'): BookRow[] {
  if (!levels.length) return []
  const step = grouping * Math.pow(10, -dp) * 10
  const buckets = new Map<number, number>()
  for (const l of levels) {
    const p = Number(l.price)
    const s = Number(l.size)
    if (!Number.isFinite(p) || !Number.isFinite(s) || s <= 0) continue
    const key = grouping === 1 ? p : side === 'bid' ? Math.floor(p / step) * step : Math.ceil(p / step) * step
    buckets.set(key, (buckets.get(key) ?? 0) + s)
  }
  const rows = [...buckets.entries()]
    .map(([price, size]) => ({ price, size, total: 0 }))
    .sort((a, b) => (side === 'bid' ? b.price - a.price : a.price - b.price))
  let run = 0
  for (const r of rows) {
    run += r.size
    r.total = run
  }
  return rows
}

export function OrderBookPanel({
  instrument,
  lastPrice,
  onPickPrice,
  className,
}: {
  instrument: string | undefined
  lastPrice?: number | null
  /** Clicking a level writes that price into the ticket (spec §3.14). */
  onPickPrice?: (price: number) => void
  className?: string
}) {
  const [grouping, setGrouping] = useState<BookGrouping>(1)
  const depth = usePrefs((s) => s.bookDepth)
  const { bids, asks, lastUpdate } = useOrderBook(instrument, depth)

  const dp = 2
  const bidRows = useMemo(() => aggregate(bids, grouping, dp, 'bid').slice(0, 12), [bids, grouping])
  const askRows = useMemo(() => aggregate(asks, grouping, dp, 'ask').slice(0, 12), [asks, grouping])

  const maxTotal = Math.max(
    1,
    bidRows.length ? bidRows[bidRows.length - 1].total : 0,
    askRows.length ? askRows[askRows.length - 1].total : 0,
  )

  const bestBid = bidRows[0]?.price ?? null
  const bestAsk = askRows[0]?.price ?? null
  const spread = bestBid !== null && bestAsk !== null ? bestAsk - bestBid : null
  const mid = bestBid !== null && bestAsk !== null ? (bestAsk + bestBid) / 2 : null
  const hasBook = bidRows.length > 0 || askRows.length > 0

  return (
    <Panel className={className}>
      <PanelHeader
        title="Order book"
        actions={
          <div className="row" style={{ gap: 'var(--s-3)' }}>
            <Segmented
              ariaLabel="Price grouping"
              value={String(grouping) as string}
              onChange={(v) => setGrouping(Number(v) as BookGrouping)}
              options={[
                { value: '1', label: '1' },
                { value: '5', label: '5' },
                { value: '10', label: '10' },
                { value: '50', label: '50' },
              ]}
            />
            <span style={{ fontSize: 'var(--t-11)' }}>
              <Label>Spread</Label>{' '}
              <span className="num">{spread !== null ? fmtPrice(instrument, spread) : DASH}</span>
            </span>
          </div>
        }
      />
      <PanelBody flush className="clip" style={{ display: 'flex', flexDirection: 'column', overflow: 'hidden' }}>
        {!hasBook ? (
          <EmptyState
            icon={<Layers size={20} aria-hidden />}
            message="No depth feed for this instrument"
            detail="The order book streams from the venue's L2 feed. Once a depth subscription is attached to this market, bids and asks appear here live."
          />
        ) : (
          <div className="book col-flex" style={{ flex: 1, minHeight: 0 }}>
            <div className="hdr lbl">
              <span>Price</span>
              <span>Size</span>
              <span>Total</span>
            </div>

            {/* asks, highest first so the spread sits in the middle */}
            <div style={{ flex: 1, minHeight: 0, overflow: 'hidden', display: 'flex', flexDirection: 'column-reverse' }}>
              {askRows.map((r) => (
                <button
                  key={`a-${r.price}`}
                  type="button"
                  className="r ask"
                  onClick={() => onPickPrice?.(r.price)}
                  title={`Ask ${fmtPrice(instrument, r.price)} · click to use this price`}
                  style={{ border: 0, background: 'transparent', width: '100%', font: 'inherit' }}
                >
                  <i style={{ width: `${(r.total / maxTotal) * 100}%` }} aria-hidden />
                  <span>{fmtPrice(instrument, r.price)}</span>
                  <span>{fmtQty(instrument, r.size)}</span>
                  <span>{fmtQty(instrument, r.total)}</span>
                </button>
              ))}
            </div>

            <div className="mid">
              <span
                className={cn('num', lastPrice != null && mid != null && lastPrice >= mid ? 'pos' : 'neg')}
                style={{ fontSize: 'var(--t-14)', fontWeight: 'var(--w-semibold)' }}
              >
                {lastPrice != null ? fmtPrice(instrument, lastPrice) : DASH}
              </span>
              <span style={{ fontSize: 'var(--t-11)' }}>
                <Label>Mid</Label> <span className="num">{mid !== null ? fmtPrice(instrument, mid) : DASH}</span>
              </span>
            </div>

            <div style={{ flex: 1, minHeight: 0, overflow: 'hidden' }}>
              {bidRows.map((r) => (
                <button
                  key={`b-${r.price}`}
                  type="button"
                  className="r bid"
                  onClick={() => onPickPrice?.(r.price)}
                  title={`Bid ${fmtPrice(instrument, r.price)} · click to use this price`}
                  style={{ border: 0, background: 'transparent', width: '100%', font: 'inherit' }}
                >
                  <i style={{ width: `${(r.total / maxTotal) * 100}%` }} aria-hidden />
                  <span>{fmtPrice(instrument, r.price)}</span>
                  <span>{fmtQty(instrument, r.size)}</span>
                  <span>{fmtQty(instrument, r.total)}</span>
                </button>
              ))}
            </div>
          </div>
        )}
      </PanelBody>
      {hasBook && (
        <div className="panel-ft" style={{ padding: '6px var(--s-3)' }}>
          <Label>
            {lastUpdate ? `Updated ${new Date(lastUpdate).toLocaleTimeString()}` : 'Awaiting first snapshot'}
          </Label>
        </div>
      )}
    </Panel>
  )
}

/* -----------------------------------------------------------------------------
   Time & sales (the tape). Streams `market.trades`.
   -------------------------------------------------------------------------- */
/** One row of the tape. */
interface Print {
  ts: number
  price: number
  size: number
  side: 'buy' | 'sell'
}

export function Tape({ instrument }: { instrument: string | undefined }) {
  // The prints are tagged with the instrument they were printed for. `setTrades([])`
  // at the top of the subscribe effect was the reset, and it landed a render late,
  // so switching symbols showed the previous market's prints under the new
  // market's header -- on a tape, every row is a claim that a trade happened at
  // that price in this instrument.
  const [tape, setTape] = useState<{ instrument: string | undefined; prints: Print[] }>({
    instrument,
    prints: [],
  })
  const trades = tape.instrument === instrument ? tape.prints : []
  // A stable per-instance subscription id. This was `useRef(`tape-${Math.random()...}`)`,
  // which evaluated a fresh random string on *every* render and threw all but
  // the first away -- impure work during render, and the kind the compiler is
  // right to refuse. `useId` gives the same guarantee (unique per mounted
  // component, constant for its life) without rolling the dice.
  const panelId = `tape-${useId()}`

  useEffect(() => {
    if (!instrument) return
    const client = getWsClient()
    const id = panelId
    client?.subscribe(id, [{ lane: 'market.trades', instrument, max_fps: 10 }])
    const off = wsBus.on((msg) => {
      if (msg.type !== 'frame') return
      const f = msg as unknown as { lane: string; instrument: string; payload: Record<string, unknown> }
      if (f.lane !== 'market.trades' || f.instrument !== instrument) return
      const p = f.payload
      const price = Number(p.price)
      const size = Number(p.size ?? p.quantity)
      if (!Number.isFinite(price) || !Number.isFinite(size)) return
      setTape((t) => ({
        instrument,
        prints: [
          { ts: Date.now(), price, size, side: (p.side === 'sell' ? 'sell' : 'buy') as 'buy' | 'sell' },
          ...(t.instrument === instrument ? t.prints : []),
        ].slice(0, 100),
      }))
    })
    return () => {
      off()
      client?.unsubscribe(id)
    }
  }, [instrument, panelId])

  return (
    <div style={{ flex: 1, minHeight: 0, overflow: 'auto' }}>
      {trades.length === 0 ? (
        <EmptyState
          message="No prints yet"
          detail="Executed trades from the venue tape stream here as they happen."
        />
      ) : (
        <div className="tape">
          <div className="r lbl" style={{ height: 22, borderBottom: '1px solid var(--line-hairline)', position: 'sticky', top: 0, background: 'var(--bg-surface)' }}>
            <span>Price</span>
            <span>Size</span>
            <span>Time</span>
          </div>
          {trades.map((t, i) => (
            <div key={`${t.ts}-${i}`} className={cn('r', t.side)}>
              <span>{fmtPrice(instrument, t.price)}</span>
              <span>{fmtQty(instrument, t.size)}</span>
              <span className="mut">{new Date(t.ts).toLocaleTimeString(undefined, { hour12: false })}</span>
            </div>
          ))}
        </div>
      )}
    </div>
  )
}

/** Panel-wrapped tape, for the workspace where it is its own panel. */
export function TapePanel({ instrument, className }: { instrument: string | undefined; className?: string }) {
  return (
    <Panel className={className}>
      <PanelHeader title="Time &amp; sales" />
      <PanelBody flush style={{ overflow: 'hidden', display: 'flex' }}>
        <Tape instrument={instrument} />
      </PanelBody>
    </Panel>
  )
}
