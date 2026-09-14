import { useState } from 'react'
import { Bell, Check, Download, Plus, Trash2 } from 'lucide-react'
import { useThemeStore } from '@/store/theme'
import { ASSET_CLASSES, assetClassChartColor } from '@/lib/assetClass'
import { FAMILIES } from '@/nodes'
import { money, pct, signedMoney, signedPct, winRate } from '@/lib/format'
import { Panel, PanelBody, PanelFooter, PanelHeader } from '@/components/primitives/Panel'
import { Button, IconButton } from '@/components/primitives/Button'
import { Badge, Chip, KeyValue, StatusDot, Swatch } from '@/components/primitives/Badge'
import { Label, Money, Num, Pct, Pnl, StatTile } from '@/components/primitives/Num'
import { Field, Input, NumberField, SearchInput, SwitchRow } from '@/components/primitives/Field'
import { PercentRow, Segmented, Tabs } from '@/components/primitives/Segmented'
import { Table, IdentityCell } from '@/components/primitives/Table'
import { MenuItem, Popover, Select, Tooltip } from '@/components/primitives/Overlay'
import { ConfirmDialog, Modal } from '@/components/primitives/Modal'
import { DisconnectedBanner, EmptyState, ErrorState, NoResults, PanelLoading, Skeleton, StaleBadge } from '@/components/primitives/States'
import { AllocationBar, AreaChart, BarChart, Legend, Meter, MiniBar, Sparkline } from '@/components/charts/Primitives'
import { ThemeToggle } from '@/components/ThemeToggle'
import { toast } from '@/hooks/useToast'

/* =============================================================================
   The design-system preview.

   Spec §8.7 requires every component's states to be reachable outside the app.
   This route is that surface: every primitive, every state, both themes, both
   densities, on one page. It is public on purpose — it must be openable without
   a session so the system can be reviewed independently of the product.
   ============================================================================= */

const SAMPLE = Array.from({ length: 40 }, (_, i) => ({
  t: Math.floor(Date.now() / 1000) - (40 - i) * 86400,
  v: 880_000 + Math.sin(i / 3) * 9000 + i * 900,
}))

function Section({ title, blurb, children }: { title: string; blurb?: string; children: React.ReactNode }) {
  return (
    <Panel style={{ marginBottom: 'var(--s-3)' }}>
      <PanelHeader title={title} actions={blurb ? <Label>{blurb}</Label> : undefined} />
      <PanelBody>{children}</PanelBody>
    </Panel>
  )
}

function Row({ children }: { children: React.ReactNode }) {
  return (
    <div className="row" style={{ flexWrap: 'wrap', gap: 'var(--s-3)', marginBottom: 'var(--s-3)' }}>
      {children}
    </div>
  )
}

export function DesignSystemPage() {
  const { theme, density, setDensity } = useThemeStore()
  const [tab, setTab] = useState('a')
  const [seg, setSeg] = useState('1h')
  const [side, setSide] = useState('buy')
  const [num, setNum] = useState('64250.00')
  const [text, setText] = useState('')
  const [sel, setSel] = useState<string | null>('ema')
  const [sw, setSw] = useState(true)
  const [pctSel, setPctSel] = useState<number | null>(50)
  const [modal, setModal] = useState(false)
  const [confirm, setConfirm] = useState(false)

  return (
    <div style={{ height: '100vh', overflow: 'auto', background: 'var(--bg-canvas)' }}>
      <header className="topbar">
        <div className="brand">
          <div className="brand-mark" aria-hidden>
            <svg width="14" height="14" viewBox="0 0 16 16" fill="none">
              <path d="M2 11.5 6 6l3 3.2L14 3.5" stroke="currentColor" strokeWidth="1.9" strokeLinecap="round" strokeLinejoin="round" />
            </svg>
          </div>
          <span className="brand-name">Meridian</span>
        </div>
        <span className="panel-title">Design system</span>
        <div className="spacer" />
        <Segmented
          ariaLabel="Density"
          value={density}
          onChange={setDensity}
          options={[
            { value: 'comfortable', label: 'Comfortable' },
            { value: 'compact', label: 'Compact' },
          ]}
        />
        <ThemeToggle />
      </header>

      <div style={{ padding: 'var(--s-5)', maxWidth: 1280, margin: '0 auto' }}>
        <div className="pagehead" style={{ padding: 0 }}>
          <h1 className="h1">Meridian</h1>
          <Chip>{theme}</Chip>
          <Chip>{density}</Chip>
          <div className="spacer" />
          <Label>Every primitive, every state, both themes</Label>
        </div>

        <Section title="Typography" blurb="One scale, explicit roles">
          <div style={{ display: 'grid', gap: 'var(--s-3)' }}>
            <div className="display" style={{ fontSize: 'var(--t-32)', fontWeight: 600 }}>Hero figure — display face</div>
            <h1 className="h1">Page title (h1)</h1>
            <div className="h2">Section title (h2)</div>
            <div style={{ fontSize: 'var(--t-13)' }}>Body at 13px. The number is the product; chrome recedes and figures dominate.</div>
            <div className="lbl">Micro-label — all four --label-* tokens</div>
            <div className="num" style={{ fontSize: 'var(--t-20)', fontWeight: 600 }}>1,234,567.89 · tabular numerals</div>
            <div className="mono" style={{ fontSize: 'var(--t-12)' }}>mono / IDs / timestamps · ND_4F21</div>
          </div>
        </Section>

        <Section title="Numbers and direction" blurb="Colour is never the only cue">
          <Row>
            <StatTile label="Account equity" value={<Money value={910432.18} />} meta="all accounts" />
            <StatTile label="Day P&L" value={<Pnl value={8214.06} />} meta={signedPct(0.91)} />
            <StatTile label="Unrealized" value={<Pnl value={-4222.5} />} meta="4 open positions" />
            <StatTile label="Win rate" value={<Num>{winRate(0.542)}</Num>} meta="982 trades" />
            <StatTile label="Zero is neutral" value={<Pnl value={0} />} meta="no sign, no colour" />
          </Row>
          <Row>
            <Pct value={2.19} />
            <Pct value={-0.86} />
            <Pct value={1.91} signed={false} arrow />
            <span className="num stale">{money(64318.4)}</span>
            <StaleBadge age="12s" />
          </Row>
        </Section>

        <Section title="Buttons" blurb="6 variants × 5 sizes × 8 states">
          <Row>
            <Button variant="primary">Primary</Button>
            <Button>Secondary</Button>
            <Button variant="ghost">Ghost</Button>
            <Button variant="danger">Danger</Button>
            <Button variant="buy">Buy</Button>
            <Button variant="sell">Sell</Button>
          </Row>
          <Row>
            <Button size="xs">Extra small</Button>
            <Button size="sm" icon={<Plus size={13} />}>Small with icon</Button>
            <Button>Medium</Button>
            <Button size="lg">Large</Button>
            <Button size="xl" variant="buy">Buy 1.250 BTC · $80,312.50</Button>
          </Row>
          <Row>
            <Button disabled>Disabled</Button>
            <Button loading>Loading keeps its label</Button>
            <Button active>Selected</Button>
            <IconButton label="Download"><Download size={14} /></IconButton>
            <IconButton label="Delete" active><Trash2 size={14} /></IconButton>
            <IconButton label="Bare" bare><Bell size={14} /></IconButton>
          </Row>
        </Section>

        <Section title="Badges, chips and status" blurb="A badge always contains a word">
          <Row>
            <Badge tone="pos">Filled</Badge>
            <Badge tone="neg">Rejected</Badge>
            <Badge tone="warn">Paused</Badge>
            <Badge tone="info">Queued</Badge>
            <Badge tone="accent">Running</Badge>
            <Badge tone="neutral">Manual</Badge>
            <Badge tone="outline">Outline</Badge>
            <Chip>Crypto</Chip>
            <span className="row" style={{ gap: 6 }}><StatusDot state="live" /><Label>12 ms</Label></span>
            <span className="row" style={{ gap: 6 }}><StatusDot state="degraded" pulsing /><Label>Reconnecting</Label></span>
            <span className="row" style={{ gap: 6 }}><StatusDot state="down" /><Label>Offline</Label></span>
          </Row>
        </Section>

        <Section title="Controls" blurb="Every control has a visible focus ring">
          <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fit,minmax(240px,1fr))', gap: 'var(--s-4)' }}>
            <Input label="Text input" placeholder="you@example.com" value={text} onChange={(e) => setText(e.target.value)} />
            <Input label="With error" value="nope" error="That email and password do not match an account." onChange={() => {}} />
            <NumberField label="Number field" unit="USD" value={num} onValueChange={setNum} dp={2} step={0.01} />
            <Field label="Select">
              <Select
                ariaLabel="Indicator"
                value={sel}
                onChange={setSel}
                options={[
                  { value: 'ema', label: 'EMA — Exponential Moving Average', text: 'EMA' },
                  { value: 'sma', label: 'SMA — Simple Moving Average', text: 'SMA' },
                  { value: 'rsi', label: 'RSI — Relative Strength Index', text: 'RSI' },
                ]}
              />
            </Field>
            <Field label="Search">
              <SearchInput value={text} onValueChange={setText} />
            </Field>
            <Field label="Disabled">
              <Input value="Locked" disabled onChange={() => {}} />
            </Field>
          </div>
          <div style={{ height: 'var(--s-4)' }} />
          <Row>
            <Segmented
              ariaLabel="Timeframe"
              value={seg}
              onChange={setSeg}
              options={['1m', '5m', '15m', '1h', '4h', '1D'].map((v) => ({ value: v, label: v }))}
            />
            <Segmented
              ariaLabel="Side"
              variant="buysell"
              value={side}
              onChange={setSide}
              options={[
                { value: 'buy', label: 'Buy / Long', side: 'buy' },
                { value: 'sell', label: 'Sell / Short', side: 'sell' },
              ]}
            />
          </Row>
          <div style={{ maxWidth: 340 }}>
            <PercentRow value={pctSel} onChange={setPctSel} />
            <div style={{ height: 'var(--s-3)' }} />
            <SwitchRow label="Tick flash" description="Flash a cell when its price changes." checked={sw} onCheckedChange={setSw} />
          </div>
        </Section>

        <Section title="Tabs">
          <Tabs
            ariaLabel="Demo"
            value={tab}
            onChange={setTab}
            tabs={[
              { value: 'a', label: 'By asset class' },
              { value: 'b', label: 'Open positions', count: 4 },
              { value: 'c', label: 'Recent fills' },
              { value: 'd', label: 'Orders' },
            ]}
          />
        </Section>

        <Section title="Table" blurb="Identity left, every other column right">
          <Table
            caption="Sample portfolio"
            rows={ASSET_CLASSES.slice(0, 6).map((a, i) => ({
              ...a,
              equity: 212480 - i * 24000,
              pnl: i % 3 === 2 ? -602.1 : 3102.44 - i * 400,
              win: 0.58 - i * 0.03,
              alloc: 23.34 - i * 3,
            }))}
            rowKey={(r) => r.id}
            columns={[
              {
                key: 'c',
                header: 'Asset class',
                align: 'left',
                numeric: false,
                sortValue: (r) => r.label,
                cell: (r) => <IdentityCell swatch={<Swatch color={assetClassChartColor(r.id)} />} ticker={r.label} name={r.description} />,
              },
              { key: 'e', header: 'Equity', sortValue: (r) => r.equity, cell: (r) => money(r.equity) },
              { key: 'p', header: 'Day P&L', sortValue: (r) => r.pnl, cell: (r) => <Pnl value={r.pnl} bare /> },
              { key: 'w', header: 'Win rate', sortValue: (r) => r.win, cell: (r) => winRate(r.win) },
              {
                key: 'a',
                header: 'Allocation',
                sortValue: (r) => r.alloc,
                cell: (r) => (
                  <span className="row" style={{ justifyContent: 'flex-end', gap: 'var(--s-2)' }}>
                    <MiniBar pct={r.alloc} max={24} color={assetClassChartColor(r.id)} />
                    <span className="num" style={{ width: 52, textAlign: 'right' }}>{pct(r.alloc)}</span>
                  </span>
                ),
              },
            ]}
            footer={
              <tr>
                <td className="l">Total</td>
                <td className="n">{money(910432.18)}</td>
                <td className="n pos">{signedMoney(8214.06)}</td>
                <td className="n">{winRate(0.542)}</td>
                <td className="n">{pct(100)}</td>
              </tr>
            }
          />
        </Section>

        <Section title="Charts" blurb="Every colour is a --chart-* token">
          <div style={{ display: 'grid', gridTemplateColumns: 'minmax(0,2fr) minmax(0,1fr)', gap: 'var(--s-4)' }}>
            <div>
              <AreaChart points={SAMPLE} height={200} />
            </div>
            <div>
              <AllocationBar
                segments={ASSET_CLASSES.slice(0, 8).map((a, i) => ({
                  key: a.id,
                  label: a.label,
                  pct: 24 - i * 2.4,
                  value: 212480 - i * 20000,
                  color: assetClassChartColor(a.id),
                }))}
              />
              <div style={{ height: 'var(--s-3)' }} />
              <Legend
                items={ASSET_CLASSES.slice(0, 5).map((a, i) => ({
                  key: a.id,
                  label: a.label,
                  color: assetClassChartColor(a.id),
                  value: pct(24 - i * 2.4),
                  meta: money(212480 - i * 20000, 'USD', 0),
                }))}
              />
              <div style={{ height: 'var(--s-4)' }} />
              <Label>Margin utilisation</Label>
              <Meter value={18.4} max={60} tone="pos" ariaLabel="Margin" />
              <div style={{ height: 'var(--s-4)' }} />
              <Sparkline points={SAMPLE.map((p) => p.v)} width={240} height={40} />
            </div>
          </div>
          <div style={{ height: 'var(--s-4)' }} />
          <Label>Monthly returns — diverging ramp, never categorical</Label>
          <BarChart
            bars={['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep'].map((m, i) => ({
              key: m,
              label: m,
              value: [3.2, -1.4, 5.1, 2.0, -3.8, 1.2, 4.4, -0.6, 6.0][i],
            }))}
          />
        </Section>

        <Section title="Strategy node families" blurb="Colour encodes what kind of block it is">
          <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fit,minmax(200px,1fr))', gap: 'var(--s-3)' }}>
            {Object.values(FAMILIES).map((f) => (
              <div key={f.family} className="node" style={{ ['--fam' as string]: f.token, width: '100%' }}>
                <div className="hd">
                  <span className="ico"><f.icon size={12} aria-hidden /></span>
                  <span className="fam">{f.label}</span>
                </div>
                <div className="ttl">Example block</div>
                <div className="fields">
                  <div className="nf"><span className="k">Stage</span><span className="v">{f.stage}</span></div>
                </div>
              </div>
            ))}
          </div>
        </Section>

        <Section title="Overlays">
          <Row>
            <Tooltip content="A tooltip never contains interactive content.">
              <Button>Hover me</Button>
            </Tooltip>
            <Popover
              ariaLabel="Demo menu"
              trigger={({ onClick, ref, 'aria-expanded': e }) => (
                <button ref={ref} type="button" className="btn" aria-expanded={e} onClick={onClick}>
                  Open menu
                </button>
              )}
            >
              {(close) => (
                <>
                  <MenuItem icon={<Check size={13} />} onClick={close}>An item</MenuItem>
                  <MenuItem selected onClick={close}>A selected item</MenuItem>
                  <MenuItem danger onClick={close}>A destructive item</MenuItem>
                </>
              )}
            </Popover>
            <Button onClick={() => setModal(true)}>Open modal</Button>
            <Button variant="danger" onClick={() => setConfirm(true)}>Destructive confirm</Button>
            <Button onClick={() => toast({ title: 'Order submitted', description: 'Buy 1.250 BTC · $80,312.50', variant: 'success' })}>
              Fire a toast
            </Button>
          </Row>
        </Section>

        <Section title="States" blurb="Empty · filtered · loading · error · stale · disconnected">
          <DisconnectedBanner state="degraded" />
          <div style={{ height: 'var(--s-3)' }} />
          <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fit,minmax(260px,1fr))', gap: 'var(--s-3)' }}>
            <Panel><PanelBody><EmptyState message="Nothing is running" detail="An automation watches a market and acts without you." action={<Button size="sm" variant="primary">Create one</Button>} /></PanelBody></Panel>
            <Panel><PanelBody><NoResults query="btcc" onClear={() => {}} /></PanelBody></Panel>
            <Panel><PanelBody><ErrorState message="This panel could not load." onRetry={() => {}} /></PanelBody></Panel>
            <Panel><PanelBody><PanelLoading lines={5} /></PanelBody></Panel>
            <Panel>
              <PanelHeader title="Skeleton rows" />
              <PanelBody flush>
                {[0, 1, 2].map((i) => (
                  <div key={i} className="skel-row">
                    <Skeleton width="34%" /><Skeleton width="18%" /><Skeleton width="22%" />
                  </div>
                ))}
              </PanelBody>
            </Panel>
            <Panel>
              <PanelHeader title="Callouts" />
              <PanelBody>
                <div className="callout warn" style={{ marginBottom: 8 }}><span>Paper account — no live funds at risk.</span></div>
                <div className="callout neg" style={{ marginBottom: 8 }}><span>Exceeds buying power by $12,480.</span></div>
                <div className="callout info"><span>Alerts are held on this device until the service is running.</span></div>
              </PanelBody>
              <PanelFooter>
                <KeyValue k="Order value" v={money(80312.5)} />
              </PanelFooter>
            </Panel>
          </div>
        </Section>

        <div style={{ height: 'var(--s-10)' }} />
      </div>

      <Modal
        open={modal}
        onClose={() => setModal(false)}
        title="A modal"
        description="Focus trapped, Esc closes, primary action last."
        footer={
          <>
            <Button onClick={() => setModal(false)}>Cancel</Button>
            <Button variant="primary" onClick={() => setModal(false)}>Confirm</Button>
          </>
        }
      >
        <p style={{ fontSize: 'var(--t-13)', lineHeight: 1.55 }} className="sec">
          Modals carry the largest radius and the deepest elevation in the system. In <code>terminal</code> the
          radius tightens and the shadow becomes a glow.
        </p>
      </Modal>

      <ConfirmDialog
        open={confirm}
        onCancel={() => setConfirm(false)}
        onConfirm={() => setConfirm(false)}
        title="Close all positions?"
        confirmLabel="Close 4 positions"
        typeToConfirm="CLOSE"
        consequence={<>This closes <strong>4 positions</strong> with a combined notional of <strong>$412,880.00</strong> at market. It cannot be undone.</>}
        alternative={<Button size="sm">Pause the automations instead</Button>}
      />
    </div>
  )
}
