import { useEffect, useMemo, useRef, useState } from 'react'
import { useMutation, useQuery } from '@tanstack/react-query'
import { Accessibility, AlertTriangle, Bell, Bot, CandlestickChart, Check, Copy, Database, Download, FlaskConical, Gauge, Globe, Info, Keyboard, KeyRound, LayoutGrid, LogOut, Palette, Plug, Receipt, RotateCcw, Shield, ShieldAlert, Sigma, Upload, User as UserIcon, Zap } from 'lucide-react'
import { api } from '@/lib/api'
import { useAuthStore } from '@/store/auth'
import { useModeStore } from '@/store/mode'
import { useThemeStore } from '@/store/theme'
import { usePrefs, PREFS_STORAGE_KEY, type Prefs } from '@/store/prefs'
import { useWatchlistStore } from '@/store/watchlist'
import { useToast } from '@/hooks/useToast'
import { useConnection } from '@/hooks/useConnection'
import { THEME_DESCRIPTION, type Theme } from '@/lib/theme'
import { ASSET_CLASSES } from '@/lib/assetClass'
import { TIMEFRAMES } from '@/hooks/useBars'
import { DASH, clockTime, dateTime, money, pct, price as fmtPrice, signedMoney, signedPct } from '@/lib/format'
import { Panel, PanelBody, PanelFooter, PanelHeader } from '@/components/primitives/Panel'
import { Button } from '@/components/primitives/Button'
import { Badge, KeyValue, StatusDot } from '@/components/primitives/Badge'
import { Label } from '@/components/primitives/Num'
import { Field, Input, NumberField, SwitchRow, TextArea } from '@/components/primitives/Field'
import { Segmented } from '@/components/primitives/Segmented'
import { Select } from '@/components/primitives/Overlay'
import { ConfirmDialog, Modal } from '@/components/primitives/Modal'
import { EmptyState } from '@/components/primitives/States'
import { VenueCredentials } from '@/components/settings/VenueCredentials'
import { LlmCredentialForm } from '@/components/agent/LlmCredentialForm'
import { ApiAccessCard } from '@/components/agent/ApiAccessCard'
import { cn } from '@/lib/utils'

/* =============================================================================
   Spec §4.4 — Settings.

   "A two-column layout: 240px section nav + a --content-max reading column.
    This is the one place paper's reading typography leads."

   Sixteen sections, grouped the way a trader actually thinks about the product:
   how it looks, how it trades, what it risks, what it shows, what it connects
   to, and what it can destroy.
   ============================================================================= */

const SECTIONS = [
  { id: 'appearance', label: 'Appearance', icon: Palette, group: 'Interface' },
  { id: 'workspace', label: 'Workspace', icon: LayoutGrid, group: 'Interface' },
  { id: 'charts', label: 'Charts', icon: CandlestickChart, group: 'Interface' },
  { id: 'accessibility', label: 'Accessibility', icon: Accessibility, group: 'Interface' },

  { id: 'ticket', label: 'Order ticket', icon: Receipt, group: 'Trading' },
  { id: 'risk', label: 'Risk limits', icon: Shield, group: 'Trading' },
  { id: 'automations', label: 'Automation defaults', icon: Zap, group: 'Trading' },
  { id: 'backtesting', label: 'Back testing', icon: FlaskConical, group: 'Trading' },

  { id: 'marketdata', label: 'Market data', icon: Gauge, group: 'Data' },
  { id: 'numbers', label: 'Numbers & locale', icon: Sigma, group: 'Data' },
  { id: 'storage', label: 'Local data', icon: Database, group: 'Data' },

  { id: 'venues', label: 'Venues', icon: Plug, group: 'Connections' },
  { id: 'ai', label: 'AI providers', icon: KeyRound, group: 'Connections' },
  { id: 'agent', label: 'Research agent', icon: Bot, group: 'Connections' },
  { id: 'api', label: 'API access', icon: Globe, group: 'Connections' },

  { id: 'notifications', label: 'Notifications', icon: Bell, group: 'Account' },
  { id: 'keyboard', label: 'Keyboard', icon: Keyboard, group: 'Account' },
  { id: 'account', label: 'Account', icon: UserIcon, group: 'Account' },
  { id: 'about', label: 'About', icon: Info, group: 'Account' },
  { id: 'danger', label: 'Danger zone', icon: ShieldAlert, group: 'Account' },
] as const

type SectionId = (typeof SECTIONS)[number]['id']

export function SettingsPage() {
  const [active, setActive] = useState<SectionId>('appearance')
  const refs = useRef<Partial<Record<SectionId, HTMLElement | null>>>({})
  const scrollRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    const root = scrollRef.current
    if (!root) return
    const io = new IntersectionObserver(
      (entries) => {
        const visible = entries
          .filter((e) => e.isIntersecting)
          .sort((a, b) => a.boundingClientRect.top - b.boundingClientRect.top)
        const id = visible[0]?.target.getAttribute('data-section') as SectionId | undefined
        if (id) setActive(id)
      },
      { root, rootMargin: '-8% 0px -74% 0px', threshold: 0 },
    )
    Object.values(refs.current).forEach((el) => el && io.observe(el))
    return () => io.disconnect()
  }, [])

  function go(id: SectionId) {
    setActive(id)
    refs.current[id]?.scrollIntoView({ behavior: 'smooth', block: 'start' })
  }

  const groups = useMemo(() => {
    const out: { group: string; items: typeof SECTIONS[number][] }[] = []
    for (const s of SECTIONS) {
      const g = out.find((x) => x.group === s.group)
      if (g) g.items.push(s)
      else out.push({ group: s.group, items: [s] })
    }
    return out
  }, [])

  return (
    <>
      <div className="pagehead">
        <h1 className="h1">Settings</h1>
        <div className="spacer" />
        <Label>Stored on this device unless a section says otherwise</Label>
      </div>

      <div className="read-body" ref={scrollRef}>
        <nav className="read-nav" aria-label="Settings sections">
          {groups.map((g) => (
            <div key={g.group}>
              <div className="lbl" style={{ padding: 'var(--s-3) var(--s-3) 4px' }}>
                {g.group}
              </div>
              {g.items.map((s) => (
                <button key={s.id} type="button" className={cn(active === s.id && 'on')} onClick={() => go(s.id)}>
                  <s.icon size={13} aria-hidden />
                  {s.label}
                </button>
              ))}
            </div>
          ))}
        </nav>

        <div className="read-col">
          <S id="appearance" title="Appearance" refs={refs} blurb="Two themes, deliberately different. Not one theme and its inversion."><AppearanceSection /></S>
          <S id="workspace" title="Workspace" refs={refs} blurb="Where the app opens and how the trading desk behaves."><WorkspaceSection /></S>
          <S id="charts" title="Charts" refs={refs} blurb="What a chart shows before you touch it."><ChartsSection /></S>
          <S id="accessibility" title="Accessibility" refs={refs} blurb="Colour is never the only cue. These make that stronger."><AccessibilitySection /></S>

          <S id="ticket" title="Order ticket" refs={refs} blurb="What the ticket is pre-filled with when it opens."><TicketSection /></S>
          <S id="risk" title="Risk limits" refs={refs} blurb="Account-level clamps that apply to every strategy and every order."><RiskSection /></S>
          <S id="automations" title="Automation defaults" refs={refs} blurb="Starting point for a new automation."><AutomationDefaultsSection /></S>
          <S id="backtesting" title="Back testing" refs={refs} blurb="Starting point for a new run."><BacktestDefaultsSection /></S>

          <S id="marketdata" title="Market data" refs={refs} blurb="Feed depth, repaint ceiling and what counts as stale."><MarketDataSection /></S>
          <S id="numbers" title="Numbers & locale" refs={refs} blurb="How every figure in the product renders."><NumbersSection /></S>
          <S id="storage" title="Local data" refs={refs} blurb="What this browser is holding, and how to clear it."><StorageSection /></S>

          <S id="venues" title="Venues" refs={refs} blurb="Credentials are verified at the venue before they are saved."><VenuesSection /></S>
          <S id="ai" title="AI providers" refs={refs} blurb="Encrypted at rest, verified on save, never returned."><AiSection /></S>
          <S id="agent" title="Research agent" refs={refs} blurb="What the agent may do without asking, and what it costs."><AgentSection /></S>
          <S id="api" title="API access" refs={refs} blurb="Service tokens for headless and MCP access."><ApiAccessCard /></S>

          <S id="notifications" title="Notifications" refs={refs} blurb="What interrupts you."><NotificationsSection /></S>
          <S id="keyboard" title="Keyboard" refs={refs} blurb="Every shortcut in the product."><KeyboardSection /></S>
          <S id="account" title="Account" refs={refs}><AccountSection /></S>
          <S id="about" title="About" refs={refs}><AboutSection /></S>
          <S id="danger" title="Danger zone" refs={refs} blurb="Everything here is irreversible."><DangerSection /></S>

          <div style={{ height: 'var(--s-16)' }} />
        </div>
      </div>
    </>
  )
}

function S({
  id,
  title,
  blurb,
  refs,
  children,
}: {
  id: SectionId
  title: string
  blurb?: string
  refs: React.MutableRefObject<Partial<Record<SectionId, HTMLElement | null>>>
  children: React.ReactNode
}) {
  return (
    <section
      data-section={id}
      ref={(el) => {
        refs.current[id] = el
      }}
      style={{ scrollMarginTop: 'var(--s-4)' }}
    >
      <div style={{ margin: 'var(--s-6) 0 var(--s-3)' }}>
        <div className="h2">{title}</div>
        {blurb && (
          <div className="mut" style={{ fontSize: 'var(--t-12)', marginTop: 3, lineHeight: 1.5 }}>
            {blurb}
          </div>
        )}
      </div>
      {children}
    </section>
  )
}

/** A small reset affordance so a group can be undone without nuking everything. */
function ResetGroup({ keys, label }: { keys: (keyof Prefs)[]; label: string }) {
  const prefs = usePrefs()
  const toast = useToast()
  return (
    <Button
      size="xs"
      variant="ghost"
      icon={<RotateCcw size={11} aria-hidden />}
      onClick={() => {
        prefs.resetGroup(keys)
        toast({ title: `${label} reset to defaults` })
      }}
    >
      Reset
    </Button>
  )
}

/* =============================================================================
   INTERFACE
   ============================================================================= */

function AppearanceSection() {
  const { theme, setTheme, density, setDensity } = useThemeStore()
  const prefs = usePrefs()

  return (
    <Panel>
      <PanelBody>
        <Field label="Theme">
          <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 'var(--s-3)' }}>
            {(['paper', 'terminal'] as Theme[]).map((t) => (
              <button
                key={t}
                type="button"
                onClick={() => setTheme(t)}
                aria-pressed={theme === t}
                style={{
                  textAlign: 'left',
                  padding: 'var(--s-3)',
                  borderRadius: 'var(--r-md)',
                  border: `1px solid ${theme === t ? 'var(--line-accent)' : 'var(--line-hairline)'}`,
                  background: theme === t ? 'var(--bg-selected)' : 'var(--bg-sunken)',
                }}
              >
                <div className="row" style={{ gap: 6 }}>
                  <span style={{ fontSize: 'var(--t-13)', fontWeight: 'var(--w-semibold)', textTransform: 'capitalize' }}>
                    {t}
                  </span>
                  {theme === t && <Check size={12} className="acc" aria-hidden />}
                </div>
                <div className="mut" style={{ fontSize: 'var(--t-11)', marginTop: 4, lineHeight: 1.45 }}>
                  {THEME_DESCRIPTION[t]}
                </div>
              </button>
            ))}
          </div>
        </Field>

        <div className="hr" />

        <Field
          label="Density"
          hint="Row heights and control sizes only. Type sizes never change, so data density never drifts with a browser font setting."
        >
          <Segmented
            ariaLabel="Density"
            wide
            value={density}
            onChange={setDensity}
            options={[
              { value: 'comfortable', label: 'Comfortable' },
              { value: 'compact', label: 'Compact' },
            ]}
          />
        </Field>

        <div className="hr" />

        <SwitchRow
          label="Tick flash"
          description="Flash a cell background when its price changes. Prices never animate their position — that is deliberate, and not configurable."
          checked={prefs.tickFlash}
          onCheckedChange={(v) => prefs.set('tickFlash', v)}
        />
      </PanelBody>
      <PanelFooter>
        <Label>
          {'⌘\\'} switches theme {'·'} {'⌘.'} switches density
        </Label>
      </PanelFooter>
    </Panel>
  )
}

function WorkspaceSection() {
  const prefs = usePrefs()
  return (
    <Panel>
      <PanelHeader
        title="Startup and desk behaviour"
        actions={<ResetGroup label="Workspace" keys={['landingPage', 'rememberLastInstrument', 'confirmPanelClose', 'scrollToNewPanel']} />}
      />
      <PanelBody>
        <Field label="Open the app on">
          <Select
            ariaLabel="Landing page"
            value={prefs.landingPage}
            onChange={(v) => prefs.set('landingPage', v as Prefs['landingPage'])}
            options={[
              { value: '/dashboard', label: 'Dashboard — where do I stand', text: 'Dashboard' },
              { value: '/trading', label: 'Trading desk — my panel layout', text: 'Trading desk' },
              { value: '/terminal', label: 'Terminal — one market, full depth', text: 'Terminal' },
              { value: '/strategy', label: 'Strategy builder', text: 'Strategy' },
              { value: '/agent', label: 'Research agent', text: 'Agent' },
            ]}
          />
        </Field>
        <div className="hr" />
        <SwitchRow
          label="Remember the last instrument"
          description={
            prefs.lastInstrument
              ? `The terminal reopens on ${prefs.lastInstrument}.`
              : 'The terminal reopens on whatever market you last had open.'
          }
          checked={prefs.rememberLastInstrument}
          onCheckedChange={(v) => prefs.set('rememberLastInstrument', v)}
        />
        <div className="hr" />
        <SwitchRow
          label="Confirm before closing a panel"
          description="Useful once a desk has a lot of configured panels on it."
          checked={prefs.confirmPanelClose}
          onCheckedChange={(v) => prefs.set('confirmPanelClose', v)}
        />
        <div className="hr" />
        <SwitchRow
          label="Scroll to a newly added panel"
          description="Keeps a wide desk pointed at whatever you just added."
          checked={prefs.scrollToNewPanel}
          onCheckedChange={(v) => prefs.set('scrollToNewPanel', v)}
        />
      </PanelBody>
      <PanelFooter>
        <Label>Panel layouts, widths and chart settings are saved per trading environment, automatically.</Label>
      </PanelFooter>
    </Panel>
  )
}

function ChartsSection() {
  const prefs = usePrefs()
  return (
    <Panel>
      <PanelHeader
        title="Chart defaults"
        actions={
          <ResetGroup
            label="Chart defaults"
            keys={['defaultTimeframeSecs', 'chartStyle', 'showVolume', 'showLastPriceLine', 'showPositionLines', 'showOrderLines', 'logScale', 'extendedHours', 'chartLookbackDays']}
          />
        }
      />
      <PanelBody>
        <Field label="Default timeframe">
          <Segmented
            ariaLabel="Default timeframe"
            wide
            value={String(prefs.defaultTimeframeSecs)}
            onChange={(v) => prefs.set('defaultTimeframeSecs', Number(v))}
            options={TIMEFRAMES.map((t) => ({ value: String(t.secs), label: t.label }))}
          />
        </Field>
        <div style={{ height: 'var(--s-3)' }} />
        <Field label="Series style">
          <Segmented
            ariaLabel="Chart style"
            wide
            value={prefs.chartStyle}
            onChange={(v) => prefs.set('chartStyle', v as Prefs['chartStyle'])}
            options={[
              { value: 'candles', label: 'Candles' },
              { value: 'bars', label: 'Bars' },
              { value: 'line', label: 'Line' },
              { value: 'area', label: 'Area' },
            ]}
          />
        </Field>
        <div style={{ height: 'var(--s-3)' }} />
        <NumberField
          label="History to load"
          unit="days"
          dp={0}
          min={1}
          max={3650}
          step={30}
          value={String(prefs.chartLookbackDays)}
          onValueChange={(v) => prefs.set('chartLookbackDays', Math.max(1, parseInt(v) || 1))}
          hint="More history costs a slower first paint. The live tail streams regardless."
        />
        <div className="hr" />
        <SwitchRow label="Volume pane" description="Volume bars are neutral, never up/down coloured — colouring them doubles the noise for no information." checked={prefs.showVolume} onCheckedChange={(v) => prefs.set('showVolume', v)} />
        <div className="hr" />
        <SwitchRow label="Last-price line" description="A dashed line with a price tag at the axis." checked={prefs.showLastPriceLine} onCheckedChange={(v) => prefs.set('showLastPriceLine', v)} />
        <div className="hr" />
        <SwitchRow label="Position lines" description="Entry, stop and target drawn on the chart, each labelled with its price." checked={prefs.showPositionLines} onCheckedChange={(v) => prefs.set('showPositionLines', v)} />
        <div className="hr" />
        <SwitchRow label="Working-order lines" description="Resting limit and stop orders drawn at their price." checked={prefs.showOrderLines} onCheckedChange={(v) => prefs.set('showOrderLines', v)} />
        <div className="hr" />
        <SwitchRow label="Logarithmic price scale" description="Better for long histories with large percentage moves." checked={prefs.logScale} onCheckedChange={(v) => prefs.set('logScale', v)} />
        <div className="hr" />
        <SwitchRow label="Extended hours" description="Include pre- and post-market bars for equities and ETFs." checked={prefs.extendedHours} onCheckedChange={(v) => prefs.set('extendedHours', v)} />
      </PanelBody>
    </Panel>
  )
}

function AccessibilitySection() {
  const prefs = usePrefs()
  return (
    <Panel>
      <PanelBody>
        <SwitchRow
          label="Always show direction glyphs"
          description="Adds ▲ / ▼ beside every directional figure, not only unsigned ones. Around 8% of men cannot rely on red and green."
          checked={prefs.alwaysShowDirectionGlyph}
          onCheckedChange={(v) => prefs.set('alwaysShowDirectionGlyph', v)}
        />
        <div className="hr" />
        <SwitchRow
          label="Underline links"
          description="Underlines every link, not only on hover."
          checked={prefs.underlineLinks}
          onCheckedChange={(v) => prefs.set('underlineLinks', v)}
        />
        <div className="hr" />
        <SwitchRow
          label="Thicker focus ring"
          description="Widens the focus indicator from 2px to 3px with a larger offset."
          checked={prefs.thickFocusRing}
          onCheckedChange={(v) => prefs.set('thickFocusRing', v)}
        />
        <div className="hr" />
        <SwitchRow
          label="Reduce motion"
          description="Disables transitions and the tick flash even when the operating system does not ask for it."
          checked={prefs.reduceMotion}
          onCheckedChange={(v) => prefs.set('reduceMotion', v)}
        />
        <div className="callout info" style={{ marginTop: 'var(--s-4)' }}>
          <Accessibility size={13} aria-hidden style={{ flex: 'none', marginTop: 2 }} />
          <span>
            Every foreground/background pair in both themes is verified against WCAG AA. Direction and state
            always carry a sign, glyph or word in addition to colour — that part is not optional and has no
            switch.
          </span>
        </div>
      </PanelBody>
    </Panel>
  )
}

/* =============================================================================
   TRADING
   ============================================================================= */

function TicketSection() {
  const prefs = usePrefs()
  return (
    <Panel>
      <PanelHeader
        title="Ticket defaults"
        actions={
          <ResetGroup
            label="Ticket defaults"
            keys={['defaultOrderType', 'defaultTif', 'defaultSizeUnit', 'defaultSizePct', 'defaultLeverage', 'attachBracketByDefault', 'defaultStopPct', 'defaultTakeProfitPct', 'reduceOnlyByDefault', 'confirmEveryOrder', 'clearTicketAfterSubmit']}
          />
        }
      />
      <PanelBody>
        <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 'var(--s-3)' }}>
          <Field label="Order type">
            <Segmented
              ariaLabel="Default order type"
              wide
              value={prefs.defaultOrderType}
              onChange={(v) => prefs.set('defaultOrderType', v as Prefs['defaultOrderType'])}
              options={[
                { value: 'limit', label: 'Limit' },
                { value: 'market', label: 'Market' },
              ]}
            />
          </Field>
          <Field label="Time in force">
            <Select
              ariaLabel="Default time in force"
              value={prefs.defaultTif}
              onChange={(v) => prefs.set('defaultTif', v as Prefs['defaultTif'])}
              options={[
                { value: 'gtc', label: 'GTC — good till cancelled', text: 'GTC' },
                { value: 'day', label: 'DAY — expires at session close', text: 'DAY' },
                { value: 'ioc', label: 'IOC — immediate or cancel', text: 'IOC' },
                { value: 'fok', label: 'FOK — fill or kill', text: 'FOK' },
              ]}
            />
          </Field>
          <Field label="Size unit">
            <Segmented
              ariaLabel="Default size unit"
              wide
              value={prefs.defaultSizeUnit}
              onChange={(v) => prefs.set('defaultSizeUnit', v as Prefs['defaultSizeUnit'])}
              options={[
                { value: 'base', label: 'Base (BTC)' },
                { value: 'quote', label: 'Quote (USD)' },
              ]}
            />
          </Field>
          <NumberField
            label="Default size"
            unit="% of buying power"
            dp={0}
            min={1}
            max={100}
            step={5}
            value={String(prefs.defaultSizePct)}
            onValueChange={(v) => prefs.set('defaultSizePct', Math.max(1, Math.min(100, parseInt(v) || 1)))}
          />
          <NumberField
            label="Default leverage"
            unit="×"
            dp={0}
            min={1}
            max={50}
            value={String(prefs.defaultLeverage)}
            onValueChange={(v) => prefs.set('defaultLeverage', Math.max(1, parseInt(v) || 1))}
            hint="Only applies to markets that support margin."
          />
        </div>

        <div className="hr" />

        <SwitchRow
          label="Attach a bracket by default"
          description="Opens the ticket in Bracket mode with a stop and target pre-filled."
          checked={prefs.attachBracketByDefault}
          onCheckedChange={(v) => prefs.set('attachBracketByDefault', v)}
        />
        {prefs.attachBracketByDefault && (
          <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 'var(--s-3)', marginTop: 'var(--s-3)' }}>
            <NumberField
              label="Default stop"
              unit="% from entry"
              step={0.1}
              min={0.05}
              value={String(prefs.defaultStopPct)}
              onValueChange={(v) => prefs.set('defaultStopPct', Number(v) || 0)}
            />
            <NumberField
              label="Default take profit"
              unit="% from entry"
              step={0.1}
              min={0.05}
              value={String(prefs.defaultTakeProfitPct)}
              onValueChange={(v) => prefs.set('defaultTakeProfitPct', Number(v) || 0)}
              hint={`Implied R : R ${(prefs.defaultTakeProfitPct / Math.max(0.01, prefs.defaultStopPct)).toFixed(1)}`}
            />
          </div>
        )}

        <div className="hr" />
        <SwitchRow
          label="Reduce-only by default"
          description="New orders can only shrink an existing position. A safety habit on leveraged markets."
          checked={prefs.reduceOnlyByDefault}
          onCheckedChange={(v) => prefs.set('reduceOnlyByDefault', v)}
        />
        <div className="hr" />
        <SwitchRow
          label="Confirm every order"
          description="Live orders always confirm — that is not configurable. Turn this on to confirm paper orders too."
          checked={prefs.confirmEveryOrder}
          onCheckedChange={(v) => prefs.set('confirmEveryOrder', v)}
        />
        <div className="hr" />
        <SwitchRow
          label="Clear the ticket after submitting"
          description="Off keeps the size and price so you can work an order in slices."
          checked={prefs.clearTicketAfterSubmit}
          onCheckedChange={(v) => prefs.set('clearTicketAfterSubmit', v)}
        />
      </PanelBody>
    </Panel>
  )
}

function RiskSection() {
  const prefs = usePrefs()
  const { mode } = useModeStore()
  const toast = useToast()
  const [killOpen, setKillOpen] = useState(false)

  const status = useQuery({
    queryKey: ['trading-status'],
    queryFn: () =>
      api
        .get<{ trading_enabled: boolean; kill_switch_active: boolean }>('/api/trading/status')
        .then((r) => r.data),
    refetchInterval: 15_000,
    retry: false,
  })
  const halted = status.data?.kill_switch_active === true

  const kill = useMutation({
    mutationFn: () => api.post('/api/trading/kill'),
    onSuccess: () => {
      toast({ title: 'Kill switch tripped', description: 'No further orders will be accepted until it is reset.', variant: 'warning' })
      void status.refetch()
    },
    onError: () => toast({ title: 'Could not trip the kill switch', variant: 'error' }),
  })
  const resume = useMutation({
    mutationFn: () => api.post('/api/trading/resume'),
    onSuccess: () => {
      toast({ title: 'Trading resumed' })
      void status.refetch()
    },
    onError: () => toast({ title: 'Could not resume trading', variant: 'error' }),
  })

  return (
    <>
      <Panel>
        <PanelHeader
          title="Limits"
          badges={<Badge tone="warn">Clamps every strategy and every ticket</Badge>}
          actions={
            <ResetGroup
              label="Risk limits"
              keys={['maxRiskPerTradePct', 'maxConcurrentPositions', 'dailyLossCircuitBreakerPct', 'marginLimitPct', 'maxOrderNotional', 'largeOrderWarnPctOfEquity']}
            />
          }
        />
        <PanelBody>
          <p className="sec" style={{ fontSize: 'var(--t-12)', lineHeight: 1.55, marginBottom: 'var(--s-4)' }}>
            These are account-level and apply at run time, whatever a strategy's blocks say. A strategy that
            asks for more is clamped rather than rejected — and the strategy inspector shows these exact
            numbers so the limit is never a surprise at deploy time.
          </p>
          <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 'var(--s-3)' }}>
            <NumberField label="Max risk per trade" unit="% of equity" value={String(prefs.maxRiskPerTradePct)} onValueChange={(v) => prefs.set('maxRiskPerTradePct', Number(v) || 0)} step={0.25} min={0.1} max={100} />
            <NumberField label="Max concurrent positions" value={String(prefs.maxConcurrentPositions)} onValueChange={(v) => prefs.set('maxConcurrentPositions', Math.max(1, parseInt(v) || 1))} dp={0} min={1} max={100} />
            <NumberField label="Daily loss circuit breaker" unit="% of equity" value={String(prefs.dailyLossCircuitBreakerPct)} onValueChange={(v) => prefs.set('dailyLossCircuitBreakerPct', Number(v) || 0)} step={0.5} min={0.5} max={100} />
            <NumberField label="Margin utilisation limit" unit="%" value={String(prefs.marginLimitPct)} onValueChange={(v) => prefs.set('marginLimitPct', Number(v) || 0)} step={5} min={5} max={100} />
            <NumberField label="Max order notional" unit="USD" dp={0} step={10000} min={0} value={String(prefs.maxOrderNotional)} onValueChange={(v) => prefs.set('maxOrderNotional', Number(v) || 0)} />
            <NumberField label="Warn above" unit="% of equity" dp={0} step={5} min={1} max={100} value={String(prefs.largeOrderWarnPctOfEquity)} onValueChange={(v) => prefs.set('largeOrderWarnPctOfEquity', Number(v) || 0)} hint="Shows an extra confirmation for unusually large orders." />
          </div>
          <div className="well" style={{ marginTop: 'var(--s-4)' }}>
            <Label>What this means in practice</Label>
            <div style={{ marginTop: 'var(--s-2)' }}>
              <KeyValue k="On a $100,000 account, one trade may risk" v={money(100000 * (prefs.maxRiskPerTradePct / 100))} />
              <KeyValue k="Trading halts for the day after" v={<span className="neg">{signedMoney(-100000 * (prefs.dailyLossCircuitBreakerPct / 100))}</span>} />
              <KeyValue k="Largest single order" v={money(prefs.maxOrderNotional)} />
            </div>
          </div>
          <div className="callout warn" style={{ marginTop: 'var(--s-4)' }}>
            <AlertTriangle size={13} aria-hidden style={{ flex: 'none', marginTop: 2 }} />
            <span>
              These limits live on this device and are shown throughout the interface. Enforcing them
              server-side needs the risk service to read them — until it does, treat them as a discipline
              aid rather than a hard gate. The kill switch below <em>is</em> server-side.
            </span>
          </div>
        </PanelBody>
      </Panel>

      <Panel>
        <PanelHeader
          title="Kill switch"
          actions={
            <span className="row" style={{ gap: 6 }}>
              <StatusDot state={halted ? 'down' : 'live'} />
              <Label>{halted ? 'Tripped — no orders accepted' : 'Trading enabled'}</Label>
              <Badge tone={mode === 'LIVE' ? 'pos' : 'warn'}>{mode}</Badge>
            </span>
          }
        />
        <PanelBody>
          <p className="sec" style={{ fontSize: 'var(--t-12)', lineHeight: 1.55 }}>
            Trips the platform-wide gate: no new orders are accepted from any source — manual tickets,
            automations or the research agent — until it is reset. Open positions are not touched, and
            become your responsibility to manage by hand.
          </p>
        </PanelBody>
        <PanelFooter>
          <Button variant="danger" onClick={() => setKillOpen(true)} loading={kill.isPending}>
            Trip the kill switch
          </Button>
          <Button onClick={() => resume.mutate()} loading={resume.isPending}>
            Resume trading
          </Button>
        </PanelFooter>
      </Panel>

      <ConfirmDialog
        open={killOpen}
        onCancel={() => setKillOpen(false)}
        onConfirm={() => {
          kill.mutate()
          setKillOpen(false)
        }}
        title="Trip the kill switch?"
        confirmLabel="Stop all order flow"
        typeToConfirm="STOP"
        consequence={
          <>
            Every order source is blocked immediately — manual tickets, automations and the agent. Existing
            positions stay open and unmanaged until you resume.
          </>
        }
      />
    </>
  )
}

function AutomationDefaultsSection() {
  const prefs = usePrefs()
  return (
    <Panel>
      <PanelHeader
        title="New automation defaults"
        actions={<ResetGroup label="Automation defaults" keys={['automationDefaultTrigger', 'automationDefaultTimeframe', 'automationArmOnCreate']} />}
      />
      <PanelBody>
        <Field label="Default trigger">
          <Segmented
            ariaLabel="Default trigger"
            wide
            value={prefs.automationDefaultTrigger}
            onChange={(v) => prefs.set('automationDefaultTrigger', v as Prefs['automationDefaultTrigger'])}
            options={[
              { value: 'ohlcv_bar', label: 'On bar close' },
              { value: 'timer', label: 'On a timer' },
            ]}
          />
        </Field>
        <div style={{ height: 'var(--s-3)' }} />
        <Field label="Default timeframe">
          <Segmented
            ariaLabel="Default automation timeframe"
            wide
            value={prefs.automationDefaultTimeframe}
            onChange={(v) => prefs.set('automationDefaultTimeframe', v)}
            options={TIMEFRAMES.map((t) => ({ value: t.label, label: t.label }))}
          />
        </Field>
        <div className="hr" />
        <SwitchRow
          label="Arm on create"
          description="Off creates the automation stopped, so you can review it before it trades."
          checked={prefs.automationArmOnCreate}
          onCheckedChange={(v) => prefs.set('automationArmOnCreate', v)}
        />
        <div className="callout neutral" style={{ marginTop: 'var(--s-4)' }}>
          <Info size={13} aria-hidden style={{ flex: 'none', marginTop: 2 }} />
          <span>
            Session windows default from the asset class: {ASSET_CLASSES.filter((a) => a.alwaysOpen).map((a) => a.short).join(', ')} trade
            continuously; everything else uses its exchange session.
          </span>
        </div>
      </PanelBody>
    </Panel>
  )
}

function BacktestDefaultsSection() {
  const prefs = usePrefs()
  return (
    <Panel>
      <PanelHeader
        title="New run defaults"
        actions={<ResetGroup label="Back testing defaults" keys={['backtestDefaultBalance', 'backtestDefaultQuote', 'backtestDefaultDays', 'backtestAutoCollect']} />}
      />
      <PanelBody>
        <div style={{ display: 'grid', gridTemplateColumns: '2fr 1fr', gap: 'var(--s-3)' }}>
          <NumberField label="Starting balance" dp={0} step={10000} min={0} value={prefs.backtestDefaultBalance} onValueChange={(v) => prefs.set('backtestDefaultBalance', v)} />
          <Input label="Quote currency" value={prefs.backtestDefaultQuote} onChange={(e) => prefs.set('backtestDefaultQuote', e.target.value.toUpperCase())} />
        </div>
        <div style={{ height: 'var(--s-3)' }} />
        <NumberField label="Default window" unit="days" dp={0} step={30} min={1} max={3650} value={String(prefs.backtestDefaultDays)} onValueChange={(v) => prefs.set('backtestDefaultDays', Math.max(1, parseInt(v) || 1))} />
        <div className="hr" />
        <SwitchRow
          label="Backfill missing bars"
          description="Collect any gaps before simulating. Only available for asset classes with an automated collector."
          checked={prefs.backtestAutoCollect}
          onCheckedChange={(v) => prefs.set('backtestAutoCollect', v)}
        />
      </PanelBody>
    </Panel>
  )
}

/* =============================================================================
   DATA
   ============================================================================= */

function MarketDataSection() {
  const prefs = usePrefs()
  const conn = useConnection()
  return (
    <Panel>
      <PanelHeader
        title="Feed"
        actions={
          <span className="row" style={{ gap: 6 }}>
            <StatusDot state={conn.state === 'connected' ? 'live' : conn.state === 'disconnected' ? 'down' : 'degraded'} />
            <Label>{conn.state}{conn.latencyMs !== null ? ` · ${conn.latencyMs} ms` : ''}</Label>
          </span>
        }
      />
      <PanelBody>
        <Field
          label="Stale quote threshold"
          hint="A streaming quote older than this renders at 60% opacity with an age badge. It is never hidden or zeroed — a stale number and a missing number are different facts."
        >
          <Segmented
            ariaLabel="Stale threshold"
            wide
            value={String(prefs.staleAfterMs)}
            onChange={(v) => prefs.set('staleAfterMs', Number(v))}
            options={[
              { value: '1000', label: '1s' },
              { value: '2000', label: '2s' },
              { value: '5000', label: '5s' },
              { value: '15000', label: '15s' },
            ]}
          />
        </Field>
        <div style={{ height: 'var(--s-3)' }} />
        <Field
          label="Repaint ceiling"
          hint="The interface never repaints faster than this, whatever the feed rate. A 200 Hz feed must not cause 200 renders."
        >
          <Segmented
            ariaLabel="Max frames per second"
            wide
            value={String(prefs.maxFps)}
            onChange={(v) => prefs.set('maxFps', Number(v))}
            options={[
              { value: '2', label: '2 fps' },
              { value: '5', label: '5 fps' },
              { value: '10', label: '10 fps' },
              { value: '30', label: '30 fps' },
            ]}
          />
        </Field>
        <div style={{ height: 'var(--s-3)' }} />
        <NumberField
          label="Order book depth"
          unit="levels per side"
          dp={0}
          min={5}
          max={200}
          step={5}
          value={String(prefs.bookDepth)}
          onValueChange={(v) => prefs.set('bookDepth', Math.max(5, parseInt(v) || 5))}
        />
        <div className="hr" />
        <SwitchRow
          label="Reconnect automatically"
          description="Reconnects with backoff and reconciles on return, briefly flashing changed cells rather than clearing the screen."
          checked={prefs.autoReconnect}
          onCheckedChange={(v) => prefs.set('autoReconnect', v)}
        />
      </PanelBody>
    </Panel>
  )
}

function NumbersSection() {
  const prefs = usePrefs()
  const sample = 1234567.891
  return (
    <Panel>
      <PanelHeader
        title="Formatting"
        actions={<ResetGroup label="Number format" keys={['compactAggregates', 'timezone', 'clock24h', 'weekStartsMonday']} />}
      />
      <PanelBody>
        <SwitchRow
          label="Abbreviate large aggregates"
          description="Shows $1.24M on headline figures. Order tickets, fills, positions and balances always show full precision, whatever this says — that rule is not configurable."
          checked={prefs.compactAggregates}
          onCheckedChange={(v) => prefs.set('compactAggregates', v)}
        />
        <div className="hr" />
        <Field label="Timestamps">
          <Segmented
            ariaLabel="Timezone"
            wide
            value={prefs.timezone}
            onChange={(v) => prefs.set('timezone', v as Prefs['timezone'])}
            options={[
              { value: 'local', label: 'Local' },
              { value: 'utc', label: 'UTC' },
              { value: 'exchange', label: 'Exchange' },
            ]}
          />
        </Field>
        <div style={{ height: 'var(--s-3)' }} />
        <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 'var(--s-3)' }}>
          <SwitchRow label="24-hour clock" checked={prefs.clock24h} onCheckedChange={(v) => prefs.set('clock24h', v)} />
          <SwitchRow label="Week starts Monday" checked={prefs.weekStartsMonday} onCheckedChange={(v) => prefs.set('weekStartsMonday', v)} />
        </div>
        <div className="hr" />
        <Label>How numbers render right now</Label>
        <div className="well" style={{ marginTop: 'var(--s-2)' }}>
          <KeyValue k="Money" v={money(sample)} />
          <KeyValue k="Gain" v={<span className="pos">{signedMoney(sample)}</span>} />
          <KeyValue k="Loss" v={<span className="neg">{signedMoney(-sample)}</span>} />
          <KeyValue k="Zero" v={money(0)} />
          <KeyValue k="Return" v={<span className="pos">{signedPct(2.19)}</span>} />
          <KeyValue k="Allocation" v={pct(23.34)} />
          <KeyValue k="Win rate" v={pct(54, 0)} />
          <KeyValue k="BTC price (2dp)" v={fmtPrice('BTC-USD', 64318.4)} />
          <KeyValue k="EURUSD price (4dp)" v={fmtPrice('EURUSD', 1.10423)} />
          <KeyValue k="Time" v={<span className="mono">{clockTime(Date.now())}</span>} />
        </div>
        <p className="mut" style={{ fontSize: 'var(--t-11)', marginTop: 'var(--s-3)', lineHeight: 1.5 }}>
          Decimal places are fixed per instrument from its metadata, never "as many as needed", so the same
          price never renders two ways on one screen. The minus sign is U+2212, not a hyphen. Every changing
          number uses tabular figures so columns do not jitter as prices tick.
        </p>
      </PanelBody>
    </Panel>
  )
}

function StorageSection() {
  const prefs = usePrefs()
  const toast = useToast()
  const watchlist = useWatchlistStore()
  const [importOpen, setImportOpen] = useState(false)
  const [importText, setImportText] = useState('')
  const [clearing, setClearing] = useState<null | { key: string; label: string; detail: string }>(null)
  const [copied, setCopied] = useState(false)

  const stores = [
    { key: 'tb-trading-workspace', label: 'Trading desk layouts', detail: 'Panels, widths and chart settings for both environments.' },
    { key: 'tb-watchlist', label: 'Watchlist', detail: `${watchlist.symbols.length} pinned markets.` },
    { key: 'meridian-alerts', label: 'Price alerts', detail: 'Alerts held locally until the notification service exists.' },
    { key: PREFS_STORAGE_KEY, label: 'Interface preferences', detail: 'Everything on this Settings page.' },
  ]

  function sizeOf(key: string): string {
    try {
      const v = localStorage.getItem(key)
      if (!v) return 'empty'
      const kb = new Blob([v]).size / 1024
      return kb < 1 ? '<1 KB' : `${kb.toFixed(1)} KB`
    } catch {
      return DASH
    }
  }

  return (
    <>
      <Panel>
        <PanelHeader title="What this browser is holding" />
        <PanelBody flush>
          {stores.map((s) => (
            <div key={s.key} className="between" style={{ padding: 'var(--s-3) var(--s-4)', borderBottom: '1px solid var(--line-hairline)' }}>
              <div style={{ minWidth: 0 }}>
                <div style={{ fontSize: 'var(--t-13)', fontWeight: 'var(--w-medium)' }}>{s.label}</div>
                <div className="mut" style={{ fontSize: 'var(--t-11)', marginTop: 2 }}>
                  {s.detail} {'·'} <span className="num">{sizeOf(s.key)}</span>
                </div>
              </div>
              <Button size="sm" onClick={() => setClearing(s)}>
                Clear
              </Button>
            </div>
          ))}
        </PanelBody>
        <PanelFooter>
          <Button
            size="sm"
            icon={copied ? <Check size={13} aria-hidden /> : <Copy size={13} aria-hidden />}
            onClick={async () => {
              try {
                await navigator.clipboard.writeText(prefs.export())
                setCopied(true)
                window.setTimeout(() => setCopied(false), 1600)
              } catch {
                toast({ title: 'Could not copy to the clipboard', variant: 'error' })
              }
            }}
          >
            {copied ? 'Copied' : 'Copy settings JSON'}
          </Button>
          <Button
            size="sm"
            icon={<Download size={13} aria-hidden />}
            onClick={() => {
              const url = URL.createObjectURL(new Blob([prefs.export()], { type: 'application/json' }))
              const a = document.createElement('a')
              a.href = url
              a.download = 'meridian-settings.json'
              a.click()
              URL.revokeObjectURL(url)
            }}
          >
            Export
          </Button>
          <Button size="sm" icon={<Upload size={13} aria-hidden />} onClick={() => setImportOpen(true)}>
            Import
          </Button>
        </PanelFooter>
      </Panel>

      <Modal
        open={importOpen}
        onClose={() => setImportOpen(false)}
        title="Import settings"
        description="Paste a settings JSON exported from this or another device."
        footer={
          <>
            <Button onClick={() => setImportOpen(false)}>Cancel</Button>
            <Button
              variant="primary"
              disabled={!importText.trim()}
              onClick={() => {
                if (prefs.import(importText)) {
                  toast({ title: 'Settings imported' })
                  setImportOpen(false)
                  setImportText('')
                } else {
                  toast({ title: 'That is not valid settings JSON', variant: 'error' })
                }
              }}
            >
              Import
            </Button>
          </>
        }
      >
        <TextArea
          label="Settings JSON"
          rows={12}
          value={importText}
          onChange={(e) => setImportText(e.target.value)}
          placeholder="{ ... }"
          hint="Unknown or mistyped keys are ignored rather than applied."
        />
      </Modal>

      <ConfirmDialog
        open={!!clearing}
        onCancel={() => setClearing(null)}
        onConfirm={() => {
          if (clearing) {
            try {
              localStorage.removeItem(clearing.key)
            } catch {
              /* ignore */
            }
            toast({ title: `${clearing.label} cleared`, description: 'Reload the page to see the effect.' })
          }
          setClearing(null)
        }}
        title={`Clear ${clearing?.label.toLowerCase()}?`}
        confirmLabel="Clear"
        consequence={<>{clearing?.detail} This affects this browser only and cannot be undone.</>}
      />
    </>
  )
}

/* =============================================================================
   CONNECTIONS
   ============================================================================= */

function VenuesSection() {
  return (
    <Panel>
      <PanelBody>
        <p className="sec" style={{ fontSize: 'var(--t-12)', lineHeight: 1.55, marginBottom: 'var(--s-4)' }}>
          Connect your venue accounts. Credentials are checked against the venue before they are saved, and
          never returned in plaintext — a saved key shows only its last four characters.
        </p>
        <VenueCredentials />
      </PanelBody>
    </Panel>
  )
}

function AiSection() {
  return (
    <Panel>
      <PanelBody>
        <p className="sec" style={{ fontSize: 'var(--t-12)', lineHeight: 1.55, marginBottom: 'var(--s-4)' }}>
          Connect an LLM provider for the research agent. Keys are verified against the provider, encrypted
          at rest, and never returned.
        </p>
        <LlmCredentialForm />
      </PanelBody>
    </Panel>
  )
}

function AgentSection() {
  const prefs = usePrefs()
  return (
    <Panel>
      <PanelHeader
        title="Autonomy and budget"
        actions={<ResetGroup label="Agent settings" keys={['agentApprovalPolicy', 'agentDailyBudgetUsd', 'agentMaxParallelJobs']} />}
      />
      <PanelBody>
        <Field
          label="Ask before acting"
          hint="The agent can research freely. This governs when it must stop and ask a human."
        >
          <Segmented
            ariaLabel="Approval policy"
            wide
            value={prefs.agentApprovalPolicy}
            onChange={(v) => prefs.set('agentApprovalPolicy', v as Prefs['agentApprovalPolicy'])}
            options={[
              { value: 'always', label: 'Always ask', title: 'Every plan and every job waits for approval.' },
              { value: 'above_budget', label: 'Over budget', title: 'Only asks when a step would exceed the daily budget.' },
              { value: 'never', label: 'Never ask', title: 'Runs to completion unattended. It still cannot place orders.' },
            ]}
          />
        </Field>
        <div style={{ height: 'var(--s-3)' }} />
        <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 'var(--s-3)' }}>
          <NumberField
            label="Daily budget"
            unit="USD"
            dp={0}
            step={5}
            min={0}
            value={String(prefs.agentDailyBudgetUsd)}
            onValueChange={(v) => prefs.set('agentDailyBudgetUsd', Number(v) || 0)}
          />
          <NumberField
            label="Parallel jobs"
            dp={0}
            min={1}
            max={8}
            value={String(prefs.agentMaxParallelJobs)}
            onValueChange={(v) => prefs.set('agentMaxParallelJobs', Math.max(1, parseInt(v) || 1))}
          />
        </div>
        <div className="callout info" style={{ marginTop: 'var(--s-4)' }}>
          <Bot size={13} aria-hidden style={{ flex: 'none', marginTop: 2 }} />
          <span>
            The agent researches, backtests and reports. It cannot place an order under any policy — that
            path does not exist for it, by design.
          </span>
        </div>
      </PanelBody>
    </Panel>
  )
}

/* =============================================================================
   ACCOUNT
   ============================================================================= */

function NotificationsSection() {
  const prefs = usePrefs()
  const rows: [keyof Prefs, string, string][] = [
    ['notifyOnFill', 'Order fills', 'Announce a fill in-app and to screen readers.'],
    ['notifyOnOrderReject', 'Rejected orders', 'Tell me when the risk gate or a venue refuses an order.'],
    ['notifyOnAutomationError', 'Automation errors', 'Tell me when a running automation stops or errors.'],
    ['notifyOnRiskBreach', 'Risk breaches', 'Margin utilisation or the daily loss limit crossed.'],
    ['notifyOnBacktestDone', 'Backtest finished', 'A queued run completed or failed.'],
    ['notifyOnAgentApproval', 'Agent needs a decision', 'A research session is blocked waiting on a human.'],
    ['notifyOnConnectionLoss', 'Connection lost', 'The market-data feed dropped.'],
  ]
  return (
    <Panel>
      <PanelHeader title="In-app notifications" actions={<ResetGroup label="Notifications" keys={rows.map((r) => r[0])} />} />
      <PanelBody>
        {rows.map(([key, label, description], i) => (
          <div key={key}>
            {i > 0 && <div className="hr" />}
            <SwitchRow
              label={label}
              description={description}
              checked={prefs[key] as boolean}
              onCheckedChange={(v) => prefs.set(key, v as never)}
            />
          </div>
        ))}
        <div className="callout neutral" style={{ marginTop: 'var(--s-4)' }}>
          <Bell size={13} aria-hidden style={{ flex: 'none', marginTop: 2 }} />
          <span>
            In-app only for now. Email and push need a notification service; these choices carry over when
            one exists.
          </span>
        </div>
      </PanelBody>
    </Panel>
  )
}

const KEY_GROUPS: { group: string; keys: [string, string][] }[] = [
  {
    group: 'Global',
    keys: [
      ['⌘K', 'Command palette'],
      ['⌘/', 'Shortcut sheet'],
      ['⌘\\', 'Toggle theme'],
      ['⌘.', 'Toggle density'],
      ['g then d', 'Dashboard'],
      ['g then t', 'Trading desk'],
      ['g then s', 'Strategy'],
      ['g then a', 'Automations'],
      ['g then b', 'Back testing'],
      ['g then m', 'Models'],
      ['g then r', 'Research'],
    ],
  },
  {
    group: 'Terminal',
    keys: [
      ['b', 'Focus the buy ticket'],
      ['s', 'Focus the sell ticket'],
      ['Esc', 'Clear the ticket'],
      ['⌘↵', 'Submit (with confirmation)'],
      ['1 – 6', 'Switch timeframe'],
      ['/', 'Focus watchlist search'],
    ],
  },
  {
    group: 'Strategy canvas',
    keys: [
      ['Space + drag', 'Pan'],
      ['⌘0', 'Fit to view'],
      ['⌘+ / ⌘−', 'Zoom'],
      ['Del', 'Delete selection'],
      ['⌘D', 'Duplicate block'],
      ['Tab', 'Cycle blocks'],
    ],
  },
]

function KeyboardSection() {
  return (
    <Panel>
      <PanelBody>
        <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fit,minmax(240px,1fr))', gap: 'var(--s-6)' }}>
          {KEY_GROUPS.map((g) => (
            <div key={g.group}>
              <Label>{g.group}</Label>
              <div style={{ marginTop: 'var(--s-2)' }}>
                {g.keys.map(([k, label]) => (
                  <div key={k} className="kv" style={{ padding: '5px 0' }}>
                    <span style={{ color: 'var(--fg-secondary)' }}>{label}</span>
                    <kbd
                      className="mono"
                      style={{
                        fontSize: 'var(--t-11)',
                        background: 'var(--bg-sunken)',
                        border: '1px solid var(--line-hairline)',
                        borderRadius: 'var(--r-xs)',
                        padding: '2px 6px',
                        color: 'var(--fg-secondary)',
                      }}
                    >
                      {k}
                    </kbd>
                  </div>
                ))}
              </div>
            </div>
          ))}
        </div>
      </PanelBody>
    </Panel>
  )
}

function AccountSection() {
  const { user, logout } = useAuthStore()
  return (
    <Panel>
      <PanelBody>
        <KeyValue k="Email" v={user?.email ?? DASH} />
        <KeyValue k="User id" v={<span className="mono">{user?.id ?? DASH}</span>} />
        <KeyValue k="Member since" v={user?.created_at ? dateTime(user.created_at) : DASH} />
        <div className="hr" />
        <Label>Sessions and devices</Label>
        <div style={{ marginTop: 'var(--s-2)' }}>
          <EmptyState
            message="Session management is not served yet"
            detail="When the platform exposes active sessions, they are listed here with a way to revoke each one."
          />
        </div>
      </PanelBody>
      <PanelFooter>
        <Button icon={<LogOut size={13} aria-hidden />} onClick={() => void logout()}>
          Sign out
        </Button>
      </PanelFooter>
    </Panel>
  )
}

function AboutSection() {
  const conn = useConnection()
  const health = useQuery({
    queryKey: ['dashboard-rollup', 'health'],
    queryFn: () => api.get('/api/dashboard/rollup').then(() => true),
    retry: false,
    staleTime: 30_000,
  })

  const services = [
    { name: 'Platform API', ok: health.isSuccess, detail: health.isSuccess ? 'Responding' : 'Unreachable' },
    { name: 'Market data stream', ok: conn.state === 'connected', detail: conn.state },
    { name: 'Order book feed', ok: false, detail: 'No venue depth subscription attached' },
    { name: 'Execution blotter', ok: false, detail: 'Endpoint not implemented' },
    { name: 'Portfolio ledger', ok: false, detail: 'Equity history not recorded' },
    { name: 'Alert service', ok: false, detail: 'Alerts held on this device' },
  ]

  return (
    <Panel>
      <PanelHeader title="Meridian" actions={<Badge tone="neutral">Design system 1.0</Badge>} />
      <PanelBody flush>
        {services.map((s) => (
          <div key={s.name} className="between" style={{ padding: 'var(--s-3) var(--s-4)', borderBottom: '1px solid var(--line-hairline)' }}>
            <span className="row" style={{ gap: 'var(--s-2)' }}>
              <StatusDot state={s.ok ? 'live' : 'degraded'} />
              <span style={{ fontSize: 'var(--t-13)' }}>{s.name}</span>
            </span>
            <Label>{s.detail}</Label>
          </div>
        ))}
      </PanelBody>
      <PanelFooter>
        <Label>
          Services marked amber have a complete interface and a fixed data contract, waiting on a backend.
          Nothing in this product fabricates a number to fill a gap.
        </Label>
      </PanelFooter>
    </Panel>
  )
}

function DangerSection() {
  const prefs = usePrefs()
  const toast = useToast()
  const [resetPrefs, setResetPrefs] = useState(false)
  const [resetPaper, setResetPaper] = useState(false)
  const [resetAll, setResetAll] = useState(false)

  const resetPaperAccount = useMutation({
    mutationFn: () => api.post('/api/paper/reset'),
    onSuccess: () => toast({ title: 'Paper accounts reset', description: 'Balances and positions are back to their starting values.' }),
    onError: () => toast({ title: 'Could not reset the paper accounts', variant: 'error' }),
  })

  const rows = [
    {
      title: 'Reset paper accounts',
      detail: 'Wipes every paper balance, position and fill across all asset classes.',
      action: () => setResetPaper(true),
      danger: true,
    },
    {
      title: 'Reset interface preferences',
      detail: 'Restores everything on this Settings page to its default on this device.',
      action: () => setResetPrefs(true),
      danger: false,
    },
    {
      title: 'Clear all local data',
      detail: 'Preferences, desk layouts, watchlist and alerts. Signs you out of this browser.',
      action: () => setResetAll(true),
      danger: true,
    },
  ]

  return (
    <>
      <Panel style={{ borderColor: 'var(--line-neg)' }}>
        <PanelHeader title="Danger zone" actions={<Badge tone="neg">Irreversible</Badge>} />
        <PanelBody flush>
          {rows.map((r, i) => (
            <div
              key={r.title}
              className="between"
              style={{ padding: 'var(--s-3) var(--s-4)', borderBottom: i < rows.length - 1 ? '1px solid var(--line-hairline)' : undefined }}
            >
              <div style={{ minWidth: 0 }}>
                <div style={{ fontSize: 'var(--t-13)', fontWeight: 'var(--w-medium)' }}>{r.title}</div>
                <div className="mut" style={{ fontSize: 'var(--t-12)', marginTop: 2, lineHeight: 1.45 }}>
                  {r.detail}
                </div>
              </div>
              <Button variant={r.danger ? 'danger' : 'secondary'} size="sm" onClick={r.action}>
                {r.danger ? 'Reset' : 'Restore defaults'}
              </Button>
            </div>
          ))}
        </PanelBody>
      </Panel>

      <ConfirmDialog
        open={resetPaper}
        onCancel={() => setResetPaper(false)}
        onConfirm={() => {
          resetPaperAccount.mutate()
          setResetPaper(false)
        }}
        title="Reset every paper account?"
        confirmLabel="Reset all paper accounts"
        typeToConfirm="RESET"
        consequence={
          <>
            Every paper balance, position and fill across all {ASSET_CLASSES.length} asset classes is wiped
            and returned to its starting balance. Backtests, strategies and automations are untouched. This
            cannot be undone.
          </>
        }
      />

      <ConfirmDialog
        open={resetPrefs}
        onCancel={() => setResetPrefs(false)}
        onConfirm={() => {
          prefs.reset()
          setResetPrefs(false)
          toast({ title: 'Preferences reset' })
        }}
        title="Reset interface preferences?"
        confirmLabel="Reset preferences"
        consequence={
          <>
            Theme, density, chart defaults, ticket defaults, risk limits and notification choices return to
            their defaults on this device. Nothing on the server changes.
          </>
        }
      />

      <ConfirmDialog
        open={resetAll}
        onCancel={() => setResetAll(false)}
        onConfirm={() => {
          try {
            localStorage.clear()
          } catch {
            /* ignore */
          }
          window.location.href = '/login'
        }}
        title="Clear all local data?"
        confirmLabel="Clear everything and sign out"
        typeToConfirm="CLEAR"
        consequence={
          <>
            Preferences, desk layouts, watchlist, alerts and your session are removed from this browser. Your
            account, strategies and server-side automations are untouched.
          </>
        }
      />
    </>
  )
}
