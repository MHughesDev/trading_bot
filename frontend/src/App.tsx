import { useEffect } from 'react'
import { BrowserRouter, Routes, Route, Navigate } from 'react-router-dom'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { useAuthStore } from '@/store/auth'
import { AppLayout } from '@/components/layout/AppLayout'
import { LoginPage } from '@/pages/LoginPage'
import { SignUpPage } from '@/pages/SignUpPage'
import { DashboardPage } from '@/pages/DashboardPage'
import { TradingPage } from '@/pages/TradingPage'
import { AutomationsPage } from '@/pages/AutomationsPage'
import { StrategyCreationPage } from '@/pages/StrategyCreationPage'
import { SettingsPage } from '@/pages/SettingsPage'
// Legacy routes kept for backwards-compat deep links.
import { AssetPage } from '@/pages/AssetPage'
import { AccountPage } from '@/pages/AccountPage'
import { TransactionsPage } from '@/pages/TransactionsPage'

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
        </AuthInit>
      </BrowserRouter>
    </QueryClientProvider>
  )
}
