// The alert inbox (FB-RX-005).
//
// One list of things that need an operator: the monitor's breaches, halts and
// trips, and price alerts that fired. Acknowledge and dismiss are recorded on
// the server (per user), so they follow the person between devices. A kill
// switch trip is critical and has no dismiss control while the switch is still
// tripped for its scope; the server refuses it too, and the sentence on the row
// says why.
//
// The agent approvals queue is not duplicated here: it lives at /desk/approvals
// and answers a running agent's pause.
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Link } from 'react-router-dom'
import { Badge } from '@/components/primitives/Badge'
import { Button } from '@/components/primitives/Button'
import { Panel, PanelBody, PanelHeader } from '@/components/primitives/Panel'
import { EmptyState } from '@/components/primitives/States'
import { ReadGate } from '@/components/ops/ReadGate'
import { alertsInboxApi, refusalWords, type InboxItem } from '@/api/ops'
import { dateTime } from '@/lib/format'
import { INBOX_KEY, dismissControl, inboxOrder, severityView } from '@/lib/ops/alerts'

type Action = 'acknowledge' | 'dismiss'

function Row({
  item,
  onAct,
  error,
}: {
  item: InboxItem
  onAct: (a: Action) => void
  error: string | null
}) {
  const sev = severityView(item.severity)
  const dismiss = dismissControl(item)
  return (
    <tr data-testid={`alert-${item.source}-${item.id}`}>
      <td style={{ textAlign: 'left' }}>
        <Badge tone={sev.tone}>{sev.word}</Badge>{' '}
        {item.state === 'unread' ? <Badge tone="accent">unread</Badge> : <Badge tone="outline">{item.state}</Badge>}
      </td>
      <td>{dateTime(item.at)}</td>
      <td style={{ textAlign: 'left' }}>
        <div>{item.title}</div>
        {item.detail && <div className="sub">{item.detail}</div>}
        {!dismiss.enabled && (
          <div className="sub" data-testid="alert-pinned">
            Cannot be dismissed: {dismiss.why}
          </div>
        )}
        {error && (
          <div className="sub" role="alert">
            Refused: {error}
          </div>
        )}
      </td>
      <td>{item.source === 'monitor' ? 'monitor' : item.source === 'calibration' ? 'model calibration' : 'price alert'}</td>
      <td>
        <Button size="xs" variant="ghost" disabled={item.state !== 'unread'} onClick={() => onAct('acknowledge')}>
          Acknowledge
        </Button>
        <Button size="xs" variant="ghost" disabled={!dismiss.enabled} onClick={() => onAct('dismiss')}>
          Dismiss
        </Button>
      </td>
    </tr>
  )
}

export function AlertsPage() {
  const qc = useQueryClient()
  const q = useQuery({
    queryKey: INBOX_KEY,
    queryFn: () => alertsInboxApi.inbox(),
    refetchInterval: 30_000,
  })
  const act = useMutation({
    mutationFn: async ({ item, action }: { item: InboxItem; action: Action }) =>
      action === 'acknowledge'
        ? alertsInboxApi.acknowledge(item.source, item.id)
        : alertsInboxApi.dismiss(item.source, item.id),
    // Re-read either way: the server decides what an item now is, and a list
    // patched locally would show a dismiss that a refusal never made.
    onSettled: () => void qc.invalidateQueries({ queryKey: INBOX_KEY }),
  })
  const failed = act.isError ? { id: act.variables?.item.id, words: refusalWords(act.error) } : null

  return (
    <>
      <div className="pagehead">
        <h1 className="h1">Alerts</h1>
        {q.data?.state === 'ok' && <Badge tone="neutral">{q.data.data.unread} unread</Badge>}
        <div className="spacer" />
        <Link to="/desk/approvals" className="sub">
          Agent approvals live on the desk
        </Link>
      </div>
      <div className="page-body">
        <Panel>
          <PanelHeader title="Inbox" />
          <PanelBody>
            <ReadGate query={q} route="/api/alert-inbox" what="the inbox">
              {(d) =>
                d.items.length === 0 ? (
                  <EmptyState
                    message="Nothing needs you"
                    detail="No breach, halt, trip or fired price alert in the last 30 days that you have not dismissed."
                  />
                ) : (
                  <table className="tbl" data-testid="alerts-table">
                    <thead>
                      <tr>
                        <th style={{ textAlign: 'left' }}>severity</th>
                        <th>when</th>
                        <th style={{ textAlign: 'left' }}>what</th>
                        <th>source</th>
                        <th />
                      </tr>
                    </thead>
                    <tbody>
                      {inboxOrder(d.items).map((item) => (
                        <Row
                          key={`${item.source}/${item.id}`}
                          item={item}
                          error={failed && failed.id === item.id ? failed.words : null}
                          onAct={(action) => act.mutate({ item, action })}
                        />
                      ))}
                    </tbody>
                  </table>
                )
              }
            </ReadGate>
          </PanelBody>
        </Panel>
      </div>
    </>
  )
}
