import { Link, useLocation } from 'react-router-dom'
import { motion, useReducedMotion } from 'framer-motion'
import {
  LayoutDashboard, Monitor, Zap, Layers, Settings,
} from 'lucide-react'
import { cn } from '@/lib/utils'

const SECTIONS = [
  { path: '/dashboard', label: 'Dashboard', icon: LayoutDashboard },
  { path: '/trading', label: 'Trading', icon: Monitor },
  { path: '/automations', label: 'Automations', icon: Zap },
  { path: '/strategy', label: 'Strategy', icon: Layers },
  { path: '/settings', label: 'Settings', icon: Settings },
]

// 380 ms asymmetric easing: fast enter, gentle settle.
const PILL_TRANSITION = {
  type: 'spring' as const,
  stiffness: 380,
  damping: 30,
  mass: 1,
}
const PILL_TRANSITION_INSTANT = {
  duration: 0.001,
}

export function GlassPillNav() {
  const location = useLocation()
  const shouldReduce = useReducedMotion()

  const activeIdx = SECTIONS.findIndex(
    (s) =>
      location.pathname === s.path ||
      location.pathname.startsWith(s.path + '/'),
  )

  return (
    <nav
      aria-label="Main navigation"
      className="flex items-center justify-center gap-0 px-4 py-2 border-b border-border bg-surface/80 backdrop-blur-sm shrink-0"
    >
      <div className="relative flex items-center rounded-full bg-surface-2 p-1 gap-0.5">
        {SECTIONS.map((section, i) => {
          const isActive = i === activeIdx
          const Icon = section.icon
          return (
            <Link
              key={section.path}
              to={section.path}
              className={cn(
                'relative flex items-center gap-1.5 rounded-full px-4 py-1.5 text-sm font-medium z-10 transition-colors',
                isActive ? 'text-text' : 'text-text-muted hover:text-text',
              )}
            >
              {isActive && (
                <motion.div
                  layoutId="glass-pill"
                  className="absolute inset-0 rounded-full bg-surface border border-border shadow-sm"
                  style={{ zIndex: -1 }}
                  transition={shouldReduce ? PILL_TRANSITION_INSTANT : PILL_TRANSITION}
                />
              )}
              <Icon className="h-3.5 w-3.5 shrink-0" />
              {section.label}
            </Link>
          )
        })}
      </div>
    </nav>
  )
}
