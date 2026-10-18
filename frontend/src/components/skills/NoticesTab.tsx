// Quarantine and retirement notices (RA-4.3, SK-14): what live sessions are told when a
// skill they may carry changes standing. Newest first; read from `GET /api/skills/notices`.
import { useQuery } from '@tanstack/react-query'
import { Badge } from '@/components/primitives/Badge'
import { Panel, PanelBody, PanelHeader } from '@/components/primitives/Panel'
import { EmptyState } from '@/components/primitives/States'
import { ReadGate } from '@/components/ops/ReadGate'
import { skillsApi } from '@/api/skills'
import { NOTICES_KEY, noticeWords, statusTone } from '@/lib/skills/view'

export function NoticesTab() {
  const q = useQuery({ queryKey: NOTICES_KEY, queryFn: () => skillsApi.notices(), refetchInterval: 30_000 })
  return (
    <Panel>
      <PanelHeader title="Notices" />
      <PanelBody>
        <ReadGate query={q} route="/api/skills/notices" what="the notices">
          {(d) =>
            d.notices.length === 0 ? (
              <EmptyState
                message="No skill has been pulled"
                detail="A quarantine or retirement appears here the moment it happens."
              />
            ) : (
              <ul data-testid="notices">
                {[...d.notices]
                  .sort((a, b) => b.notice_id - a.notice_id)
                  .map((n) => (
                    <li key={n.notice_id}>
                      <Badge tone={statusTone(n.kind)}>{n.kind}</Badge> {noticeWords(n)}
                    </li>
                  ))}
              </ul>
            )
          }
        </ReadGate>
      </PanelBody>
    </Panel>
  )
}
