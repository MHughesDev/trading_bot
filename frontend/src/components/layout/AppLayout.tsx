import { useEffect } from 'react'
import { Navigate, Outlet, useLocation } from 'react-router-dom'
import { useAuthStore } from '@/store/auth'
import { GlassPillNav } from './GlassPillNav'
import { ToastContainer } from '@/components/ToastContainer'
import { initWsClient, destroyWsClient } from '@/api/ws'
import { getStoredToken } from '@/lib/api'

const FULL_HEIGHT_ROUTES = ['/trading', '/dashboard']

export function AppLayout() {
  const { user, initialized } = useAuthStore()
  const location = useLocation()

  useEffect(() => {
    if (!user) return
    initWsClient(getStoredToken() ?? 'dev-local')
    return () => destroyWsClient()
  }, [user])

  if (!initialized) {
    return (
      <div className="flex h-screen items-center justify-center bg-background">
        <div className="h-6 w-6 animate-spin rounded-full border-2 border-blue-500 border-t-transparent" />
      </div>
    )
  }

  if (!user) return <Navigate to="/login" replace />

  const isFullHeight = FULL_HEIGHT_ROUTES.some(
    (r) => location.pathname === r || location.pathname.startsWith(r + '/'),
  )

  return (
    <div className="flex flex-col h-screen overflow-hidden bg-background">
      <GlassPillNav />
      <main className={isFullHeight ? 'flex-1 overflow-hidden' : 'flex-1 overflow-y-auto'}>
        <Outlet />
      </main>
      <ToastContainer />
    </div>
  )
}
