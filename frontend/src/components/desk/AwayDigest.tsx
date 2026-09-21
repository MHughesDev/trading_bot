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
import type { AwayDigest as AwayDigestData, DigestLine } from '@/api/desk'
import { ProvenanceBadge } from './Shared'

/**
 * What the digest is blind to, as a sentence.
 *
 * A pure function because the rule is worth a test and rendering it is not. The
 * server names the kinds it has no source for; the panel used to drop the field
 * entirely, so a digest that could not see refutations rendered exactly like one
 * where nothing had been refuted. Returns `null` only when the server said
 * nothing was uncovered — never when it simply was not asked.
 */
export function blindSpotNote(data: AwayDigestData | null): string | null {
  const gaps = data?.kinds_not_covered ?? []
  if (gaps.length === 0) return null
  return `Not covered here: ${gaps.map((g) => `${g.kind} (${g.why})`).join('; ')}.`
}

/**
 * Whether the panel has anything honest to say.
 *
 * An empty line list is **not** a reason to render nothing when the server also
 * declared blind spots: "nothing happened" is a finding about your research and
 * "I cannot see three of the seven kinds" is a fact about the platform, and the
 * operator reading it is deciding whether to stop looking.
 */
export function hasSomethingToSay(data: AwayDigestData | null, linesInWindow: number): boolean {
  if (!data) return false
  if (data.absent === true) return false
  return linesInWindow > 0 || (data.kinds_not_covered ?? []).length > 0
}

const MARKER_KEY = 'desk:last-visit'

/** The gap below which the digest is not shown at all. */
export const MIN_GAP_MS = 60 * 60 * 1000

export function readLastVisit(now = Date.now()): number {
  try {
    const raw = localStorage.getItem(MARKER_KEY)
    const n = raw ? Number(raw) : NaN
    return Number.isFinite(n) ? n : now
  } catch {
    return now
  }
}

export function writeLastVisit(at = Date.now()) {
  try {
    localStorage.setItem(MARKER_KEY, String(at))
  } catch {
    // A browser that refuses storage simply never shows the digest. That is a
    // worse experience, not a wrong one.
  }
}

/** Whether a digest should be shown at all, given the gap. */
export function shouldShowDigest(lastVisit: number, now = Date.now()): boolean {
  return now - lastVisit >= MIN_GAP_MS
}

/**
 * Keeps only the lines inside the gap.
 *
 * "Exactly the gap and no more" is a filter, not a hope: a backend that returns
 * a generous window must not turn the digest into yesterday's news.
 */
export function linesInGap(lines: DigestLine[], since: number, until: number): DigestLine[] {
  return lines.filter((l) => {
    const t = Date.parse(l.at)
    return Number.isFinite(t) && t >= since && t <= until
  })
}

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

  const lines = linesInGap(data.lines, lastVisit, now)
  if (!hasSomethingToSay(data, lines.length)) return null
  const blind = blindSpotNote(data)

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
            reader is deciding whether they have seen everything. */}
        {blind && (
          <p className="sub" data-testid="digest-blind-spots">
            {blind}
          </p>
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
