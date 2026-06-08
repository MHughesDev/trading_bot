import { Link, useLocation, useNavigate } from 'react-router-dom'
import { useQuery } from '@tanstack/react-query'
import {
  LayoutDashboard, User, LogOut, Star, Zap, Briefcase, Search,
  ChevronDown, ChevronRight, Settings, Layers, Receipt
} from 'lucide-react'
import { useAuthStore } from '@/store/auth'
import { useWatchlistStore } from '@/store/watchlist'
import { systemApi, portfolioApi } from '@/lib/api'
import { cn, fmtCurrency, pnlClass } from '@/lib/utils'
import { useState } from 'react'

const NAV = [
  { to: '/dashboard', label: 'Dashboard', icon: LayoutDashboard },
  { to: '/asset', label: 'Asset', icon: Search },
  { to: '/strategy-builder', label: 'Strategy Builder', icon: Layers },
  { to: '/transactions', label: 'Transactions', icon: Receipt },
  { to: '/account', label: 'Account', icon: User },
]

function CollapseSection({
  label, children, defaultOpen = true
}: { label: string; children: React.ReactNode; defaultOpen?: boolean }) {
  const [open, setOpen] = useState(defaultOpen)
  return (
    <div>
      <button
        onClick={() => setOpen(!open)}
        className="flex w-full items-center justify-between px-3 py-1.5 text-xs font-semibold uppercase tracking-widest text-text-dim hover:text-text-muted transition-colors"
      >
        {label}
        {open ? <ChevronDown className="h-3 w-3" /> : <ChevronRight className="h-3 w-3" />}
      </button>
      {open && <div className="space-y-0.5">{children}</div>}
    </div>
  )
}

export function Sidebar() {
  const location = useLocation()
  const navigate = useNavigate()
  const { user, logout } = useAuthStore()
  const { symbols: watchlist } = useWatchlistStore()

  const { data: statusData } = useQuery({
    queryKey: ['status'],
    queryFn: () => systemApi.status().then((r) => r.data),
    refetchInterval: 5000,
    enabled: !!user,
  })

  const { data: positions } = useQuery({
    queryKey: ['positions'],
    queryFn: () => portfolioApi.positions().then((r) => r.data),
    refetchInterval: 10000,
    enabled: !!user,
  })

  const activeAssets: string[] = statusData?.symbols
    ? Object.entries(statusData.symbols)
        .filter(([, v]: [string, unknown]) => (v as { lifecycle?: string })?.lifecycle === 'active')
        .map(([k]) => k)
    : []

  const positionList = Array.isArray(positions) ? positions : (positions?.positions ?? [])

  return (
    <aside className="flex h-screen w-56 flex-col border-r border-border bg-surface overflow-y-auto">
      {/* Brand */}
      <div className="flex items-center gap-2 px-4 py-4 border-b border-border">
        <div className="flex h-7 w-7 items-center justify-center rounded bg-blue-500 text-white text-xs font-bold">TB</div>
        <span className="font-mono text-sm font-semibold tracking-tight text-text">TradingBot</span>
      </div>

      {/* Primary nav */}
      <nav className="flex flex-col gap-0.5 p-2 border-b border-border">
        {NAV.map(({ to, label, icon: Icon }) => (
          <Link
            key={to}
            to={to}
            className={cn(
              'flex items-center gap-2.5 rounded-md px-3 py-2 text-sm transition-colors',
              location.pathname === to || location.pathname.startsWith(to + '/')
                ? 'bg-blue-500/10 text-blue-400'
                : 'text-text-muted hover:bg-border hover:text-text'
            )}
          >
            <Icon className="h-4 w-4 shrink-0" />
            {label}
          </Link>
        ))}
      </nav>

      {/* Dynamic sections */}
      <div className="flex flex-col gap-3 p-2 flex-1">
        {/* Watchlist */}
        {watchlist.length > 0 && (
          <CollapseSection label="Watchlist">
            {watchlist.map((sym) => (
              <button
                key={sym}
                onClick={() => navigate(`/asset/${sym}`)}
                className={cn(
                  'flex w-full items-center gap-2 rounded-md px-3 py-1.5 text-sm transition-colors',
                  location.pathname === `/asset/${sym}`
                    ? 'bg-blue-500/10 text-blue-400'
                    : 'text-text-muted hover:bg-border hover:text-text'
                )}
              >
                <Star className="h-3 w-3 text-amber-400" />
                {sym}
              </button>
            ))}
          </CollapseSection>
        )}

        {/* Active assets */}
        {activeAssets.length > 0 && (
          <CollapseSection label="Active">
            {activeAssets.map((sym) => (
              <button
                key={sym}
                onClick={() => navigate(`/asset/${sym}`)}
                className={cn(
                  'flex w-full items-center gap-2 rounded-md px-3 py-1.5 text-sm transition-colors',
                  location.pathname === `/asset/${sym}`
                    ? 'bg-blue-500/10 text-blue-400'
                    : 'text-text-muted hover:bg-border hover:text-text'
                )}
              >
                <Zap className="h-3 w-3 text-emerald-400" />
                {sym}
              </button>
            ))}
          </CollapseSection>
        )}

        {/* Holdings */}
        {positionList.length > 0 && (
          <CollapseSection label="Holdings">
            {positionList.map((pos: { symbol: string; qty: number; unrealized_pnl?: number }) => (
              <button
                key={pos.symbol}
                onClick={() => navigate(`/asset/${pos.symbol}`)}
                className="flex w-full items-center justify-between rounded-md px-3 py-1.5 text-sm text-text-muted hover:bg-border hover:text-text transition-colors"
              >
                <span className="flex items-center gap-2">
                  <Briefcase className="h-3 w-3" />
                  {pos.symbol}
                </span>
                {pos.unrealized_pnl != null && (
                  <span className={cn('text-xs font-mono', pnlClass(pos.unrealized_pnl))}>
                    {fmtCurrency(pos.unrealized_pnl)}
                  </span>
                )}
              </button>
            ))}
          </CollapseSection>
        )}
      </div>

      {/* Footer */}
      <div className="border-t border-border p-2 space-y-0.5">
        <Link
          to="/account"
          className="flex items-center gap-2.5 rounded-md px-3 py-2 text-sm text-text-muted hover:bg-border hover:text-text transition-colors"
        >
          <Settings className="h-4 w-4" />
          <span className="truncate">{user?.email ?? 'Account'}</span>
        </Link>
        <button
          onClick={logout}
          className="flex w-full items-center gap-2.5 rounded-md px-3 py-2 text-sm text-text-muted hover:bg-border hover:text-red-400 transition-colors"
        >
          <LogOut className="h-4 w-4" />
          Sign out
        </button>
      </div>
    </aside>
  )
}
