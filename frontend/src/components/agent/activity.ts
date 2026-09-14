// Indexing tool results, split out from the renderer.
//
// Its own module because a file that exports both a component and a plain function
// breaks React Fast Refresh — the whole file reloads instead of the component.

import type { AgentMessage } from '@/api/agent'

/** The shape a tool_result message carries. */
interface ToolResultContent {
  name?: unknown
  content?: unknown
  is_error?: unknown
}

function str(v: unknown): string | null {
  return typeof v === 'string' && v.length > 0 ? v : null
}

/**
 * Indexes tool results by the call they answer, so a card can show both.
 *
 * Matched on name because the loop runs one tool at a time — with parallel calls this
 * would need the call id, and the result carries one.
 */
export function indexResults(
  messages: AgentMessage[],
): Map<string, { content: unknown; isError: boolean }> {
  const out = new Map<string, { content: unknown; isError: boolean }>()
  for (const m of messages) {
    if (m.kind !== 'tool_result') continue
    const c = m.content as ToolResultContent
    const name = str(c.name)
    if (!name) continue
    out.set(name, { content: c.content, isError: Boolean(c.is_error) })
  }
  return out
}
