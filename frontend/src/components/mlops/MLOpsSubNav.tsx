import { Link, useLocation } from 'react-router-dom'
import { Brain, Database, Workflow, Trophy, GitBranch } from 'lucide-react'
import { cn } from '@/lib/utils'

/**
 * Sub-navigation shared across the consolidated MLOps surface.
 * Each area is a route under /mlops; the active tab is derived from the path.
 * Model-detail pages (/mlops/models/:id) keep "Models" highlighted.
 */
const TABS = [
  { path: '/mlops', label: 'Models', icon: Brain, match: (p: string) => p === '/mlops' || p.startsWith('/mlops/models') || p === '/mlops/create' },
  { path: '/mlops/data', label: 'Data', icon: Database, match: (p: string) => p.startsWith('/mlops/data') },
  { path: '/mlops/automation', label: 'Automation', icon: Workflow, match: (p: string) => p.startsWith('/mlops/automation') },
  { path: '/mlops/leaderboard', label: 'Leaderboard', icon: Trophy, match: (p: string) => p.startsWith('/mlops/leaderboard') },
  { path: '/mlops/lineage', label: 'Lineage', icon: GitBranch, match: (p: string) => p.startsWith('/mlops/lineage') },
]

export function MLOpsSubNav() {
  const { pathname } = useLocation()

  return (
    <div className="flex items-center gap-1 border-b border-border mb-6">
      {TABS.map((tab) => {
        const active = tab.match(pathname)
        const Icon = tab.icon
        return (
          <Link
            key={tab.path}
            to={tab.path}
            className={cn(
              'flex items-center gap-1.5 px-3 py-2.5 text-sm font-medium border-b-2 -mb-px transition-colors',
              active
                ? 'border-accent text-text'
                : 'border-transparent text-text-muted hover:text-text',
            )}
          >
            <Icon className="h-3.5 w-3.5 shrink-0" />
            {tab.label}
          </Link>
        )
      })}
    </div>
  )
}
