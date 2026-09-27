
/** A line diff, enough to see what changed between two steps' prompts. */
export function diffLines(previous: string, current: string): Array<{ sign: '+' | '-' | ' '; text: string }> {
  const a = previous.split('\n')
  const b = current.split('\n')
  const out: Array<{ sign: '+' | '-' | ' '; text: string }> = []
  const seen = new Set(a)
  const kept = new Set(b)
  for (const line of a) if (!kept.has(line)) out.push({ sign: '-', text: line })
  for (const line of b) out.push({ sign: seen.has(line) ? ' ' : '+', text: line })
  return out
}
