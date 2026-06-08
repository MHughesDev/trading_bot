import { useEffect } from 'react'
import { BrowserRouter, Routes, Route, Navigate } from 'react-router-dom'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { useAuthStore } from '@/store/auth'
import { AppLayout } from '@/components/layout/AppLayout'
import { LoginPage } from '@/pages/LoginPage'
import { SignUpPage } from '@/pages/SignUpPage'
import { DashboardPage } from '@/pages/DashboardPage'
import { AssetPage } from '@/pages/AssetPage'
import { AccountPage } from '@/pages/AccountPage'
import { TransactionsPage } from '@/pages/TransactionsPage'
import { StrategyBuilderPage } from '@/pages/StrategyBuilderPage'

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
              <Route path="/dashboard" element={<DashboardPage />} />
              <Route path="/asset/:symbol" element={<AssetPage />} />
              <Route path="/asset" element={<AssetPage />} />
              <Route path="/account" element={<AccountPage />} />
              <Route path="/transactions" element={<TransactionsPage />} />
              <Route path="/strategy-builder" element={<StrategyBuilderPage />} />
              <Route path="*" element={<Navigate to="/dashboard" replace />} />
            </Route>
          </Routes>
        </AuthInit>
      </BrowserRouter>
    </QueryClientProvider>
  )
}
