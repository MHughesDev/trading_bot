import { useEffect, useMemo, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { useNavigate } from 'react-router-dom'
import { ArrowRight, Keyboard, Moon, Rows3, Search, Sun } from 'lucide-react'
import { NAV_SECTIONS, SECONDARY_SECTIONS } from './TopBar'
import { useThemeStore } from '@/store/theme'
import { useModeStore } from '@/store/mode'
import { cn } from '@/lib/utils'

/* Spec §5.6 — ⌘K command palette, ⌘/ shortcut sheet.
   Every shortcut in the product MUST be discoverable in the sheet. */

export interface Command {
  id: string
  label: string
  hint?: string
  group: string
  icon?: React.ReactNode
  run: () => void
  keywords?: string
}

export function CommandPalette({ open, onClose }: { open: boolean; onClose: () => void }) {
  const navigate = useNavigate()
  const { theme, toggleTheme, density, setDensity } = useThemeStore()
  const { mode } = useModeStore()
  const [q, setQ] = useState('')
  const [i, setI] = useState(0)
  const listRef = useRef<HTMLDivElement>(null)

  const commands = useMemo<Command[]>(() => {
    const go = (path: string, label: string, icon?: React.ReactNode): Command => ({
      id: `go:${path}`,
      label,
      group: 'Go to',
      hint: path,
      icon: icon ?? <ArrowRight size={13} />,
      run: () => navigate(path),
    })
    return [
      ...NAV_SECTIONS.map((s) => go(s.path, s.label, <s.icon size={13} />)),
      ...SECONDARY_SECTIONS.map((s) => go(s.path, s.label, <s.icon size={13} />)),
      {
        id: 'theme',
        label: theme === 'terminal' ? 'Switch to Paper theme' : 'Switch to Terminal theme',
        group: 'Appearance',
        hint: '⌘\\',
        icon: theme === 'terminal' ? <Sun size={13} /> : <Moon size={13} />,
        run: toggleTheme,
        keywords: 'dark light theme colour color',
      },
      {
        id: 'density',
        label: density === 'compact' ? 'Comfortable density' : 'Compact density',
        group: 'Appearance',
        hint: '⌘.',
        icon: <Rows3 size={13} />,
        run: () => setDensity(density === 'compact' ? 'comfortable' : 'compact'),
        keywords: 'row height spacing dense',
      },
      {
        id: 'shortcuts',
        label: 'Keyboard shortcuts',
        group: 'Help',
        hint: '⌘/',
        icon: <Keyboard size={13} />,
        run: () => window.dispatchEvent(new CustomEvent('meridian:shortcuts')),
        keywords: 'keys hotkeys bindings',
      },
      {
        id: 'neworder',
        label: `New order (${mode === 'LIVE' ? 'live' : 'paper'})`,
        group: 'Trade',
        hint: 'b / s',
        icon: <ArrowRight size={13} />,
        run: () => navigate('/terminal'),
        keywords: 'buy sell ticket trade',
      },
    ]
  }, [navigate, theme, toggleTheme, density, setDensity, mode])

  const filtered = useMemo(() => {
    const t = q.trim().toLowerCase()
    if (!t) return commands
    return commands.filter((c) =>
      `${c.label} ${c.group} ${c.hint ?? ''} ${c.keywords ?? ''}`.toLowerCase().includes(t),
    )
  }, [q, commands])

  useEffect(() => setI(0), [q])
  useEffect(() => {
    if (open) {
      setQ('')
      setI(0)
    }
  }, [open])

  useEffect(() => {
    if (!open) return
    function onKey(e: KeyboardEvent) {
      if (e.key === 'Escape') { e.preventDefault(); onClose() }
      else if (e.key === 'ArrowDown') { e.preventDefault(); setI((x) => Math.min(filtered.length - 1, x + 1)) }
      else if (e.key === 'ArrowUp') { e.preventDefault(); setI((x) => Math.max(0, x - 1)) }
      else if (e.key === 'Enter') {
        e.preventDefault()
        const c = filtered[i]
        if (c) { c.run(); onClose() }
      }
    }
    document.addEventListener('keydown', onKey)
    return () => document.removeEventListener('keydown', onKey)
  }, [open, filtered, i, onClose])

  useEffect(() => {
    listRef.current?.querySelector<HTMLElement>('[data-active="true"]')?.scrollIntoView({ block: 'nearest' })
  }, [i])

  if (!open) return null

  const groups = filtered.reduce<Record<string, Command[]>>((acc, c) => {
    ;(acc[c.group] ??= []).push(c)
    return acc
  }, {})
  let flat = -1

  return createPortal(
    <div
      className="scrim"
      style={{ alignItems: 'flex-start', paddingTop: '12vh' }}
      onMouseDown={(e) => e.target === e.currentTarget && onClose()}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-label="Command palette"
        className="modal"
        style={{ maxWidth: 560 }}
      >
        <div className="modal-hd" style={{ padding: 'var(--s-3) var(--s-4)' }}>
          <Search size={15} className="mut" aria-hidden />
          <input
            autoFocus
            value={q}
            onChange={(e) => setQ(e.target.value)}
            placeholder="Search commands and pages…"
            aria-label="Search commands"
            style={{
              flex: 1,
              background: 'transparent',
              border: 0,
              outline: 'none',
              fontSize: 'var(--t-14)',
              color: 'var(--fg-primary)',
            }}
          />
          <span className="lbl">esc</span>
        </div>
        <div ref={listRef} className="modal-bd" style={{ padding: 6, maxHeight: '52vh' }}>
          {filtered.length === 0 && (
            <div className="empty" style={{ minHeight: 120 }}>
              <span className="msg">
                No commands match <em>{q}</em>
              </span>
            </div>
          )}
          {Object.entries(groups).map(([g, cmds]) => (
            <div key={g}>
              <div className="lbl" style={{ padding: '8px 8px 4px' }}>
                {g}
              </div>
              {cmds.map((c) => {
                flat += 1
                const active = flat === i
                return (
                  <button
                    key={c.id}
                    type="button"
                    data-active={active}
                    className={cn('menu-item', active && 'on')}
                    onMouseEnter={() => setI(filtered.indexOf(c))}
                    onClick={() => { c.run(); onClose() }}
                  >
                    {c.icon}
                    <span className="truncate-1">{c.label}</span>
                    {c.hint && (
                      <span className="lbl mono" style={{ marginLeft: 'auto' }}>
                        {c.hint}
                      </span>
                    )}
                  </button>
                )
              })}
            </div>
          ))}
        </div>
      </div>
    </div>,
    document.body,
  )
}

const SHORTCUTS: { group: string; items: [string, string][] }[] = [
  {
    group: 'Global',
    items: [
      ['⌘K', 'Command palette'],
      ['⌘/', 'This shortcut sheet'],
      ['⌘\\', 'Toggle theme'],
      ['⌘.', 'Toggle density'],
      ['g then d', 'Go to Dashboard'],
      ['g then t', 'Go to Trading'],
      ['g then s', 'Go to Strategy'],
      ['g then a', 'Go to Automations'],
      ['g then b', 'Go to Back Testing'],
    ],
  },
  {
    group: 'Trading terminal',
    items: [
      ['b', 'Focus the buy ticket'],
      ['s', 'Focus the sell ticket'],
      ['Esc', 'Clear the ticket'],
      ['⌘↵', 'Submit the ticket (with confirmation)'],
      ['1 – 6', 'Switch timeframe'],
      ['/', 'Focus watchlist search'],
    ],
  },
  {
    group: 'Strategy canvas',
    items: [
      ['Space + drag', 'Pan the canvas'],
      ['⌘0', 'Fit to view'],
      ['⌘+ / ⌘−', 'Zoom'],
      ['Del', 'Delete selection'],
      ['⌘D', 'Duplicate node'],
      ['Tab', 'Cycle nodes'],
    ],
  },
]

export function ShortcutSheet({ open, onClose }: { open: boolean; onClose: () => void }) {
  useEffect(() => {
    if (!open) return
    function onKey(e: KeyboardEvent) {
      if (e.key === 'Escape') onClose()
    }
    document.addEventListener('keydown', onKey)
    return () => document.removeEventListener('keydown', onKey)
  }, [open, onClose])

  if (!open) return null

  return createPortal(
    <div className="scrim" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div role="dialog" aria-modal="true" aria-label="Keyboard shortcuts" className="modal wide">
        <div className="modal-hd">
          <div className="h2">Keyboard shortcuts</div>
          <span className="spacer" />
          <span className="lbl">esc to close</span>
        </div>
        <div className="modal-bd">
          <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fit,minmax(260px,1fr))', gap: 'var(--s-6)' }}>
            {SHORTCUTS.map((g) => (
              <div key={g.group}>
                <div className="lbl" style={{ marginBottom: 'var(--s-2)' }}>
                  {g.group}
                </div>
                {g.items.map(([k, label]) => (
                  <div key={k} className="kv" style={{ padding: '5px 0' }}>
                    <span style={{ color: 'var(--fg-secondary)' }}>{label}</span>
                    <kbd
                      className="mono"
                      style={{
                        fontSize: 'var(--t-11)',
                        background: 'var(--bg-sunken)',
                        border: '1px solid var(--line-hairline)',
                        borderRadius: 'var(--r-xs)',
                        padding: '2px 6px',
                        color: 'var(--fg-secondary)',
                      }}
                    >
                      {k}
                    </kbd>
                  </div>
                ))}
              </div>
            ))}
          </div>
        </div>
      </div>
    </div>,
    document.body,
  )
}
