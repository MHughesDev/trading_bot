// The one polite live region (AGENT-012 §15; Set S task S-6.22).
//
// Mounted once in `AppLayout`, above the routed outlet, so it exists before any
// announcement is made — a live region inserted at the same time as its text is
// not announced by any screen reader, which is the failure mode that makes this
// a shell component rather than a desk one.
//
// `role="status"` carries an implicit `aria-live="polite"`; both are written out
// because a handful of older assistive stacks honour only one of them.
//
// Messages are appended rather than replacing a single node, and each is dropped
// after a while so the region does not accumulate the whole run. `aria-atomic`
// is **false** on purpose: a new approval arriving while the previous
// announcement is still in the region must read as the new line, not as both.
import { useCallback, useEffect, useState } from 'react'
import { registerAnnouncer } from '@/lib/announce'

/** How long a line stays in the region. Long enough to be read, short enough not to pile up. */
const LINGER_MS = 8_000

export function LiveRegion() {
  const [lines, setLines] = useState<{ id: number; text: string }[]>([])

  const push = useCallback((text: string) => {
    const id = Date.now() + Math.random()
    setLines((prev) => [...prev, { id, text }])
    window.setTimeout(() => setLines((prev) => prev.filter((l) => l.id !== id)), LINGER_MS)
  }, [])

  useEffect(() => {
    registerAnnouncer(push)
  }, [push])

  return (
    <div
      className="sr-only"
      role="status"
      aria-live="polite"
      aria-atomic="false"
      data-testid="live-region"
    >
      {lines.map((l) => (
        <p key={l.id}>{l.text}</p>
      ))}
    </div>
  )
}
