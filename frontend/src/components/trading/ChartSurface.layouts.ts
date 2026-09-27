import { Columns2, Grid2x2, LayoutPanelTop, Square } from 'lucide-react'
import type { IndicatorInstance } from '@/components/charts/MultiPaneChart'
import type { LayoutMode } from './ChartSurface'

export const LAYOUT_MODES: { value: LayoutMode; label: string; icon: typeof Square; panes: number }[] = [
  { value: '1', label: 'Single chart', icon: Square, panes: 1 },
  { value: '2', label: 'Two charts side by side', icon: Columns2, panes: 2 },
  { value: '4', label: 'Four charts', icon: Grid2x2, panes: 4 },
  { value: '3+1', label: 'One large, three small', icon: LayoutPanelTop, panes: 4 },
]

export function indicatorLabel(i: IndicatorInstance): string {
  switch (i.kind) {
    case 'bb': return `BB(${i.period ?? 20}, ${i.stddev ?? 2})`
    case 'macd': return `MACD(${i.fast ?? 12}, ${i.slow ?? 26}, ${i.signal ?? 9})`
    case 'volume': return 'Volume'
    default: return `${i.kind.toUpperCase()}(${i.period ?? 20})`
  }
}
