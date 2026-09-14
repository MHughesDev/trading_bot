import { useMemo, useState } from 'react'
import { useNavigate } from 'react-router-dom'
import { LayoutTemplate, Maximize2, Plus, RotateCcw } from 'lucide-react'
import { useModeStore } from '@/store/mode'
import { useWorkspaceStore } from '@/store/workspace'
import { usePortfolioSummary, flattenPositions } from '@/hooks/usePortfolioSummary'
import {
  layoutTemplates,
  panelKindInfo,
  PANEL_KINDS,
  type PanelKind,
  type PanelSpec,
} from '@/config/layoutTemplates'
import { assetClassChartColor, assetClassInfo, inferAssetClass, type PlatformAssetClass } from '@/lib/assetClass'
import { DASH, money, price as fmtPrice } from '@/lib/format'
import { WorkspacePanel } from '@/components/trading/WorkspacePanel'
import { WatchlistPanel, useInitializedAssets, useLiveQuotes } from '@/components/trading/Watchlist'
import { OrderBookPanel, Tape } from '@/components/trading/OrderBook'
import { OrderTicket } from '@/components/trading/OrderTicket'
import { ScannerPanel } from '@/components/trading/ScannerPanel'
import { ChartPane, indicatorLabel } from '@/components/trading/ChartSurface'
import { Button, IconButton } from '@/components/primitives/Button'
import { Badge, Swatch } from '@/components/primitives/Badge'
import { Label, Pnl, Pct } from '@/components/primitives/Num'
import { Segmented } from '@/components/primitives/Segmented'
import { MenuItem, MenuLabel, MenuSeparator, Popover, Tooltip } from '@/components/primitives/Overlay'
import { Table, IdentityCell, TableScroll } from '@/components/primitives/Table'
import { EmptyState } from '@/components/primitives/States'
import { SearchInput } from '@/components/primitives/Field'
import { TIMEFRAMES, timeframeLabel } from '@/hooks/useBars'
import type { IndicatorInstance } from '@/components/charts/MultiPaneChart'

/* =============================================================================
   The trading dashboard — the desk.

   Many markets, many views, arranged by the user. Panels are added from one
   menu, dragged to reorder, resized and collapsed, and all of it persists per
   trading mode. Named layouts switch the whole desk at once.

   The per-asset terminal (/terminal/:symbol) is the other half of the product:
   one market, full depth, a fixed expert layout. Both are first-class.
   ============================================================================= */

export function TradingPage() {
  const { mode } = useModeStore()
  const navigate = useNavigate()
  const ws = useWorkspaceStore()
  const workspace = ws.byMode[mode]
  const panels = workspace.panels
  const assets = useInitializedAssets()
  const portfolio = usePortfolioSummary()

  const [dragIndex, setDragIndex] = useState<number | null>(null)
  const [dragOverIndex, setDragOverIndex] = useState<number | null>(null)

  const symbols = useMemo(
    () => Array.from(new Set(panels.map((p) => p.instrument).filter(Boolean) as string[])),
    [panels],
  )
  const quotes = useLiveQuotes(symbols)

  function addPanel(kind: PanelKind, instrument?: string, assetClass?: string) {
    const info = panelKindInfo(kind)
    ws.setPanels(mode, (prev) => [
      ...prev,
      {
        id: `${kind}-${Date.now()}`,
        kind,
        instrument,
        assetClass: assetClass ?? (instrument ? inferAssetClass(instrument) : undefined),
        venue: 'kraken',
        width: info.defaultWidth,
      },
    ])
  }

  function removePanel(id: string) {
    ws.setPanels(mode, (prev) => prev.filter((p) => p.id !== id))
    ws.removeChartSettings(mode, id)
  }

  function reorder(from: number, to: number) {
    ws.setPanels(mode, (prev) => {
      const next = [...prev]
      const [moved] = next.splice(from, 1)
      next.splice(to, 0, moved)
      return next
    })
  }

  return (
    <>
      <div className="pagehead">
        <h1 className="h1">Trading desk</h1>
        <Badge tone="neutral">{panels.length} panels</Badge>
        <div className="spacer" />

        <span className="row" style={{ gap: 'var(--s-4)', marginRight: 'var(--s-3)', fontSize: 'var(--t-12)' }}>
          <span>
            <Label>Equity</Label> <span className="num">{money(portfolio.equity)}</span>
          </span>
          <span>
            <Label>Open P&L</Label> <Pnl value={portfolio.unrealized} />
          </span>
          <span>
            <Label>Positions</Label> <span className="num">{portfolio.openPositions}</span>
          </span>
        </span>

        <Popover
          ariaLabel="Layout templates"
          width={300}
          trigger={({ onClick, ref, 'aria-expanded': expanded }) => (
            <button ref={ref} type="button" className="btn sm" aria-expanded={expanded} onClick={onClick}>
              <LayoutTemplate size={13} aria-hidden />
              Layouts
            </button>
          )}
        >
          {(close) => (
            <>
              <MenuLabel>Replace the desk with</MenuLabel>
              {Object.values(layoutTemplates).map((t) => (
                <button
                  key={t.id}
                  type="button"
                  className="menu-item"
                  style={{ height: 'auto', alignItems: 'flex-start', flexDirection: 'column', gap: 2, padding: '8px' }}
                  onClick={() => {
                    ws.applyTemplate(mode, t.panels.map((p) => ({ ...p, id: `${p.id}-${Date.now()}` })))
                    close()
                  }}
                >
                  <span style={{ fontWeight: 'var(--w-semibold)', color: 'var(--fg-primary)' }}>{t.name}</span>
                  <span className="mut" style={{ fontSize: 'var(--t-11)', whiteSpace: 'normal', lineHeight: 1.4 }}>
                    {t.description}
                  </span>
                </button>
              ))}
              <MenuSeparator />
              <MenuItem
                icon={<RotateCcw size={13} />}
                onClick={() => {
                  ws.applyTemplate(mode, layoutTemplates.default.panels)
                  close()
                }}
              >
                Reset to default desk
              </MenuItem>
            </>
          )}
        </Popover>

        <AddPanelMenu assets={assets.data ?? []} onAdd={addPanel} />
      </div>

      <div className="ws-scroll">
        {panels.length === 0 && (
          <div style={{ flex: 1, display: 'grid', placeItems: 'center' }}>
            <EmptyState
              icon={<Plus size={20} aria-hidden />}
              message="Your desk is empty"
              detail="Add a chart, a watchlist or a scanner — or start from a named layout."
              action={
                <Button variant="primary" onClick={() => ws.applyTemplate(mode, layoutTemplates.default.panels)}>
                  Use the default desk
                </Button>
              }
            />
          </div>
        )}

        {panels.map((panel, index) => {
          const info = panelKindInfo(panel.kind)
          const quote = panel.instrument ? quotes[panel.instrument] : undefined
          return (
            <WorkspacePanel
              key={panel.id}
              title={info.label}
              subtitle={
                panel.instrument ? (
                  <span className="row" style={{ gap: 6 }}>
                    <Swatch color={assetClassChartColor(panel.assetClass)} />
                    {panel.instrument}
                    {quote?.last != null && (
                      <span className={quote.changePct != null && quote.changePct >= 0 ? 'pos num' : 'neg num'}>
                        {fmtPrice(panel.instrument, quote.last)}
                      </span>
                    )}
                  </span>
                ) : undefined
              }
              width={panel.width ?? info.defaultWidth}
              collapsed={panel.collapsed}
              onWidthChange={(w) => ws.patchPanel(mode, panel.id, { width: w })}
              onCollapsedChange={(c) => ws.patchPanel(mode, panel.id, { collapsed: c })}
              onClose={() => removePanel(panel.id)}
              actions={
                panel.instrument ? (
                  <Tooltip content={`Open ${panel.instrument} in the full terminal`}>
                    <IconButton
                      label={`Open ${panel.instrument} in the terminal`}
                      bare
                      onClick={() => navigate(`/terminal/${encodeURIComponent(panel.instrument!)}`)}
                    >
                      <Maximize2 size={12} aria-hidden />
                    </IconButton>
                  </Tooltip>
                ) : undefined
              }
              isDragging={dragIndex === index}
              isDragOver={dragOverIndex === index && dragIndex !== index}
              onDragStart={(e) => {
                setDragIndex(index)
                e.dataTransfer.effectAllowed = 'move'
              }}
              onDragOver={(e) => {
                e.preventDefault()
                e.dataTransfer.dropEffect = 'move'
                setDragOverIndex(index)
              }}
              onDrop={() => {
                if (dragIndex !== null && dragIndex !== index) reorder(dragIndex, index)
                setDragIndex(null)
                setDragOverIndex(null)
              }}
              onDragEnd={() => {
                setDragIndex(null)
                setDragOverIndex(null)
              }}
            >
              <PanelBodyFor
                panel={panel}
                onPickInstrument={(sym, cls) => ws.patchPanel(mode, panel.id, { instrument: sym, assetClass: cls })}
              />
            </WorkspacePanel>
          )
        })}

        {panels.length > 0 && (
          <div className="ws-add">
            <AddPanelMenu assets={assets.data ?? []} onAdd={addPanel} inline />
          </div>
        )}
      </div>
    </>
  )
}

/* -----------------------------------------------------------------------------
   Add panel — one menu for every panel kind, with an instrument picker for the
   kinds that need one.
   -------------------------------------------------------------------------- */
function AddPanelMenu({
  assets,
  onAdd,
  inline,
}: {
  assets: { symbol: string; asset_class: string }[]
  onAdd: (kind: PanelKind, instrument?: string, assetClass?: string) => void
  inline?: boolean
}) {
  const [kind, setKind] = useState<PanelKind | null>(null)
  const [q, setQ] = useState('')

  const filtered = useMemo(() => {
    const t = q.trim().toLowerCase()
    return t ? assets.filter((a) => a.symbol.toLowerCase().includes(t)) : assets
  }, [assets, q])

  return (
    <Popover
      ariaLabel="Add panel"
      width={300}
      trigger={({ onClick, ref, 'aria-expanded': expanded }) =>
        inline ? (
          <button ref={ref} type="button" className="ws-add-btn" aria-expanded={expanded} onClick={onClick}>
            <Plus size={14} aria-hidden />
            Add panel
          </button>
        ) : (
          <button ref={ref} type="button" className="btn sm primary" aria-expanded={expanded} onClick={onClick}>
            <Plus size={13} aria-hidden />
            Add panel
          </button>
        )
      }
    >
      {(close) => (
        <>
          {kind === null ? (
            <>
              <MenuLabel>Add a panel</MenuLabel>
              {PANEL_KINDS.map((k) => (
                <button
                  key={k.kind}
                  type="button"
                  className="menu-item"
                  style={{ height: 'auto', alignItems: 'flex-start', padding: 8 }}
                  onClick={() => {
                    if (k.needsInstrument) setKind(k.kind)
                    else {
                      onAdd(k.kind)
                      close()
                    }
                  }}
                >
                  <k.icon size={14} aria-hidden style={{ marginTop: 2, flex: 'none' }} />
                  <span style={{ display: 'flex', flexDirection: 'column', gap: 2, minWidth: 0 }}>
                    <span style={{ fontWeight: 'var(--w-medium)', color: 'var(--fg-primary)' }}>{k.label}</span>
                    <span className="mut" style={{ fontSize: 'var(--t-11)', whiteSpace: 'normal', lineHeight: 1.4 }}>
                      {k.description}
                    </span>
                  </span>
                </button>
              ))}
            </>
          ) : (
            <>
              <div className="row" style={{ padding: '6px 8px' }}>
                <Button size="xs" variant="ghost" onClick={() => setKind(null)}>
                  {'←'} Back
                </Button>
                <span className="lbl" style={{ marginLeft: 'auto' }}>
                  {panelKindInfo(kind).label}
                </span>
              </div>
              <div style={{ padding: '0 8px 6px' }}>
                <SearchInput value={q} onValueChange={setQ} placeholder="Search markets" autoFocus />
              </div>
              {filtered.length === 0 ? (
                <div className="empty" style={{ minHeight: 90 }}>
                  <span className="msg">No markets match</span>
                </div>
              ) : (
                filtered.slice(0, 40).map((a) => (
                  <MenuItem
                    key={a.symbol}
                    icon={<Swatch color={assetClassChartColor(a.asset_class)} />}
                    onClick={() => {
                      onAdd(kind, a.symbol, a.asset_class)
                      setKind(null)
                      setQ('')
                      close()
                    }}
                  >
                    {a.symbol}
                    <span className="lbl" style={{ marginLeft: 'auto' }}>
                      {assetClassInfo(a.asset_class).short}
                    </span>
                  </MenuItem>
                ))
              )}
            </>
          )}
        </>
      )}
    </Popover>
  )
}

/* -----------------------------------------------------------------------------
   Panel bodies
   -------------------------------------------------------------------------- */
function PanelBodyFor({
  panel,
  onPickInstrument,
}: {
  panel: PanelSpec
  onPickInstrument: (symbol: string, assetClass: PlatformAssetClass) => void
}) {
  const { mode } = useModeStore()
  const ws = useWorkspaceStore()
  const portfolio = usePortfolioSummary()
  const settings = ws.byMode[mode].chartSettings[panel.id]
  const [pickedPrice, setPickedPrice] = useState<number | null>(null)
  const quotes = useLiveQuotes(panel.instrument ? [panel.instrument] : [])
  const last = panel.instrument ? (quotes[panel.instrument]?.last ?? null) : null

  const tfSecs = settings?.tfSecs ?? 3600
  const indicators = settings?.indicators ?? []

  function setSettings(next: { tfSecs?: number; indicators?: IndicatorInstance[] }) {
    ws.setChartSettings(mode, panel.id, {
      tfSecs: next.tfSecs ?? tfSecs,
      indicators: next.indicators ?? indicators,
    })
  }

  if (panel.kind !== 'watchlist' && panel.kind !== 'scanner' && panel.kind !== 'positions' &&
      panel.kind !== 'orders' && panel.kind !== 'automations' && !panel.instrument) {
    return (
      <EmptyState
        message="No market chosen"
        detail="Pick a market for this panel."
      />
    )
  }

  switch (panel.kind) {
    case 'chart':
      return (
        <>
          <div className="row" style={{ padding: '6px var(--s-3)', borderBottom: '1px solid var(--line-hairline)', gap: 'var(--s-2)' }}>
            <Segmented
              ariaLabel="Timeframe"
              value={String(tfSecs)}
              onChange={(v) => setSettings({ tfSecs: Number(v) })}
              options={TIMEFRAMES.map((t) => ({ value: String(t.secs), label: t.label }))}
            />
            <span className="spacer" />
            {indicators.slice(0, 2).map((i) => (
              <Badge key={i.uid} tone="neutral">
                {indicatorLabel(i)}
              </Badge>
            ))}
            <span className="lbl">{timeframeLabel(tfSecs)}</span>
          </div>
          <div style={{ flex: 1, minHeight: 0, position: 'relative' }}>
            <ChartPane instrument={panel.instrument!} tfSecs={tfSecs} indicators={indicators} />
          </div>
        </>
      )

    case 'ticket':
      return (
        <div style={{ flex: 1, minHeight: 0, overflow: 'auto', display: 'flex' }}>
          <OrderTicketBare
            instrument={panel.instrument!}
            assetClass={(panel.assetClass as PlatformAssetClass) ?? 'crypto_spot_cex'}
            lastPrice={last}
            buyingPower={portfolio.buyingPower}
            equity={portfolio.equity}
            presetPrice={pickedPrice}
            onPresetConsumed={() => setPickedPrice(null)}
          />
        </div>
      )

    case 'book':
      return (
        <div style={{ flex: 1, minHeight: 0, display: 'flex' }}>
          <OrderBookPanel instrument={panel.instrument} lastPrice={last} className="flex-1" />
        </div>
      )

    case 'tape':
      return <Tape instrument={panel.instrument} />

    case 'watchlist':
      return (
        <div style={{ flex: 1, minHeight: 0, display: 'flex' }}>
          <WatchlistPanel
            active={panel.instrument}
            onSelect={(sym, cls) => onPickInstrument(sym, cls)}
            className="flex-1"
            compact
          />
        </div>
      )

    case 'scanner':
      return (
        <ScannerPanel
          initialInstruments={panel.instruments}
          initialStrategyId={panel.strategyId}
          initialTimeframe={panel.timeframe}
        />
      )

    case 'positions':
      return <PositionsBody />

    case 'orders':
      return (
        <EmptyState
          message="No working orders"
          detail="A cross-venue order feed is not served yet. Working orders for a single market are in that market's terminal dock."
        />
      )

    case 'automations':
      return <AutomationsBody />

    default:
      return null
  }
}

/** Ticket without its own Panel chrome — the workspace panel already provides it. */
function OrderTicketBare(props: React.ComponentProps<typeof OrderTicket>) {
  return (
    <div style={{ flex: 1, minWidth: 0, display: 'flex' }}>
      <OrderTicket {...props} className="flex-1" />
    </div>
  )
}

function PositionsBody() {
  const s = usePortfolioSummary()
  const navigate = useNavigate()
  const positions = useMemo(() => flattenPositions(s.rows), [s.rows])

  return (
    <TableScroll>
      <Table
        caption="Open positions"
        rows={positions}
        rowKey={(p) => p.key}
        loading={s.loading}
        dense
        onRowClick={(p) => navigate(`/terminal/${encodeURIComponent(p.instrumentId)}`)}
        empty={<EmptyState message="No open positions" detail="Positions appear here the moment a fill lands." />}
        columns={[
          {
            key: 'inst',
            header: 'Instrument',
            align: 'left',
            numeric: false,
            sortValue: (p) => p.instrumentId,
            cell: (p) => (
              <IdentityCell
                swatch={<Swatch color={assetClassChartColor(p.assetClass)} />}
                ticker={p.instrumentId}
                name={p.assetClassLabel}
              />
            ),
          },
          { key: 'qty', header: 'Size', sortValue: (p) => Math.abs(p.qty), cell: (p) => Math.abs(p.qty) },
          { key: 'entry', header: 'Entry', sortValue: (p) => p.entry, cell: (p) => p.entry },
          { key: 'mark', header: 'Mark', sortValue: (p) => p.mark ?? 0, cell: (p) => p.mark ?? DASH },
          { key: 'unreal', header: 'Unrealized', sortValue: (p) => p.unrealized, cell: (p) => <Pnl value={p.unrealized} bare /> },
          { key: 'ret', header: 'Return', sortValue: (p) => p.returnPct ?? 0, cell: (p) => (p.returnPct === null ? DASH : <Pct value={p.returnPct} />) },
        ]}
      />
    </TableScroll>
  )
}

function AutomationsBody() {
  const navigate = useNavigate()
  return (
    <EmptyState
      message="Automations panel"
      detail="A compact view of what is running. Manage them on the Automations screen."
      action={
        <Button size="sm" variant="primary" onClick={() => navigate('/automations')}>
          Open Automations
        </Button>
      }
    />
  )
}
