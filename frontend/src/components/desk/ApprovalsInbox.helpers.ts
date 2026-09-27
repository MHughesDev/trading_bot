
/** Option ids, whether the server sent strings or objects. */
export function optionIds(options: unknown): string[] {
  if (!Array.isArray(options)) return []
  return options
    .map((o) =>
      typeof o === 'string' ? o : ((o as { id?: string; label?: string }).id ?? (o as { label?: string }).label ?? ''),
    )
    .filter(Boolean)
}

export function timeLeft(timeoutAt?: string | null, now = Date.now()): string | null {
  if (!timeoutAt) return null
  const ms = new Date(timeoutAt).getTime() - now
  if (ms <= 0) return 'expired'
  const mins = Math.round(ms / 60_000)
  if (mins < 60) return `${mins}m left`
  const hours = Math.round(mins / 60)
  return hours < 48 ? `${hours}h left` : `${Math.round(hours / 24)}d left`
}

/** Evidence handles carried on the payload. */
export function evidenceOf(payload: Record<string, unknown>): string[] {
  const keys = ['dossier_ref', 'evidence', 'artifact', 'eval_results', 'qc_report', 'diff']
  const out: string[] = []
  for (const k of keys) {
    const v = payload[k]
    if (typeof v === 'string') out.push(v)
    else if (Array.isArray(v)) out.push(...v.filter((x): x is string => typeof x === 'string'))
  }
  return out
}
