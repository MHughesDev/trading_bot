
/**
 * The last decision anybody made on this desk.
 *
 * `/api/approvals?state=all` returns every approval newest-first with its
 * `answer` and `answered_at`, so the answered ones are the decision log the
 * empty state needs. Meridian §3.22 asks an empty "Needs you" to say what was
 * last decided rather than "No data", and the panel used to pass a hardcoded
 * `null` and then print "No decision has been recorded on this desk yet" — an
 * assertion about the desk's whole history made without looking at anything.
 *
 * Returns `null` only when the rows genuinely contain no answered approval.
 */
export function lastDecision(rows: Array<Record<string, unknown>>): string | null {
  const answered = rows
    .filter((r) => typeof r.answer === 'string' && (r.answer as string).length > 0)
    .sort((a, b) => String(b.answered_at ?? '').localeCompare(String(a.answered_at ?? '')))
  const top = answered[0]
  if (!top) return null
  const who = typeof top.answered_by === 'string' && top.answered_by ? ` by ${top.answered_by}` : ''
  return `${String(top.kind ?? 'an approval')} → ${String(top.answer)}${who}`
}

/**
 * A count the run recorded, or the words for one it did not.
 *
 * Migration 0062 made every telemetry column nullable so a run that crashed
 * before filling them is still a run, and `null` there is *not recorded*. A
 * `?? 0` would say the run used no context, spent no trials and took no step —
 * three measurements nobody made.
 */
export function runCount(value: number | null, name: string): string {
  return value === null || value === undefined
    ? `${name} not recorded`
    : `${value.toLocaleString()} ${name}`
}
