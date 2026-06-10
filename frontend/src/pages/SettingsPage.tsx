// Settings page — Profile, Appearance, Venue Credentials.
// No risk section (C-114).

import { useState } from 'react'
import { useAuthStore } from '@/store/auth'
import { ThemeToggle } from '@/components/ThemeToggle'
import { VenueCredentials } from '@/components/settings/VenueCredentials'
import { cn } from '@/lib/utils'

type SettingsTab = 'profile' | 'appearance' | 'credentials'

const TABS: Array<{ id: SettingsTab; label: string }> = [
  { id: 'profile', label: 'Profile' },
  { id: 'appearance', label: 'Appearance' },
  { id: 'credentials', label: 'Venue Credentials' },
]

function ProfileTab() {
  const { user, logout } = useAuthStore()

  return (
    <div className="space-y-6 max-w-sm">
      <div className="rounded-xl border border-border bg-surface-2 p-4 space-y-3">
        <div>
          <div className="text-xs text-text-dim mb-0.5">Email</div>
          <div className="text-sm text-text">{user?.email ?? '—'}</div>
        </div>
        <div>
          <div className="text-xs text-text-dim mb-0.5">User ID</div>
          <div className="text-xs font-mono text-text-muted">{user?.id ?? '—'}</div>
        </div>
        <div>
          <div className="text-xs text-text-dim mb-0.5">Member since</div>
          <div className="text-sm text-text">
            {user?.created_at ? new Date(user.created_at).toLocaleDateString() : '—'}
          </div>
        </div>
      </div>

      <button
        onClick={logout}
        className="rounded-lg px-4 py-2 text-sm text-red-400 hover:bg-red-400/10 border border-red-400/30 transition-colors"
      >
        Sign out
      </button>
    </div>
  )
}

function AppearanceTab() {
  return (
    <div className="space-y-4 max-w-sm">
      <div className="rounded-xl border border-border bg-surface-2 p-4">
        <div className="flex items-center justify-between">
          <div>
            <div className="text-sm font-medium text-text">Theme</div>
            <div className="text-xs text-text-dim mt-0.5">Switch between light and dark mode</div>
          </div>
          <ThemeToggle />
        </div>
      </div>
    </div>
  )
}

export function SettingsPage() {
  const [tab, setTab] = useState<SettingsTab>('credentials')

  return (
    <div className="flex flex-col gap-6 p-6">
      <div>
        <h1 className="text-xl font-semibold text-text">Settings</h1>
        <p className="text-sm text-text-muted mt-1">
          Manage your account, appearance, and venue connections.
        </p>
      </div>

      {/* Tab selector */}
      <div className="flex gap-1 rounded-lg bg-surface-2 p-1 w-fit border border-border">
        {TABS.map((t) => (
          <button
            key={t.id}
            onClick={() => setTab(t.id)}
            className={cn(
              'rounded-md px-4 py-1.5 text-sm font-medium transition-colors',
              tab === t.id
                ? 'bg-surface border border-border text-text shadow-sm'
                : 'text-text-muted hover:text-text',
            )}
          >
            {t.label}
          </button>
        ))}
      </div>

      {/* Tab content */}
      {tab === 'profile' && <ProfileTab />}
      {tab === 'appearance' && <AppearanceTab />}
      {tab === 'credentials' && (
        <div className="max-w-lg">
          <p className="text-sm text-text-muted mb-4">
            Connect your venue accounts. Credentials are verified before saving and
            never returned in plaintext.
          </p>
          <VenueCredentials />
        </div>
      )}
    </div>
  )
}
