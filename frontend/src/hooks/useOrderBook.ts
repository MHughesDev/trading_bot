import { useEffect, useId, useState } from 'react'
import { getWsClient, wsBus } from '@/api/ws'

/** An empty book, plus the instrument it is empty *for*. */
const EMPTY = { bids: [] as RawLevel[], asks: [] as RawLevel[], lastUpdate: null as number | null, subscribed: false }

interface Book {
  /** Which instrument these levels belong to. Absent until the first frame. */
  instrument: string | undefined
  bids: RawLevel[]
  asks: RawLevel[]
  lastUpdate: number | null
  subscribed: boolean
}

export function useOrderBook(instrument: string | undefined, depth = 25) {
  // One value, tagged with its instrument, rather than four that an effect had to
  // clear together. The four `setState` calls at the top of the subscribe effect
  // were the reset, and they landed a render late: switching instruments drew one
  // frame of the *previous* symbol's bids and asks under the new symbol's header,
  // at a depth and a spread that were not this market's. Tagging the book and
  // checking the tag on read makes that frame impossible instead of unlikely.
  const [book, setBook] = useState<Book>({ instrument, ...EMPTY })
  // A stable per-instance subscription id. This was `useRef(`book-${Math.random()...}`)`,
  // which evaluated a fresh random string on *every* render and threw all but
  // the first away -- impure work during render, and the kind the compiler is
  // right to refuse. `useId` gives the same guarantee (unique per mounted
  // component, constant for its life) without rolling the dice.
  const panelId = `book-${useId()}`

  useEffect(() => {
    if (!instrument) return
    const client = getWsClient()
    const id = panelId

    client?.subscribe(id, [{ lane: 'ui.orderbook.snapshot', instrument, depth, max_fps: 6 }])

    /** Whatever we hold for *this* instrument — an empty book if we hold another's. */
    const mine = (b: Book): Book => (b.instrument === instrument ? b : { instrument, ...EMPTY })

    const off = wsBus.on((msg) => {
      if ((msg as { type: string }).type === 'subscribed') {
        const m = msg as unknown as { lane: string; instrument: string }
        if (m.lane === 'ui.orderbook.snapshot' && m.instrument === instrument)
          setBook((b) => ({ ...mine(b), subscribed: true }))
        return
      }
      if (msg.type !== 'frame') return
      const f = msg as unknown as { lane: string; instrument: string; payload: BookPayload }
      if (f.lane !== 'ui.orderbook.snapshot' || f.instrument !== instrument) return
      const p = f.payload
      setBook((b) => {
        const prev = mine(b)
        return {
          ...prev,
          bids: Array.isArray(p?.bids) ? p.bids : prev.bids,
          asks: Array.isArray(p?.asks) ? p.asks : prev.asks,
          lastUpdate: Date.now(),
        }
      })
    })

    return () => {
      off()
      client?.unsubscribe(id)
    }
  }, [instrument, depth, panelId])

  // A book carrying another instrument's tag is not this instrument's book, and
  // reads as absent rather than as thin liquidity.
  const shown = book.instrument === instrument ? book : EMPTY
  return { bids: shown.bids, asks: shown.asks, lastUpdate: shown.lastUpdate, subscribed: shown.subscribed }
}

export interface RawLevel {
  price: string
  size: string
}

interface BookPayload {
  kind?: 'snapshot' | 'delta'
  bids?: RawLevel[]
  asks?: RawLevel[]
  sequence?: number
}
