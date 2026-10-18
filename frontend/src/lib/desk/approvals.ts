// One answered approval, everywhere at once (AGENT-012 §1, §3.2; task S-6.15).
//
// The acceptance: "an approval answered inline disappears from the global inbox
// in the same event". Three surfaces list pending approvals under two cache
// keys — the global inbox (`['approvals', …]`), desk home's *Needs you* and its
// decision log (`['desk', 'approvals', …]`) — and an answer from any one of them
// invalidated at most its own. So an approval answered on a card in the
// conversation stayed in the inbox until that list's own 15-second poll.
import type { QueryClient } from '@tanstack/react-query'

/** Every key a pending approval is listed under. */
export const APPROVAL_KEYS = [['approvals'], ['desk', 'approvals']] as const

export function invalidateApprovals(qc: QueryClient): void {
  for (const queryKey of APPROVAL_KEYS) void qc.invalidateQueries({ queryKey: [...queryKey] })
}
