import { Navigate, Outlet, useLocation } from 'react-router-dom'
import { useAuthStore } from '@/store/auth'
import { Sidebar } from './Sidebar'
import { GlassPillNav } from './GlassPillNav'
import { ToastContainer } from '@/components/ToastContainer'

// The Trading page fills the viewport — no vertical scroll on the main outlet.
const FULL_HEIGHT_ROUTES = ['/trading']

export function AppLayout() {
  const { user, initialized } = useAuthStore()
  const location = useLocation()

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
      {/* Glass-pill top navigation */}
      <GlassPillNav />

      {/* Body: sidebar + main */}
      <div className="flex flex-1 overflow-hidden">
        <Sidebar />
        <main
          className={
            isFullHeight
              ? 'flex-1 overflow-hidden'
              : 'flex-1 overflow-y-auto'
          }
        >
          <Outlet />
        </main>
      </div>

      <ToastContainer />
    </div>
  )
}
