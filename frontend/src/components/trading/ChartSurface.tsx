import { useMemo, useState } from 'react'
import { Bell, CandlestickChart, Columns2, Grid2x2, LayoutPanelTop, Plus, Square, Trash2 } from 'lucide-react'
import { MultiPaneChart, type IndicatorInstance, type IndicatorKind } from '@/components/charts/MultiPaneChart'
import { useBars, TIMEFRAMES, timeframeLabel } from '@/hooks/useBars'
import { Panel, PanelBody, PanelHeader } from '@/components/primitives/Panel'
import { Button, IconButton } from '@/components/primitives/Button'
import { Badge } from '@/components/primitives/Badge'
import { Label } from '@/components/primitives/Num'
import { Segmented } from '@/components/primitives/Segmented'
import { MenuItem, MenuLabel, MenuSeparator, Popover, Tooltip } from '@/components/primitives/Overlay'
import { EmptyState, ErrorState, PanelLoading } from '@/components/primitives/States'
import { NumberField } from '@/components/primitives/Field'
import { price as fmtPrice, DASH, seriesColor } from '@/lib/format'
import { cn } from '@/lib/utils'
import type { PriceLineAnnotation } from '@/components/charts/Annotations'

/* =============================================================================
   The chart surface used by both the terminal and the workspace chart panel.

   Spec §4.2: ONE chart is the default. Multi-chart is an explicit layout mode
   (1 · 2 · 2×2 · 3+1) chosen in the panel header, and the focused pane carries
   a 1px --line-accent border.
   ============================================================================= */

export type LayoutMode = '1' | '2' | '4' | '3+1'

export const LAYOUT_MODES: { value: LayoutMode; label: string; icon: typeof Square; panes: number }[] = [
  { value: '1', label: 'Single chart', icon: Square, panes: 1 },
  { value: '2', label: 'Two charts side by side', icon: Columns2, panes: 2 },
  { value: '4', label: 'Four charts', icon: Grid2x2, panes: 4 },
  { value: '3+1', label: 'One large, three small', icon: LayoutPanelTop, panes: 4 },
]

const OVERLAY_KINDS: { kind: IndicatorKind; label: string; overlay: boolean }[] = [
  { kind: 'ema', label: 'EMA', overlay: true },
  { kind: 'sma', label: 'SMA', overlay: true },
  { kind: 'bb', label: 'Bollinger bands', overlay: true },
  { kind: 'volume', label: 'Volume', overlay: false },
  { kind: 'rsi', label: 'RSI', overlay: false },
  { kind: 'macd', label: 'MACD', overlay: false },
]

export function indicatorLabel(i: IndicatorInstance): string {
  switch (i.kind) {
    case 'bb': return `BB(${i.period ?? 20}, ${i.stddev ?? 2})`
    case 'macd': return `MACD(${i.fast ?? 12}, ${i.slow ?? 26}, ${i.signal ?? 9})`
    case 'volume': return 'Volume'
    default: return `${i.kind.toUpperCase()}(${i.period ?? 20})`
  }
}

function uid(kind: string) {
  return `${kind}_${Date.now()}_${Math.random().toString(36).slice(2, 5)}`
}

/* -----------------------------------------------------------------------------
   Indicator picker
   -------------------------------------------------------------------------- */
function IndicatorMenu({
  indicators,
  onChange,
}: {
  indicators: IndicatorInstance[]
  onChange: (next: IndicatorInstance[]) => void
}) {
  const [kind, setKind] = useState<IndicatorKind>('ema')
  const [period, setPeriod] = useState('20')
  const [stddev, setStddev] = useState('2')

  function add() {
    const overlayCount = indicators.filter((i) => ['ema', 'sma', 'bb'].includes(i.kind)).length
    const inst: IndicatorInstance = {
      uid: uid(kind),
      kind,
      period: Number(period) || 20,
      stddev: kind === 'bb' ? Number(stddev) || 2 : undefined,
      fast: kind === 'macd' ? 12 : undefined,
      slow: kind === 'macd' ? 26 : undefined,
      signal: kind === 'macd' ? 9 : undefined,
      // Fixed categorical slots, assigned in order and never re-cycled (V7).
      color: ['ema', 'sma', 'bb'].includes(kind) ? seriesColorValue(overlayCount) : undefined,
    }
    onChange([...indicators, inst])
  }

  return (
    <Popover
      ariaLabel="Indicators"
      width={264}
      trigger={({ onClick, ref, 'aria-expanded': expanded }) => (
        <button
          ref={ref}
          type="button"
          className={cn('btn sm', indicators.length > 0 && 'on')}
          aria-expanded={expanded}
          onClick={onClick}
        >
          <CandlestickChart size={13} aria-hidden />
          Indicators
          {indicators.length > 0 && <span className="badge count accent">{indicators.length}</span>}
        </button>
      )}
    >
      {() => (
        <div style={{ padding: 2 }}>
          <MenuLabel>Add indicator</MenuLabel>
          {OVERLAY_KINDS.map((k) => (
            <MenuItem key={k.kind} selected={kind === k.kind} onClick={() => setKind(k.kind)}>
              {k.label}
              <span className="lbl" style={{ marginLeft: 'auto' }}>
                {k.overlay ? 'overlay' : 'pane'}
              </span>
            </MenuItem>
          ))}
          {kind !== 'volume' && (
            <div style={{ padding: '8px 8px 4px', display: 'grid', gap: 6 }}>
              <NumberField label="Period" small value={period} onValueChange={setPeriod} min={1} max={500} />
              {kind === 'bb' && (
                <NumberField label="Std dev" small value={stddev} onValueChange={setStddev} min={0.5} max={5} step={0.5} />
              )}
            </div>
          )}
          <div style={{ padding: '4px 8px 8px' }}>
            <Button size="sm" variant="primary" block icon={<Plus size={13} aria-hidden />} onClick={add}>
              Add {kind.toUpperCase()}
            </Button>
          </div>

          {indicators.length > 0 && (
            <>
              <MenuSeparator />
              <MenuLabel>Active</MenuLabel>
              {indicators.map((i) => (
                <div key={i.uid} className="menu-item" style={{ cursor: 'default' }}>
                  <i
                    className="swatch"
                    aria-hidden
                    style={{ background: i.color ?? 'var(--fg-tertiary)' }}
                  />
                  <span className="truncate-1">{indicatorLabel(i)}</span>
                  <IconButton
                    label={`Remove ${indicatorLabel(i)}`}
                    bare
                    style={{ marginLeft: 'auto', width: 20, height: 20 }}
                    onClick={() => onChange(indicators.filter((x) => x.uid !== i.uid))}
                  >
                    <Trash2 size={12} aria-hidden />
                  </IconButton>
                </div>
              ))}
              <div style={{ padding: '4px 8px 8px' }}>
                <Button size="sm" block onClick={() => onChange([])}>
                  Clear all
                </Button>
              </div>
            </>
          )}
        </div>
      )}
    </Popover>
  )
}

function seriesColorValue(index: number): string {
  return seriesColor((index % 8) + 1)
}

/* -----------------------------------------------------------------------------
   A single chart pane: bars + indicators for one instrument/timeframe.
   -------------------------------------------------------------------------- */
export function ChartPane({
  instrument,
  tfSecs,
  indicators,
  priceLines,
  height,
  showHeader,
  focused,
  onFocus,
}: {
  instrument: string
  tfSecs: number
  indicators: IndicatorInstance[]
  priceLines?: PriceLineAnnotation[]
  height?: number
  showHeader?: boolean
  focused?: boolean
  onFocus?: () => void
}) {
  const { bars, loading, error, refetch } = useBars(instrument, tfSecs)
  const last = bars.length ? bars[bars.length - 1] : null

  return (
    <div
      className={cn('chart-pane', focused && 'focused')}
      onMouseDown={onFocus}
      role={onFocus ? 'button' : undefined}
      tabIndex={onFocus ? 0 : undefined}
      aria-label={onFocus ? `Focus ${instrument} chart` : undefined}
    >
      {showHeader && (
        <div className="chart-pane-hd">
          <span style={{ fontSize: 'var(--t-12)', fontWeight: 'var(--w-semibold)' }}>{instrument}</span>
          <Label>{timeframeLabel(tfSecs)}</Label>
          <span className="spacer" />
          {last && (
            <span className={cn('num', last.close >= last.open ? 'pos' : 'neg')} style={{ fontSize: 'var(--t-12)' }}>
              {fmtPrice(instrument, last.close)}
            </span>
          )}
        </div>
      )}
      <div style={{ position: 'absolute', inset: showHeader ? '26px 0 0' : 0 }}>
        {error ? (
          <ErrorState message={`Could not load bars for ${instrument}.`} onRetry={refetch} />
        ) : loading && bars.length === 0 ? (
          <div style={{ padding: 'var(--s-4)' }}>
            <PanelLoading lines={6} />
          </div>
        ) : bars.length === 0 ? (
          <EmptyState
            icon={<CandlestickChart size={20} aria-hidden />}
            message={`No bars stored for ${instrument}`}
            detail="Initialise this instrument to seed its history, then live bars stream in."
          />
        ) : (
          <MultiPaneChart bars={bars} indicators={indicators} priceLines={priceLines} mainHeight={height ?? 320} />
        )}
      </div>
    </div>
  )
}

/* -----------------------------------------------------------------------------
   The full chart panel with header controls and layout modes.
   -------------------------------------------------------------------------- */
export function ChartSurface({
  instruments,
  focusedIndex,
  onFocusIndex,
  tfSecs,
  onTimeframe,
  indicators,
  onIndicators,
  layout,
  onLayout,
  priceLines,
  onAlert,
  className,
  title,
}: {
  instruments: string[]
  focusedIndex: number
  onFocusIndex: (i: number) => void
  tfSecs: number
  onTimeframe: (secs: number) => void
  indicators: IndicatorInstance[]
  onIndicators: (next: IndicatorInstance[]) => void
  layout: LayoutMode
  onLayout: (m: LayoutMode) => void
  priceLines?: PriceLineAnnotation[]
  onAlert?: () => void
  className?: string
  title?: string
}) {
  const mode = LAYOUT_MODES.find((m) => m.value === layout) ?? LAYOUT_MODES[0]
  const panes = useMemo(() => {
    const list = instruments.filter(Boolean)
    const out: string[] = []
    for (let i = 0; i < mode.panes; i += 1) out.push(list[i] ?? list[0] ?? '')
    return out.filter(Boolean)
  }, [instruments, mode.panes])

  const focused = panes[focusedIndex] ?? panes[0]

  return (
    <Panel className={className}>
      <PanelHeader
        title={title ?? `${focused ?? DASH} · ${timeframeLabel(tfSecs)}`}
        badges={
          indicators.length > 0 ? (
            <span className="row" style={{ gap: 4 }}>
              {indicators.slice(0, 3).map((i) => (
                <Badge key={i.uid} tone="neutral">
                  {indicatorLabel(i)}
                </Badge>
              ))}
              {indicators.length > 3 && <Badge tone="neutral">+{indicators.length - 3}</Badge>}
            </span>
          ) : undefined
        }
        actions={
          <div className="row" style={{ gap: 'var(--s-2)' }}>
            <Segmented
              ariaLabel="Timeframe"
              value={String(tfSecs)}
              onChange={(v) => onTimeframe(Number(v))}
              options={TIMEFRAMES.map((t) => ({ value: String(t.secs), label: t.label }))}
            />
            <div className="seg" role="radiogroup" aria-label="Chart layout">
              {LAYOUT_MODES.map((m) => (
                <Tooltip key={m.value} content={m.label}>
                  <button
                    type="button"
                    role="radio"
                    aria-checked={layout === m.value}
                    aria-label={m.label}
                    className={cn(layout === m.value && 'on')}
                    onClick={() => onLayout(m.value)}
                  >
                    <m.icon size={13} aria-hidden />
                  </button>
                </Tooltip>
              ))}
            </div>
            <IndicatorMenu indicators={indicators} onChange={onIndicators} />
            {onAlert && (
              <Button size="sm" icon={<Bell size={13} aria-hidden />} onClick={onAlert}>
                Alert
              </Button>
            )}
          </div>
        }
      />
      <PanelBody flush className="clip" style={{ display: 'flex', flexDirection: 'column' }}>
        {panes.length === 0 ? (
          <EmptyState message="No instrument selected" detail="Pick a market from the watchlist to chart it." />
        ) : (
          <div className={cn('chart-grid', `m${layout === '3+1' ? '31' : layout}`)}>
            {panes.map((sym, i) => (
              <ChartPane
                key={`${sym}-${i}`}
                instrument={sym}
                tfSecs={tfSecs}
                indicators={indicators}
                priceLines={i === focusedIndex ? priceLines : undefined}
                showHeader={panes.length > 1}
                focused={panes.length > 1 && i === focusedIndex}
                onFocus={panes.length > 1 ? () => onFocusIndex(i) : undefined}
              />
            ))}
          </div>
        )}
      </PanelBody>
    </Panel>
  )
}
