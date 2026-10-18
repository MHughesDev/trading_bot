// The alert inbox's wording and rules (FB-RX-005).
//
// Severity is *received*, not decided: the server's `monitor_severity` is the
// one mapping, and this file only gives each word a label and a tone. An
// unrecognised word is shown as itself, never quietly downgraded to "info".
import type { AlertSeverity, InboxItem } from '@/api/ops'

/** The query key every inbox reader and writer shares. */
export const INBOX_KEY = ['alerts', 'inbox'] as const

export interface SeverityView {
  word: string
  tone: 'neg' | 'warn' | 'neutral'
}

export function severityView(s: AlertSeverity | string): SeverityView {
  switch (s) {
    case 'critical':
      return { word: 'critical', tone: 'neg' }
    case 'warning':
      return { word: 'warning', tone: 'warn' }
    case 'info':
      return { word: 'info', tone: 'neutral' }
    default:
      return { word: String(s), tone: 'warn' }
  }
}

export interface DismissControl {
  enabled: boolean
  /** Printed beside the disabled control: why this cannot be dismissed. */
  why: string | null
}

/**
 * Whether the dismiss control is live. The server refuses the dismiss of a trip
 * that is still in force (409 `pinned`); this mirrors it so the control is not
 * a button that can only fail, and the server remains the authority.
 */
export function dismissControl(item: InboxItem): DismissControl {
  if (item.dismissible) return { enabled: true, why: null }
  return { enabled: false, why: item.pinned_reason ?? 'This item cannot be dismissed yet.' }
}

const RANK: Record<string, number> = { critical: 0, warning: 1, info: 2 }

/** Unread first, then by severity, then newest: what to look at, in order. */
export function inboxOrder(items: InboxItem[]): InboxItem[] {
  return [...items].sort(
    (a, b) =>
      Number(b.state === 'unread') - Number(a.state === 'unread') ||
      (RANK[a.severity] ?? 1) - (RANK[b.severity] ?? 1) ||
      (a.at < b.at ? 1 : a.at > b.at ? -1 : 0),
  )
}

/** The shell badge: nothing at zero, `99+` beyond it. */
export function unreadLabel(n: number | null | undefined): string | null {
  if (n === null || n === undefined || n <= 0) return null
  return n > 99 ? '99+' : String(n)
}

/** Whether any unread item is critical: the bell says so in a word, not a colour. */
export function hasUnreadCritical(items: InboxItem[]): boolean {
  return items.some((i) => i.state === 'unread' && i.severity === 'critical')
}
