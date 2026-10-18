// The shell's alert-inbox entry: a bell with the unread count (FB-RX-005).
//
// The count comes from the server (`GET /api/alert-inbox`), which is also where
// acknowledge and dismiss live, so the number is the same on every device. When
// the inbox cannot be read the bell says so in words rather than showing no
// badge: no badge reads as "nothing needs you".
import { useQuery } from '@tanstack/react-query'
import { Link } from 'react-router-dom'
import { Bell } from 'lucide-react'
import { alertsInboxApi } from '@/api/ops'
import { INBOX_KEY } from '@/lib/ops/alerts'
import { hasUnreadCritical, unreadLabel } from '@/lib/ops/alerts'

export function AlertsBell() {
  const q = useQuery({
    queryKey: INBOX_KEY,
    queryFn: () => alertsInboxApi.inbox(),
    refetchInterval: 60_000,
  })
  const r = q.data
  const unread = r?.state === 'ok' ? r.data.unread : null
  const critical = r?.state === 'ok' && hasUnreadCritical(r.data.items)
  const label = unreadLabel(unread)
  const unknown = !q.isPending && r?.state !== 'ok'
  const name =
    unread === null
      ? unknown
        ? 'Alerts: the inbox could not be read'
        : 'Alerts'
      : unread === 0
        ? 'Alerts: nothing unread'
        : `Alerts: ${unread} unread${critical ? ', including a critical one' : ''}`
  return (
    <Link
      to="/alerts"
      className="iconbtn"
      aria-label={name}
      title={name}
      data-testid="alerts-bell"
      style={{ position: 'relative', display: 'inline-flex', alignItems: 'center', gap: 4 }}
    >
      <Bell size={14} aria-hidden />
      {label && (
        <span className={`badge count ${critical ? 'neg' : 'neutral'}`} data-testid="alerts-unread">
          {label}
          {critical && <span className="sr-only"> (critical)</span>}
        </span>
      )}
      {unknown && <span className="badge warn">?</span>}
    </Link>
  )
}
