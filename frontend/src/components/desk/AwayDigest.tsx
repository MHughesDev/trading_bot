// "Since you were away" (AGENT-012 §2.2a; task S-6.24).
//
// The operator's actual question, which no other panel answers. A research agent
// that runs for hours without you is only useful if coming back to it takes a
// minute — and a reverse-chronological event list is not that, because it makes
// the human reconstruct a state the platform already knows.
//
// Three rules the code enforces rather than merely intends:
//   1. The digest covers exactly the gap since the last visit and no more.
//   2. Every line is a verdict or a state change, never an event name.
//   3. It is absent for a gap under an hour.
import { useEffect, useState } from 'react'
import { Link } from 'react-router-dom'
import { Button } from '@/components/primitives/Button'
import { Panel, PanelBody, PanelHeader } from '@/components/primitives/Panel'
import type { AwayDigest as AwayDigestData } from '@/api/desk'
import { ProvenanceBadge } from './Shared'
import { blindSpots, hasSomethingToSay, writeLastVisit, shouldShowDigest, linesInGap, digestWindow } from './AwayDigest.helpers'

export function AwayDigestPanel({
  data,
  lastVisit,
  now = Date.now(),
  loading = false,
  onDismiss,
}: {
  data: AwayDigestData | null
  lastVisit: number
  now?: number
  /**
   * True while the request is in flight. Without it a pending query and a
   * failed one are both `null` at this boundary, and the panel would report the
   * first as the second.
   */
  loading?: boolean
  onDismiss?: () => void
}) {
  const [dismissed, setDismissed] = useState(false)
  useEffect(() => setDismissed(false), [data])

  if (dismissed || loading || !shouldShowDigest(lastVisit, now)) return null

  // `optional()` turns a 404 or a 501 into `null`, so a `null` here after a long
  // gap means the endpoint did not answer — which is not the same as a quiet
  // night, and the panel that is meant to save the operator an hour of
  // reconstruction must not be the one that hides its own absence.
  if (!data) {
    return (
      <Panel>
        <PanelHeader title="Since you were away" />
        <PanelBody>
          <p className="sub" data-testid="digest-unavailable">
            The digest could not be read, so this is not a report that nothing happened. Open the
            conversations below rather than trusting this panel's silence.
          </p>
        </PanelBody>
      </Panel>
    )
  }

  const [since, until] = digestWindow(data, lastVisit, now)
  const lines = linesInGap(data.lines, since, until)
  if (!hasSomethingToSay(data, lines.length)) return null
  const gaps = blindSpots(data)

  return (
    <Panel>
      <PanelHeader
        title="Since you were away"
        actions={<span className="sub">{new Date(lastVisit).toLocaleString()} → now</span>}
      />
      <PanelBody>
        {lines.length === 0 ? (
          <p className="sub" data-testid="digest-quiet">
            Nothing the desk can see changed while you were away.
          </p>
        ) : (
          <ul style={{ listStyle: 'none', margin: 0, padding: 0, display: 'flex', flexDirection: 'column', gap: 'var(--s-2)' }}>
            {lines.map((l) => (
              <li key={l.id} style={{ fontSize: 'var(--t-12)' }}>
                <ProvenanceBadge mark={l.mark} />{' '}
                {l.href ? <Link to={l.href}>{l.text}</Link> : <span>{l.text}</span>}
              </li>
            ))}
          </ul>
        )}
        {/* Said out loud, beside the lines rather than behind a disclosure: the
            reader is deciding whether they have seen everything.

            **One row per kind, not one sentence.** `blindSpotNote` joins every
            reason with `; `, and since the route stopped asserting its gaps and
            started measuring them the reasons have become paragraphs — the
            budget one alone is three clauses about what an absent measurement is
            not. Four of them concatenated into a single `.sub` is a wall an
            operator skims past, and the whole purpose of this field is that it
            is the thing they read before deciding to stop looking.

            The `budget` row is why this matters most. `Budget::Unmeasured`
            deliberately emits **no line at all** — `budget_line` returns `None`
            for it — so the gap row is the only trace in the entire response that
            runs were active and nothing recorded what they cost. If that row is
            unreadable, the digest has silently become one that reports a window
            of spending as a window of silence. */}
        {gaps.length > 0 && (
          <div className="sub" data-testid="digest-blind-spots">
            <p style={{ margin: '0 0 var(--s-1)' }}>
              Not covered here — this is what the desk could not see, not what did not happen:
            </p>
            <ul style={{ margin: 0, paddingLeft: 'var(--s-4)' }}>
              {gaps.map((g) => (
                <li key={g.kind} data-testid={`digest-gap-${g.kind}`}>
                  <strong>{g.kind}</strong> — {g.why}
                </li>
              ))}
            </ul>
          </div>
        )}
        <Button
          size="xs"
          variant="ghost"
          onClick={() => {
            setDismissed(true)
            writeLastVisit(now)
            onDismiss?.()
          }}
        >
          Dismiss
        </Button>
      </PanelBody>
    </Panel>
  )
}
