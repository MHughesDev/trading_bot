import { lazy, Suspense, useEffect, Component } from 'react'
import type { ReactNode, ErrorInfo } from 'react'
import { BrowserRouter, Routes, Route, Navigate, useLocation } from 'react-router-dom'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { useAuthStore } from '@/store/auth'
import { usePrefs } from '@/store/prefs'
import { AppLayout } from '@/components/layout/AppLayout'
import { LoginPage } from '@/pages/LoginPage'
import { SignUpPage } from '@/pages/SignUpPage'
import { ForgotPasswordPage } from '@/pages/ForgotPasswordPage'
import { DashboardPage } from '@/pages/DashboardPage'
import { TradingPage } from '@/pages/TradingPage'
import { TerminalPage } from '@/pages/TerminalPage'
import { AutomationsPage } from '@/pages/AutomationsPage'
import { SettingsPage } from '@/pages/SettingsPage'
import { MarketsPage } from '@/pages/MarketsPage'
import { TransactionsPage } from '@/pages/TransactionsPage'

// Heavy routes are code-split into their own chunks so they stay out of the
// main bundle and load on demand (#29).  These pages pull in the visual
// strategy builder and the backtesting UI, which dominate bundle size.
const BackTestingPage = lazy(() =>
  import('@/pages/BackTestingPage').then((m) => ({ default: m.BackTestingPage })),
)
const StrategyCreationPage = lazy(() =>
  import('@/pages/StrategyCreationPage').then((m) => ({ default: m.StrategyCreationPage })),
)
const MlOpsPage = lazy(() =>
  import('@/pages/MlOpsPage').then((m) => ({ default: m.MlOpsPage })),
)
const ModelDetailPage = lazy(() =>
  import('@/pages/ModelDetailPage').then((m) => ({ default: m.ModelDetailPage })),
)
const ModelCreatePage = lazy(() =>
  import('@/pages/ModelCreatePage').then((m) => ({ default: m.ModelCreatePage })),
)
const ModelLineagePage = lazy(() =>
  import('@/pages/ModelLineagePage').then((m) => ({ default: m.ModelLineagePage })),
)
const LeaderboardPage = lazy(() =>
  import('@/pages/LeaderboardPage').then((m) => ({ default: m.LeaderboardPage })),
)
// Lazy, like the other heavy pages: the workspace pulls in the timeline, the panes
// and their polling, and most visits to the app never open it.
const ResearchWorkspacePage = lazy(() =>
  import('@/pages/ResearchWorkspacePage').then((m) => ({
    default: m.ResearchWorkspacePage,
  })),
)
const ApprovalsPage = lazy(() =>
  import('@/pages/ApprovalsPage').then((m) => ({ default: m.ApprovalsPage })),
)
const AgentPage = lazy(() =>
  import('@/pages/AgentPage').then((m) => ({ default: m.AgentPage })),
)
// The design-system preview is public on purpose: the system must be reviewable
// without a session (spec §8.7 — every state reachable outside the app).
const DesignSystemPage = lazy(() =>
  import('@/pages/DesignSystemPage').then((m) => ({ default: m.DesignSystemPage })),
)

const qc = new QueryClient({
  defaultOptions: {
    queries: { retry: 1, staleTime: 5000 },
  },
})

class ErrorBoundary extends Component<{ children: ReactNode }, { error: Error | null }> {
  state = { error: null }
  static getDerivedStateFromError(error: Error) { return { error } }
  componentDidCatch(_: Error, info: ErrorInfo) { console.error('[ErrorBoundary]', _, info) }
  render() {
    if (this.state.error) {
      const msg = (this.state.error as Error).message
      return (
        <div className="auth-shell">
          <div className="auth-card">
            <div className="panel">
              <div className="panel-hd">
                <span className="panel-title">Something broke</span>
              </div>
              <div className="panel-bd">
                <p className="sec" style={{ fontSize: 'var(--t-13)', lineHeight: 1.5 }}>
                  This screen hit an error it could not recover from. Nothing was sent and no order
                  was placed. The details below are also in the browser console.
                </p>
                <pre
                  className="mono well"
                  style={{ marginTop: 'var(--s-4)', whiteSpace: 'pre-wrap', fontSize: 'var(--t-11)', overflow: 'auto', maxHeight: 240 }}
                >
                  {msg}
                </pre>
              </div>
              <div className="panel-ft">
                <button type="button" className="btn" onClick={() => this.setState({ error: null })}>
                  Try again
                </button>
                <button type="button" className="btn primary" onClick={() => { window.location.href = '/dashboard' }}>
                  Back to the dashboard
                </button>
              </div>
            </div>
          </div>
        </div>
      )
    }
    return this.props.children
  }
}

// Forwards legacy "/models/..." deep links to the renamed "/mlops/..." routes,
// preserving the sub-path and query string (e.g. /models/abc?tab=test).
function ModelsRedirect() {
  const loc = useLocation()
  const target = loc.pathname.replace(/^\/models/, '/mlops') + loc.search
  return <Navigate to={target} replace />
}

/** Route-level loading: skeletons shaped like the page, never a bare spinner. */
function RouteSkeleton() {
  return (
    <div style={{ padding: 'var(--s-5)', display: 'flex', flexDirection: 'column', gap: 'var(--s-3)' }}>
      <span className="skel" style={{ height: 28, width: 220 }} />
      <span className="skel" style={{ height: 120 }} />
      <span className="skel" style={{ height: '40vh' }} />
      <span className="sr-only">Loading</span>
    </div>
  )
}

/** /asset/:symbol → /terminal/:symbol, preserving the symbol. */
function AssetRedirect() {
  const loc = useLocation()
  const symbol = loc.pathname.replace(/^\/asset\//, '')
  return <Navigate to={`/terminal/${symbol}${loc.search}`} replace />
}

/** Sends the user wherever they asked the app to open (Settings → Workspace). */
function LandingRedirect() {
  const landing = usePrefs((s) => s.landingPage)
  return <Navigate to={landing} replace />
}

function AuthInit({ children }: { children: React.ReactNode }) {
  const { fetchMe } = useAuthStore()
  useEffect(() => { fetchMe() }, [fetchMe])
  return <>{children}</>
}

export default function App() {
  return (
    <QueryClientProvider client={qc}>
      <BrowserRouter>
        <AuthInit>
          <ErrorBoundary>
          <Suspense fallback={<RouteSkeleton />}>
          <Routes>
            <Route path="/login" element={<LoginPage />} />
            <Route path="/signup" element={<SignUpPage />} />
            <Route path="/forgot-password" element={<ForgotPasswordPage />} />
            <Route path="/design" element={<DesignSystemPage />} />

            <Route element={<AppLayout />}>
              <Route index element={<LandingRedirect />} />

              {/* Five primary sections */}
              <Route path="/dashboard" element={<DashboardPage />} />
              <Route path="/trading" element={<TradingPage />} />
              {/* The per-asset terminal: one market, full depth (spec §4.2). */}
              <Route path="/terminal" element={<TerminalPage />} />
              <Route path="/terminal/:symbol" element={<TerminalPage />} />
              <Route path="/automations" element={<AutomationsPage />} />
              <Route path="/strategy" element={<StrategyCreationPage />} />
              <Route path="/backtesting" element={<BackTestingPage />} />
              <Route path="/workbench" element={<Navigate to="/backtesting" replace />} />
              <Route path="/proving-ground" element={<Navigate to="/backtesting" replace />} />
              <Route path="/mlops" element={<MlOpsPage />} />
              <Route path="/mlops/create" element={<ModelCreatePage />} />
              <Route path="/mlops/graph" element={<ModelLineagePage />} />
              <Route path="/mlops/leaderboard" element={<LeaderboardPage />} />
              <Route path="/mlops/:id" element={<ModelDetailPage />} />
              <Route path="/agent" element={<AgentPage />} />
              {/* The research workspace (COMP-006). One layout, no modes: a Desk
                  question and an overnight campaign are the same screen. */}
              <Route path="/research" element={<ResearchWorkspacePage />} />
              <Route
                path="/research/:projectId"
                element={<ResearchWorkspacePage />}
              />
              <Route
                path="/research/:projectId/sessions/:sessionId"
                element={<ResearchWorkspacePage />}
              />
              <Route path="/approvals" element={<ApprovalsPage />} />
              <Route path="/settings" element={<SettingsPage />} />

              <Route path="/markets" element={<MarketsPage />} />
              <Route path="/transactions" element={<TransactionsPage />} />

              {/* Legacy deep links.
                  /asset was a per-symbol page that duplicated the terminal; the
                  terminal is the one place a single market is charted and traded,
                  and /markets is where the set of markets is managed.
                  /account duplicated Settings. */}
              <Route path="/asset/:symbol" element={<AssetRedirect />} />
              <Route path="/asset" element={<Navigate to="/markets" replace />} />
              <Route path="/account" element={<Navigate to="/settings" replace />} />
              <Route path="/strategy-builder" element={<Navigate to="/strategy" replace />} />
              {/* ML Ops was renamed from "AI Models"; keep old /models* deep links alive. */}
              <Route path="/models/*" element={<ModelsRedirect />} />

              <Route path="*" element={<Navigate to="/dashboard" replace />} />
            </Route>
          </Routes>
          </Suspense>
          </ErrorBoundary>
        </AuthInit>
      </BrowserRouter>
    </QueryClientProvider>
  )
}
