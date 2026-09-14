import { create } from 'zustand'
import { persist } from 'zustand/middleware'

/* =============================================================================
   Per-device product preferences.

   Settings that change how the interface behaves rather than what the account
   is. They live on the device, like theme and density — a user's laptop and
   their multi-monitor desk want different answers to most of these.

   Anything that must hold across devices (risk limits the server enforces,
   venue credentials, account identity) does NOT belong here.

   Spec references: §2.6 M2 (tick flash disableable), §2.2.5 (number format),
   §5.2 (staleness), §5.5 (confirmation policy), §6 (accessibility).
   ============================================================================= */

export type OrderType = 'market' | 'limit'
export type Tif = 'gtc' | 'day' | 'ioc' | 'fok'
export type ChartStyle = 'candles' | 'bars' | 'line' | 'area'
export type LandingPage = '/dashboard' | '/trading' | '/terminal' | '/strategy' | '/agent'
export type SizeUnit = 'base' | 'quote'
export type ApprovalPolicy = 'always' | 'above_budget' | 'never'

export interface Prefs {
  /* --- appearance ------------------------------------------------------- */
  tickFlash: boolean
  /** Always show ▲/▼ beside a directional figure, not only when unsigned. */
  alwaysShowDirectionGlyph: boolean
  /** Underline every link, not only on hover. */
  underlineLinks: boolean
  /** Force reduced motion even when the OS does not ask for it. */
  reduceMotion: boolean
  /** Thicker focus ring for low-vision use. */
  thickFocusRing: boolean

  /* --- workspace --------------------------------------------------------- */
  landingPage: LandingPage
  /** Re-open the last instrument the terminal was on. */
  rememberLastInstrument: boolean
  lastInstrument: string | null
  /** Collapse workspace panels rather than closing them on double-click. */
  confirmPanelClose: boolean
  /** Keep the trading desk scrolled to a newly added panel. */
  scrollToNewPanel: boolean

  /* --- charts ------------------------------------------------------------ */
  defaultTimeframeSecs: number
  chartStyle: ChartStyle
  showVolume: boolean
  showLastPriceLine: boolean
  showPositionLines: boolean
  showOrderLines: boolean
  logScale: boolean
  extendedHours: boolean
  /** Bars fetched per chart request. */
  chartLookbackDays: number

  /* --- order ticket ------------------------------------------------------ */
  defaultOrderType: OrderType
  defaultTif: Tif
  defaultSizeUnit: SizeUnit
  /** Percent-of-buying-power preset written into the size field. */
  defaultSizePct: number
  defaultLeverage: number
  attachBracketByDefault: boolean
  defaultStopPct: number
  defaultTakeProfitPct: number
  reduceOnlyByDefault: boolean
  confirmEveryOrder: boolean
  /** Clear the ticket after a successful submit. */
  clearTicketAfterSubmit: boolean

  /* --- risk -------------------------------------------------------------- */
  maxRiskPerTradePct: number
  maxConcurrentPositions: number
  dailyLossCircuitBreakerPct: number
  marginLimitPct: number
  maxOrderNotional: number
  /** Warn before an order whose notional exceeds this share of equity. */
  largeOrderWarnPctOfEquity: number

  /* --- market data -------------------------------------------------------- */
  staleAfterMs: number
  /** UI repaint ceiling for streaming panels, frames per second. */
  maxFps: number
  bookDepth: number
  /** Reconnect automatically when the feed drops. */
  autoReconnect: boolean

  /* --- numbers and locale -------------------------------------------------- */
  compactAggregates: boolean
  timezone: 'local' | 'utc' | 'exchange'
  clock24h: boolean
  weekStartsMonday: boolean

  /* --- automations --------------------------------------------------------- */
  automationDefaultTrigger: 'ohlcv_bar' | 'timer'
  automationDefaultTimeframe: string
  automationArmOnCreate: boolean

  /* --- backtesting ---------------------------------------------------------- */
  backtestDefaultBalance: string
  backtestDefaultQuote: string
  backtestDefaultDays: number
  backtestAutoCollect: boolean

  /* --- research agent -------------------------------------------------------- */
  agentApprovalPolicy: ApprovalPolicy
  agentDailyBudgetUsd: number
  agentMaxParallelJobs: number

  /* --- notifications ---------------------------------------------------------- */
  notifyOnFill: boolean
  notifyOnOrderReject: boolean
  notifyOnAutomationError: boolean
  notifyOnRiskBreach: boolean
  notifyOnBacktestDone: boolean
  notifyOnAgentApproval: boolean
  notifyOnConnectionLoss: boolean
}

export const PREF_DEFAULTS: Prefs = {
  tickFlash: true,
  alwaysShowDirectionGlyph: false,
  underlineLinks: false,
  reduceMotion: false,
  thickFocusRing: false,

  landingPage: '/dashboard',
  rememberLastInstrument: true,
  lastInstrument: null,
  confirmPanelClose: false,
  scrollToNewPanel: true,

  defaultTimeframeSecs: 3600,
  chartStyle: 'candles',
  showVolume: true,
  showLastPriceLine: true,
  showPositionLines: true,
  showOrderLines: true,
  logScale: false,
  extendedHours: false,
  chartLookbackDays: 90,

  defaultOrderType: 'limit',
  defaultTif: 'gtc',
  defaultSizeUnit: 'base',
  defaultSizePct: 25,
  defaultLeverage: 1,
  attachBracketByDefault: false,
  defaultStopPct: 1.5,
  defaultTakeProfitPct: 4,
  reduceOnlyByDefault: false,
  confirmEveryOrder: false,
  clearTicketAfterSubmit: true,

  maxRiskPerTradePct: 2,
  maxConcurrentPositions: 3,
  dailyLossCircuitBreakerPct: 4,
  marginLimitPct: 60,
  maxOrderNotional: 250000,
  largeOrderWarnPctOfEquity: 25,

  staleAfterMs: 2000,
  maxFps: 10,
  bookDepth: 25,
  autoReconnect: true,

  compactAggregates: true,
  timezone: 'local',
  clock24h: true,
  weekStartsMonday: true,

  automationDefaultTrigger: 'ohlcv_bar',
  automationDefaultTimeframe: '1h',
  automationArmOnCreate: true,

  backtestDefaultBalance: '100000',
  backtestDefaultQuote: 'USD',
  backtestDefaultDays: 90,
  backtestAutoCollect: true,

  agentApprovalPolicy: 'above_budget',
  agentDailyBudgetUsd: 25,
  agentMaxParallelJobs: 2,

  notifyOnFill: true,
  notifyOnOrderReject: true,
  notifyOnAutomationError: true,
  notifyOnRiskBreach: true,
  notifyOnBacktestDone: true,
  notifyOnAgentApproval: true,
  notifyOnConnectionLoss: true,
}

interface PrefsState extends Prefs {
  set: <K extends keyof Prefs>(key: K, value: Prefs[K]) => void
  /** Reset one group, so a user can undo a bad chart session without losing risk limits. */
  resetGroup: (keys: (keyof Prefs)[]) => void
  reset: () => void
  export: () => string
  import: (json: string) => boolean
}

export const PREFS_STORAGE_KEY = 'meridian-prefs'

export const usePrefs = create<PrefsState>()(
  persist(
    (set) => ({
      ...PREF_DEFAULTS,
      set: (key, value) => set({ [key]: value } as Partial<PrefsState>),
      resetGroup: (keys) =>
        set(() => {
          const patch: Partial<Prefs> = {}
          for (const k of keys) (patch as Record<string, unknown>)[k] = PREF_DEFAULTS[k]
          return patch as Partial<PrefsState>
        }),
      reset: () => set({ ...PREF_DEFAULTS }),
      export: () => {
        const state = usePrefs.getState()
        const out: Record<string, unknown> = {}
        for (const k of Object.keys(PREF_DEFAULTS) as (keyof Prefs)[]) out[k] = state[k]
        return JSON.stringify(out, null, 2)
      },
      import: (json) => {
        try {
          const parsed = JSON.parse(json) as Partial<Prefs>
          const patch: Partial<Prefs> = {}
          for (const k of Object.keys(PREF_DEFAULTS) as (keyof Prefs)[]) {
            if (k in parsed && typeof parsed[k] === typeof PREF_DEFAULTS[k]) {
              ;(patch as Record<string, unknown>)[k] = parsed[k]
            }
          }
          set(patch as Partial<PrefsState>)
          return true
        } catch {
          return false
        }
      },
    }),
    { name: PREFS_STORAGE_KEY, version: 2 },
  ),
)

/* -----------------------------------------------------------------------------
   Accessibility preferences are applied as attributes on <html> so they are
   pure paint, exactly like theme and density.
   -------------------------------------------------------------------------- */
export function applyA11yPrefs(p: Pick<Prefs, 'reduceMotion' | 'thickFocusRing' | 'underlineLinks'>) {
  const el = document.documentElement
  el.toggleAttribute('data-reduce-motion', p.reduceMotion)
  el.toggleAttribute('data-thick-focus', p.thickFocusRing)
  el.toggleAttribute('data-underline-links', p.underlineLinks)
}

if (typeof window !== 'undefined') {
  applyA11yPrefs(usePrefs.getState())
  usePrefs.subscribe((s) => applyA11yPrefs(s))
}
