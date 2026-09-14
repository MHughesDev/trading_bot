import { useEffect, useState } from 'react'
import { Link, NavLink, useNavigate } from 'react-router-dom'
import { Bot, Command, ExternalLink, FlaskConical, Inbox, LayoutDashboard, Layers, LogOut, Monitor, Moon, Pin, Receipt, Rows3, Settings as SettingsIcon, Sun, Zap, Brain, Microscope } from 'lucide-react'
import { useAuthStore } from '@/store/auth'
import { useModeStore } from '@/store/mode'
import { useThemeStore } from '@/store/theme'
import { useConnection } from '@/hooks/useConnection'
import { usePortfolioSummary } from '@/hooks/usePortfolioSummary'
import { Money, Pnl, Label } from '@/components/primitives/Num'
import { StatusDot } from '@/components/primitives/Badge'
import { IconButton } from '@/components/primitives/Button'
import { MenuItem, MenuLabel, MenuSeparator, Popover, Tooltip } from '@/components/primitives/Overlay'
import { ConfirmDialog } from '@/components/primitives/Modal'
import { cn } from '@/lib/utils'

/* Spec §3.19 — top bar.
   Left: brand. Centre: nav pills. Right, in this order:
   environment pill → equity → day P&L → connection/latency → account. */

export const NAV_SECTIONS = [
  { path: '/dashboard', label: 'Dashboard', icon: LayoutDashboard },
  { path: '/trading', label: 'Trading', icon: Monitor },
  { path: '/strategy', label: 'Strategy', icon: Layers },
  { path: '/automations', label: 'Automations', icon: Zap },
  { path: '/backtesting', label: 'Back Testing', icon: FlaskConical },
  { path: '/mlops', label: 'Models', icon: Brain },
  { path: '/agent', label: 'Agent', icon: Bot },
] as const

export const SECONDARY_SECTIONS = [
  { path: '/markets', label: 'Markets', icon: Layers },
  { path: '/research', label: 'Research workspace', icon: Microscope },
  { path: '/approvals', label: 'Approvals', icon: Inbox },
  { path: '/transactions', label: 'Transactions', icon: Receipt },
  { path: '/settings', label: 'Settings', icon: SettingsIcon },
] as const

function BrandMark() {
  return (
    <div className="brand-mark" aria-hidden>
      <svg width="14" height="14" viewBox="0 0 16 16" fill="none">
        <path
          d="M2 11.5 6 6l3 3.2L14 3.5"
          stroke="currentColor"
          strokeWidth="1.9"
          strokeLinecap="round"
          strokeLinejoin="round"
        />
      </svg>
    </div>
  )
}

function initials(email: string | undefined): string {
  if (!email) return '—'
  const name = email.split('@')[0]
  const parts = name.split(/[._-]+/).filter(Boolean)
  if (parts.length >= 2) return (parts[0][0] + parts[1][0]).toUpperCase()
  return name.slice(0, 2).toUpperCase()
}

/* Spec §5.4 — environment is a first-class, always-visible state, and the thing
   standing between a user and an accidental live order. */
function EnvironmentPill() {
  const { mode, pinned, setMode } = useModeStore()
  const [confirming, setConfirming] = useState(false)
  const live = mode === 'LIVE'

  return (
    <>
      <Tooltip
        content={
          live
            ? 'Live trading. Orders use real funds.'
            : 'Paper trading. No live funds at risk. Click to switch environment.'
        }
      >
        <button
          type="button"
          className={cn('mode-pill', live && 'live')}
          aria-label={`Environment: ${live ? 'Live' : 'Paper'}. Switch environment.`}
          onClick={() => (live ? setMode('PAPER') : setConfirming(true))}
        >
          <StatusDot state={live ? 'live' : 'neutral'} />
          {live ? 'Live' : 'Paper'}
          {pinned && <Pin size={9} aria-hidden />}
        </button>
      </Tooltip>

      <ConfirmDialog
        open={confirming}
        onCancel={() => setConfirming(false)}
        onConfirm={() => {
          setMode('LIVE')
          setConfirming(false)
        }}
        title="Switch to live trading?"
        confirmLabel="Switch to live"
        typeToConfirm="LIVE"
        consequence={
          <>
            <strong>Orders placed in live mode use real funds and cannot be undone.</strong> The order
            ticket will change colour and every submit button will be prefixed <code>LIVE</code>. This
            confirmation is required again at the start of every session.
          </>
        }
      />
    </>
  )
}

function ConnectionIndicator() {
  const { state, latencyMs } = useConnection()
  const dot = state === 'connected' ? 'live' : state === 'disconnected' ? 'down' : 'degraded'
  const word =
    state === 'connected'
      ? latencyMs !== null
        ? `${latencyMs} ms`
        : 'Live'
      : state === 'reconnecting'
        ? 'Reconnecting'
        : state === 'degraded'
          ? 'Delayed'
          : 'Offline'

  return (
    <Tooltip content={`Market data ${state}${latencyMs !== null ? ` · ${latencyMs} ms round trip` : ''}`}>
      <div
        className="row"
        style={{ gap: 6, paddingLeft: 'var(--s-3)', borderLeft: '1px solid var(--line-hairline)' }}
        role="status"
        aria-live="polite"
      >
        <StatusDot state={dot} pulsing={state === 'reconnecting'} />
        <Label>{word}</Label>
      </div>
    </Tooltip>
  )
}

function AccountMenu() {
  const user = useAuthStore((s) => s.user)
  const logout = useAuthStore((s) => s.logout)
  const { theme, toggleTheme, density, setDensity } = useThemeStore()
  const navigate = useNavigate()

  return (
    <Popover
      ariaLabel="Account menu"
      width={252}
      trigger={({ onClick, ref, 'aria-expanded': expanded }) => (
        <button
          ref={ref}
          type="button"
          className="avatar"
          aria-label="Account menu"
          aria-haspopup="menu"
          aria-expanded={expanded}
          onClick={onClick}
        >
          {initials(user?.email)}
        </button>
      )}
    >
      {(close) => (
        <>
          <div style={{ padding: '6px 8px 8px' }}>
            <div style={{ fontSize: 'var(--t-12)', fontWeight: 'var(--w-semibold)' }} className="truncate-1">
              {user?.email ?? 'Signed out'}
            </div>
            <div className="lbl" style={{ marginTop: 2 }}>
              All accounts
            </div>
          </div>
          <MenuSeparator />
          <MenuLabel>Appearance</MenuLabel>
          <MenuItem
            icon={theme === 'terminal' ? <Sun size={13} /> : <Moon size={13} />}
            onClick={() => toggleTheme()}
          >
            {theme === 'terminal' ? 'Switch to Paper' : 'Switch to Terminal'}
            <span className="lbl" style={{ marginLeft: 'auto' }}>
              {'⌘\\'}
            </span>
          </MenuItem>
          <MenuItem
            icon={<Rows3 size={13} />}
            onClick={() => setDensity(density === 'compact' ? 'comfortable' : 'compact')}
          >
            {density === 'compact' ? 'Comfortable density' : 'Compact density'}
            <span className="lbl" style={{ marginLeft: 'auto' }}>
              {'⌘.'}
            </span>
          </MenuItem>
          <MenuSeparator />
          <MenuLabel>Go to</MenuLabel>
          {SECONDARY_SECTIONS.map((s) => (
            <MenuItem
              key={s.path}
              icon={<s.icon size={13} />}
              onClick={() => {
                navigate(s.path)
                close()
              }}
            >
              {s.label}
            </MenuItem>
          ))}
          <MenuSeparator />
          <MenuItem icon={<LogOut size={13} />} danger onClick={() => void logout()}>
            Sign out
          </MenuItem>
        </>
      )}
    </Popover>
  )
}

export function TopBar({ onOpenPalette }: { onOpenPalette: () => void }) {
  const { equity, dayPnl, currency, loading } = usePortfolioSummary()
  const { mode } = useModeStore()

  return (
    <header className="topbar">
      <Link to="/dashboard" className="brand" style={{ textDecoration: 'none', color: 'inherit' }}>
        <BrandMark />
        <span className="brand-name">Meridian</span>
      </Link>

      <nav className="nav" aria-label="Primary">
        {NAV_SECTIONS.map((s) => (
          <NavLink key={s.path} to={s.path} className={({ isActive }) => (isActive ? 'on' : undefined)}>
            {s.label}
          </NavLink>
        ))}
      </nav>

      <div className="spacer" />

      <div className="topright">
        <Tooltip content="Command palette · ⌘K">
          <IconButton label="Open command palette" onClick={onOpenPalette}>
            <Command size={14} aria-hidden />
          </IconButton>
        </Tooltip>

        <EnvironmentPill />

        <div className="equity-readout">
          <Label>Equity</Label>
          {loading ? (
            <span className="skel" style={{ width: 76, height: 13 }} />
          ) : (
            <Money value={equity} ccy={currency} className="v" />
          )}
        </div>

        <div className="equity-readout">
          <Label>{mode === 'LIVE' ? 'Day P&L' : 'Open P&L'}</Label>
          {loading ? (
            <span className="skel" style={{ width: 68, height: 13 }} />
          ) : (
            <Pnl value={dayPnl} ccy={currency} className="v" />
          )}
        </div>

        <ConnectionIndicator />
        <AccountMenu />
      </div>
    </header>
  )
}

/** Announces the environment on every route change for screen-reader users. */
export function useEnvironmentAnnouncement() {
  const { mode } = useModeStore()
  useEffect(() => {
    const el = document.getElementById('env-live-region')
    if (el) el.textContent = `Environment: ${mode === 'LIVE' ? 'Live trading' : 'Paper trading'}`
  }, [mode])
}

export { ExternalLink }
