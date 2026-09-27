import type { Proposal } from '@/api/desk'

/**
 * What a ring means, in the words of the thing it protects.
 *
 * A number alone makes the two decisions look like degrees of the same thing.
 */
export function ringLabel(ring: number): string {
  if (ring >= 2) return 'ring 2 · reaches paper trading'
  if (ring === 1) return "ring 1 · writes the platform's own artifacts"
  return 'ring 0 · research only'
}

/**
 * The line shown under an approve button.
 *
 * Returns `null` when approving would perform the write, because a card that
 * explained itself in the ordinary case would bury the case that matters.
 */
export function inertNotice(p: Proposal): string | null {
  if (p.actionable) return null
  if (!p.subject_id) {
    return 'This proposal names no target row — it predates the column — so approving it records the decision and changes nothing.'
  }
  return 'The platform has no approval-side writer for this kind yet, so approving it records the decision and changes nothing.'
}
