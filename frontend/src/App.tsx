import { lazy, Suspense, useEffect } from 'react'
import { BrowserRouter, Routes, Route, Navigate } from 'react-router-dom'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { useAuthStore } from '@/store/auth'
import { AppLayout } from '@/components/layout/AppLayout'
import { LoginPage } from '@/pages/LoginPage'
import { SignUpPage } from '@/pages/SignUpPage'
import { DashboardPage } from '@/pages/DashboardPage'
import { TradingPage } from '@/pages/TradingPage'
import { AutomationsPage } from '@/pages/AutomationsPage'
import { SettingsPage } from '@/pages/SettingsPage'
// Legacy routes kept for backwards-compat deep links.
import { AssetPage } from '@/pages/AssetPage'
import { AccountPage } from '@/pages/AccountPage'
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

const qc = new QueryClient({
  defaultOptions: {
    queries: { retry: 1, staleTime: 5000 },
  },
})

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
          <Suspense fallback={<div className="p-8 text-sm text-muted-foreground">Loading…</div>}>
          <Routes>
            <Route path="/login" element={<LoginPage />} />
            <Route path="/signup" element={<SignUpPage />} />

            <Route element={<AppLayout />}>
              <Route index element={<Navigate to="/dashboard" replace />} />

              {/* Five primary sections */}
              <Route path="/dashboard" element={<DashboardPage />} />
              <Route path="/trading" element={<TradingPage />} />
              <Route path="/automations" element={<AutomationsPage />} />
              <Route path="/strategy" element={<StrategyCreationPage />} />
              <Route path="/backtesting" element={<BackTestingPage />} />
              <Route path="/settings" element={<SettingsPage />} />

              {/* Legacy / deep-link routes */}
              <Route path="/asset/:symbol" element={<AssetPage />} />
              <Route path="/asset" element={<AssetPage />} />
              <Route path="/account" element={<AccountPage />} />
              <Route path="/transactions" element={<TransactionsPage />} />
              <Route path="/strategy-builder" element={<Navigate to="/strategy" replace />} />

              <Route path="*" element={<Navigate to="/dashboard" replace />} />
            </Route>
          </Routes>
          </Suspense>
        </AuthInit>
      </BrowserRouter>
    </QueryClientProvider>
  )
}
