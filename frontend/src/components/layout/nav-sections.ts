import { Bot, FlaskConical, Inbox, LayoutDashboard, Layers, Monitor, Receipt, Settings as SettingsIcon, Zap, Brain, Microscope } from 'lucide-react'

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
  { path: '/settings', label: 'Settings', icon: SettingsIcon },
] as const
