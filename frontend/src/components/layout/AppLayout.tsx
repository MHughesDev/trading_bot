import { useEffect, useRef, useState } from 'react'
import { Navigate, Outlet, useLocation, useNavigate } from 'react-router-dom'
import { useAuthStore } from '@/store/auth'
import { useThemeStore } from '@/store/theme'
import { useConnection } from '@/hooks/useConnection'
import { useSettingsSync } from '@/hooks/useSettingsSync'
import { TopBar } from './TopBar'
import { CommandPalette, ShortcutSheet } from './CommandPalette'
import { ToastContainer } from '@/components/ToastContainer'
import { DisconnectedBanner, Skeleton } from '@/components/primitives/States'
import { initWsClient } from '@/api/ws'
import { getStoredToken } from '@/lib/api'
import { registerInstruments } from '@/lib/format'
import { fetchInstruments } from '@/api/portfolio'
import { toSpecAssetClass } from '@/lib/assetClass'

/* Routes that own their own scroll and must fill the viewport exactly. */
const FULL_HEIGHT_ROUTES = ['/trading', '/terminal', '/dashboard', '/agent', '/strategy', '/research']

/** `g` then a letter jumps between sections (spec §5.6). */
const GOTO: Record<string, string> = {
  d: '/dashboard',
  t: '/trading',
  s: '/strategy',
  a: '/automations',
  b: '/backtesting',
  m: '/mlops',
  g: '/agent',
  r: '/research',
}

export function AppLayout() {
  const { user, initialized } = useAuthStore()
  const { toggleTheme, toggleDensity } = useThemeStore()
  const location = useLocation()
  const navigate = useNavigate()
  const connection = useConnection()
  // Risk limits and order defaults follow the user between devices (§Settings).
  useSettingsSync(!!user)
  const [palette, setPalette] = useState(false)
  const [shortcuts, setShortcuts] = useState(false)
  const gPressed = useRef(0)

  // Ensure the live socket exists for this session. Deliberately does NOT tear
  // it down on unmount: the client is a module singleton shared by every panel,
  // and destroying it here would drop subscriptions made by children that
  // mounted first. `logout()` owns the teardown.
  useEffect(() => {
    if (!user) return
    initWsClient(getStoredToken() ?? 'dev-local')
  }, [user])

  // Instrument metadata drives per-instrument decimal precision (spec N2).
  useEffect(() => {
    if (!user) return
    void fetchInstruments().then((rows) => {
      registerInstruments(
        rows.map((r) => ({
          symbol: r.symbol ?? r.id,
          priceDp: r.price_dp ?? (r.asset_class === 'fx' ? 4 : 2),
          qtyDp: r.qty_dp ?? (r.asset_class === 'equity' || r.asset_class === 'etf' ? 0 : 4),
          quote: r.quote ?? 'USD',
          assetClass: toSpecAssetClass(r.asset_class),
        })),
      )
    })
  }, [user])

  useEffect(() => {
    function onShortcutsEvent() { setShortcuts(true) }
    window.addEventListener('meridian:shortcuts', onShortcutsEvent)
    return () => window.removeEventListener('meridian:shortcuts', onShortcutsEvent)
  }, [])

  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      const el = e.target as HTMLElement | null
      const typing =
        el?.tagName === 'INPUT' ||
        el?.tagName === 'TEXTAREA' ||
        el?.tagName === 'SELECT' ||
        el?.isContentEditable

      const meta = e.metaKey || e.ctrlKey
      if (meta && e.key.toLowerCase() === 'k') { e.preventDefault(); setPalette((p) => !p); return }
      if (meta && e.key === '/') { e.preventDefault(); setShortcuts((s) => !s); return }
      if (meta && e.key === '\\') { e.preventDefault(); toggleTheme(); return }
      if (meta && e.key === '.') { e.preventDefault(); toggleDensity(); return }
      if (typing) return

      if (e.key === 'g') { gPressed.current = Date.now(); return }
      if (Date.now() - gPressed.current < 1200) {
        const path = GOTO[e.key.toLowerCase()]
        if (path) { e.preventDefault(); gPressed.current = 0; navigate(path) }
      }
    }
    document.addEventListener('keydown', onKey)
    return () => document.removeEventListener('keydown', onKey)
  }, [navigate, toggleTheme, toggleDensity])

  if (!initialized) {
    return (
      <div className="app" style={{ padding: 'var(--s-5)' }}>
        <Skeleton height={56} />
        <div style={{ height: 'var(--s-3)' }} />
        <Skeleton height={120} />
        <div style={{ height: 'var(--s-3)' }} />
        <Skeleton height="40vh" />
        <span className="sr-only">Loading your session</span>
      </div>
    )
  }

  if (!user) return <Navigate to="/login" replace />

  const fullHeight = FULL_HEIGHT_ROUTES.some(
    (r) => location.pathname === r || location.pathname.startsWith(r + '/'),
  )

  return (
    <div className="app">
      <a href="#main" className="skiplink">
        Skip to content
      </a>
      <TopBar onOpenPalette={() => setPalette(true)} />
      <DisconnectedBanner state={connection.state} />
      <main
        id="main"
        className={fullHeight ? 'flex-1 overflow-hidden' : 'flex-1 overflow-y-auto'}
        style={{ minHeight: 0, display: 'flex', flexDirection: 'column' }}
      >
        <Outlet />
      </main>
      <div id="env-live-region" className="sr-only" aria-live="polite" />
      <ToastContainer />
      <CommandPalette open={palette} onClose={() => setPalette(false)} />
      <ShortcutSheet open={shortcuts} onClose={() => setShortcuts(false)} />
    </div>
  )
}
