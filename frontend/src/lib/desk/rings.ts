// What a mode may spend and reach — the UI half of AGENT-013 §4 / ADR-0037 §1.
//
// The enforcement that matters lives below the model: a tool outside the ring is
// filtered out of exposure before routing, so it is unnameable rather than
// refused. This module is the *surface's* half of the same rule, and it exists
// because the desk itself is a door: the Tools panel pins tools for the next
// step, the catalogue offers "use this", and the composer picks a mode. If the
// desk offered a trial-spending tool inside a Chat turn, the ring would be
// enforced everywhere except the one place a human clicks.
//
// So: one predicate, used by every tool surface in the desk, tested adversarially.
import type { Mode, Ring, ToolCard } from '@/api/desk'

/** Rings, innermost first. `research` is ring 0; `paper` is ring 2. */
export const RING_INDEX: Record<Ring, number> = {
  research: 0,
  platform_artifacts: 1,
  paper: 2,
}

/**
 * The outermost ring a mode may *see*.
 *
 * Chat is ring 0 and nothing else. Research is ring 0 too — rings 1 and 2 are
 * reached by a proposal a human approves (AGENT-013 §7.5), never by a tool the
 * model may name. That both modes stop at ring 0 is the point: the difference
 * between them is trials, not reach.
 */
export function maxRingFor(mode: Mode): Ring {
  return mode === 'chat' ? 'research' : 'research'
}

/** Whether a turn in this mode may spend a trial at all. Chat: never. */
export function modeMaySpendTrials(mode: Mode): boolean {
  return mode === 'research'
}

/** Whether this mode may see this tool. */
export function modeMaySeeTool(mode: Mode, tool: ToolCard): boolean {
  if (tool.spends_trial && !modeMaySpendTrials(mode)) return false
  return RING_INDEX[tool.ring] <= RING_INDEX[maxRingFor(mode)]
}

/**
 * The tools a surface may offer in this mode.
 *
 * Every tool list in the desk goes through here — the Tools panel, the catalogue's
 * pin action, the composer's attach menu. A caller that filters by hand is a
 * caller that will forget.
 */
export function toolsVisibleInMode(tools: ToolCard[], mode: Mode): ToolCard[] {
  return tools.filter((t) => modeMaySeeTool(mode, t))
}

/** Why a tool is hidden, for the surface that wants to say so honestly. */
export function whyHidden(mode: Mode, tool: ToolCard): string | null {
  if (modeMaySeeTool(mode, tool)) return null
  if (tool.spends_trial && !modeMaySpendTrials(mode)) {
    return 'spends a trial; this mode spends none'
  }
  return `reaches ring ${RING_INDEX[tool.ring]}; this mode stops at ring ${RING_INDEX[maxRingFor(mode)]}`
}
