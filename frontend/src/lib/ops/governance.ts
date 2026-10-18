// Governance: why something is allowed to run, and how to undo it (FB-RX-004).
//
// Read-only except for one write, the model-alias rollback, which goes through
// the existing `POST /api/models/{id}/aliases/{alias}/rollback` with a confirm
// step. There is no new write path on this page.
import type { DeploymentRecord, GateStack, TimelineRow } from '@/api/ops'

/** One sentence on a gate stack: only a complete, wholly passing one authorises anything. */
export function gateStackWords(g: GateStack): { word: string; tone: 'pos' | 'warn' | 'neg'; sentence: string } {
  if (g.recorded === 0) {
    return {
      word: 'nothing recorded',
      tone: 'warn',
      sentence: `No gate verdict is recorded for ${g.subject}. That is not a pass.`,
    }
  }
  if (g.blocked_at !== null) {
    return {
      word: `blocked at gate ${g.blocked_at}`,
      tone: 'neg',
      sentence: `${g.passed} of ${g.recorded} recorded gates passed; the first failure is gate ${g.blocked_at}.`,
    }
  }
  if (!g.complete) {
    return {
      word: 'incomplete',
      tone: 'warn',
      sentence: `All ${g.recorded} recorded gates passed, but the stack has ${g.of}; the rest did not run, which is not the same as passing them.`,
    }
  }
  return { word: 'complete and passing', tone: 'pos', sentence: `All ${g.of} gates recorded and passed.` }
}

/** What the confirm step says before an alias is moved back. */
export function rollbackConsequence(modelName: string, alias: string, version: number | undefined): string {
  const now = version === undefined ? 'its current version' : `version ${version}`
  return `Moves the "${alias}" alias of ${modelName} off ${now} and back to the version it pointed at before. Anything resolving that alias changes model immediately. The move is recorded.`
}

/** Timeline rows a reviewer reads first: what was stopped. */
export function preventedRows(rows: TimelineRow[]): TimelineRow[] {
  return rows.filter((r) => r.kind === 'audit' && (r.verdict === 'denied' || r.verdict === 'pending_approval'))
}

/** A deployment's standing in one word. */
export function deploymentStanding(d: DeploymentRecord): { word: string; tone: 'pos' | 'neutral' | 'warn' } {
  if (d.stopped_at) return { word: 'stopped', tone: 'neutral' }
  return d.running ? { word: 'running', tone: 'pos' } : { word: 'recorded, not running', tone: 'warn' }
}
