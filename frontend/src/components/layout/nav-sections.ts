import { Activity, Bell, Bot, Database, FlaskConical, GitCompareArrows, Inbox, LayoutDashboard, Layers, Monitor, Receipt, Scale, Settings as SettingsIcon, Zap, Brain, Microscope } from 'lucide-react'

export const NAV_SECTIONS = [
  { path: '/dashboard', label: 'Dashboard', icon: LayoutDashboard },
  { path: '/trading', label: 'Trading', icon: Monitor },
  { path: '/strategy', label: 'Strategy', icon: Layers },
  { path: '/automations', label: 'Automations', icon: Zap },
  { path: '/backtesting', label: 'Back Testing', icon: FlaskConical },
  { path: '/mlops', label: 'Models', icon: Brain },
  { path: '/desk', label: 'Desk', icon: Bot },
] as const

export const SECONDARY_SECTIONS = [
  { path: '/markets', label: 'Markets', icon: Layers },
  { path: '/desk/memory', label: 'Research memory', icon: Microscope },
  { path: '/desk/approvals', label: 'Approvals', icon: Inbox },
  { path: '/transactions', label: 'Transactions', icon: Receipt },
  { path: '/alerts', label: 'Alerts', icon: Bell },
  { path: '/reconciliation', label: 'Reconciliation', icon: GitCompareArrows },
  { path: '/deployments', label: 'Deployments', icon: Layers },
  { path: '/mlops/studio', label: 'Model Studio', icon: Brain },
  { path: '/skills', label: 'Skills', icon: Microscope },
  { path: '/data/coverage', label: 'Data coverage', icon: Database },
  { path: '/execution-quality', label: 'Execution quality', icon: Activity },
  { path: '/governance', label: 'Governance', icon: Scale },
  { path: '/settings', label: 'Settings', icon: SettingsIcon },
] as const
