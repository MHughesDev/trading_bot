import { useEffect, useRef } from 'react'
import { settingsApi } from '@/lib/api'
import { usePrefs, PREF_DEFAULTS, type Prefs } from '@/store/prefs'

/* =============================================================================
   Settings sync.

   Preferences live on the device, because a phone and a multi-monitor desk want
   different answers to most of them. A subset does NOT: risk limits, order
   defaults and the agent's autonomy policy describe how the user trades, not
   which screen they are on, and losing them when they open a second browser is
   a bug.

   Those keys are mirrored to `/api/settings`, pulled once on sign-in and pushed
   (debounced) whenever they change. Device-only keys are never sent.
   ============================================================================= */

/** The keys that follow the user between devices. */
export const SYNCED_KEYS = [
  'defaultOrderType',
  'defaultTif',
  'defaultSizeUnit',
  'defaultSizePct',
  'defaultLeverage',
  'attachBracketByDefault',
  'defaultStopPct',
  'defaultTakeProfitPct',
  'reduceOnlyByDefault',
  'confirmEveryOrder',
  'maxRiskPerTradePct',
  'maxConcurrentPositions',
  'dailyLossCircuitBreakerPct',
  'marginLimitPct',
  'maxOrderNotional',
  'largeOrderWarnPctOfEquity',
  'automationDefaultTrigger',
  'automationDefaultTimeframe',
  'automationArmOnCreate',
  'backtestDefaultBalance',
  'backtestDefaultQuote',
  'backtestDefaultDays',
  'backtestAutoCollect',
  'agentApprovalPolicy',
  'agentDailyBudgetUsd',
  'agentMaxParallelJobs',
  'notifyOnFill',
  'notifyOnOrderReject',
  'notifyOnAutomationError',
  'notifyOnRiskBreach',
  'notifyOnBacktestDone',
  'notifyOnAgentApproval',
  'notifyOnConnectionLoss',
] as const satisfies readonly (keyof Prefs)[]

function syncedSlice(state: Prefs): Partial<Prefs> {
  const out: Record<string, unknown> = {}
  for (const k of SYNCED_KEYS) out[k] = state[k]
  return out as Partial<Prefs>
}

/**
 * Pulls the server copy once, then pushes local changes with a short debounce.
 * Mount once, in the app shell, after the user is known.
 */
export function useSettingsSync(enabled: boolean) {
  const pulled = useRef(false)
  const timer = useRef<number>(0)
  const lastPushed = useRef<string>('')

  // Pull once per session. The server copy wins on first load — a second device
  // should adopt the user's limits, not overwrite them with its defaults.
  useEffect(() => {
    if (!enabled || pulled.current) return
    pulled.current = true
    void settingsApi
      .get()
      .then(({ data }) => {
        if (!data || typeof data !== 'object') return
        const patch: Record<string, unknown> = {}
        for (const k of SYNCED_KEYS) {
          const v = (data as Record<string, unknown>)[k]
          if (v !== undefined && typeof v === typeof PREF_DEFAULTS[k]) patch[k] = v
        }
        const store = usePrefs.getState()
        for (const [k, v] of Object.entries(patch)) {
          store.set(k as keyof Prefs, v as never)
        }
        lastPushed.current = JSON.stringify(syncedSlice(usePrefs.getState()))
      })
      .catch(() => {
        // A settings service that is down must not block the interface; the
        // device copy is already loaded and correct.
      })
  }, [enabled])

  // Push on change, debounced, and only when the synced slice actually differs.
  useEffect(() => {
    if (!enabled) return
    return usePrefs.subscribe((state) => {
      const slice = syncedSlice(state)
      const json = JSON.stringify(slice)
      if (json === lastPushed.current) return
      window.clearTimeout(timer.current)
      timer.current = window.setTimeout(() => {
        lastPushed.current = json
        void settingsApi.put(slice as Record<string, unknown>).catch(() => {
          // Leave `lastPushed` set: retrying every keystroke against a service
          // that is refusing would be worse than waiting for the next change.
        })
      }, 800)
    })
  }, [enabled])
}
