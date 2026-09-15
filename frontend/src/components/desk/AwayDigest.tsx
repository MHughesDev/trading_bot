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
  onDismiss,
}: {
  data: AwayDigestData | null
  lastVisit: number
  now?: number
  onDismiss?: () => void
}) {
  const [dismissed, setDismissed] = useState(false)
  useEffect(() => setDismissed(false), [data])

  if (dismissed || !data || !shouldShowDigest(lastVisit, now)) return null
  const lines = linesInGap(data.lines, lastVisit, now)
  if (lines.length === 0) return null

  return (
    <Panel>
      <PanelHeader
        title="Since you were away"
        actions={<span className="sub">{new Date(lastVisit).toLocaleString()} → now</span>}
      />
      <PanelBody>
        <ul style={{ listStyle: 'none', margin: 0, padding: 0, display: 'flex', flexDirection: 'column', gap: 'var(--s-2)' }}>
          {lines.map((l) => (
            <li key={l.id} style={{ fontSize: 'var(--t-12)' }}>
              <ProvenanceBadge mark={l.mark} />{' '}
              {l.href ? <Link to={l.href}>{l.text}</Link> : <span>{l.text}</span>}
            </li>
          ))}
        </ul>
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
