// Chart annotation types and helpers for TP/SL price lines and order markers.
//
// Annotations are passed into OhlcvChart (or used directly via its extended API)
// to draw bracket order lines and fill markers on the candle chart.

export type AnnotationLineStyle = 'dashed' | 'solid' | 'dotted'

export interface PriceLineAnnotation {
  id: string
  price: number
  color: string
  lineStyle?: AnnotationLineStyle
  label?: string
}

export interface OrderAnnotation {
  /** unix timestamp (seconds) */
  time: number
  price: number
  side: 'buy' | 'sell'
  /** 'entry' | 'tp' | 'sl' | 'fill' */
  kind: 'entry' | 'tp' | 'sl' | 'fill'
  qty?: number
  tpPrice?: number
  slPrice?: number
}

export interface ChartAnnotations {
  priceLines: PriceLineAnnotation[]
  orderMarkers: OrderAnnotation[]
}

export function emptyAnnotations(): ChartAnnotations {
  return { priceLines: [], orderMarkers: [] }
}

/** Build PriceLineAnnotations from a bracket order's TP and SL levels. */
export function bracketToLines(
  orderId: string,
  tpPrice?: number,
  slPrice?: number,
): PriceLineAnnotation[] {
  const lines: PriceLineAnnotation[] = []
  if (tpPrice != null) {
    lines.push({
      id: `${orderId}-tp`,
      price: tpPrice,
      color: 'var(--tb-pnl-up, #22c55e)',
      lineStyle: 'dashed',
      label: 'TP',
    })
  }
  if (slPrice != null) {
    lines.push({
      id: `${orderId}-sl`,
      price: slPrice,
      color: 'var(--tb-pnl-down, #ef4444)',
      lineStyle: 'dashed',
      label: 'SL',
    })
  }
  return lines
}
