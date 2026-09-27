import type { AwayDigest as AwayDigestData, DigestGap, DigestLine } from '@/api/desk'

/**
 * The kinds this digest is blind to, in the order the server declared them.
 *
 * A pure function because the rule is worth a test and rendering it is not. The
 * server names the kinds it has no source for; the panel used to drop the field
 * entirely, so a digest that could not see refutations rendered exactly like one
 * where nothing had been refuted.
 *
 * This replaces `blindSpotNote`, which joined every reason into one sentence
 * with `; `. That was written when the reasons were short constants. They are
 * not any more — `kinds_not_covered` is computed per request now and quotes what
 * each source said, so the four possible rows concatenate into a paragraph
 * nobody finishes. Returns `[]` only when the server said nothing was uncovered,
 * never when it simply was not asked.
 */
export function blindSpots(data: AwayDigestData | null): DigestGap[] {
  return data?.kinds_not_covered ?? []
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

/**
 * The window to filter against — **the server's, whenever it sent one.**
 *
 * The filter was run against `lastVisit` and the browser's `Date.now()`, and
 * every timestamp it compares them to was stamped by the server. Two clocks,
 * and the difference between them is subtracted from the digest.
 *
 * It bites hardest on the one line that matters most. `desk_digest.rs` stamps
 * the budget line at `until = Utc::now()` — the newest timestamp in the whole
 * response, by construction — so a server clock even a second ahead of the
 * browser's put it past `now` and dropped it. The panel then rendered every
 * other line and no spend figure, which reads as a window in which nothing was
 * spent. That is this route's own stated non-negotiable, undone in the client:
 * an absent measurement rendered as a zero.
 *
 * `since` and `until` are the window the server says it covered, so filtering
 * against them cannot subtract anything the server included. The browser's
 * marker is the fallback for a response that carried no window, and it is only
 * ever a fallback.
 */
export function digestWindow(
  data: Pick<AwayDigestData, 'since' | 'until'>,
  lastVisit: number,
  now: number,
): [number, number] {
  const since = Date.parse(data.since ?? '')
  const until = Date.parse(data.until ?? '')
  return [Number.isFinite(since) ? since : lastVisit, Number.isFinite(until) ? until : now]
}
