// Reconciliation: divergence halts and the fleet verdict (Set T T-0.3, T-2.2).
//
// The halt is the deliverable and the clear is the dangerous half, so the one
// piece of logic here that matters is *when the clear button may be pressed*:
// the strategy id typed out exactly (byte for byte — the server compares the
// same way and answers 400 otherwise) and a note a later reader can act on.
// Nothing here relaxes either; a case-folded or trimmed match would let a
// near-miss clear the wrong strategy's halt.
import type { DivergenceHalt, FleetReport, FleetVerdict } from '@/api/ops'

export interface ClearReadiness {
  ready: boolean
  /** The field-adjacent reason it is not ready; `null` when it is. */
  why: string | null
}

export function clearReadiness(halt: DivergenceHalt, typed: string, note: string): ClearReadiness {
  if (typed !== halt.strategy_id) {
    return {
      ready: false,
      why: typed === '' ? 'Type the strategy id to confirm.' : 'That is not the strategy id, character for character.',
    }
  }
  if (note.trim().length === 0) {
    return { ready: false, why: 'Say why it is safe to resume; the note is recorded with your name.' }
  }
  return { ready: true, why: null }
}

/** What a halt's two flags mean together. */
export function haltStanding(h: DivergenceHalt): { word: string; tone: 'neg' | 'warn'; meaning: string } {
  if (h.in_force && h.persisted) {
    return { word: 'in force', tone: 'neg', meaning: 'Orders for this strategy are refused now, and a restart will not forget it.' }
  }
  if (h.in_force) {
    return {
      word: 'in force, not persisted',
      tone: 'warn',
      meaning: 'Refused now, but only in memory: a restart would lose it until it is re-imposed.',
    }
  }
  return {
    word: 'persisted, not in force',
    tone: 'warn',
    meaning: 'Recorded durably but the order gate is not holding it yet; the next rehydration re-imposes it.',
  }
}

export interface FleetView {
  headline: string
  tone: 'pos' | 'warn' | 'neg' | 'neutral'
  /** One sentence the page prints under the headline. */
  sentence: string
}

const VERDICT_WORD: Record<FleetVerdict, string> = {
  not_computable: 'Not computable',
  no_divergence: 'No divergence',
  idiosyncratic: 'Idiosyncratic divergence',
  common_mode: 'Common-mode divergence',
}

/**
 * The fleet verdict in words.
 *
 * `not_computable` is rendered as that and never as "no divergence": the
 * comfortable reading of a fleet nobody could measure is the one this
 * dashboard exists to refuse. `detail` is read defensively because its shape
 * depends on the verdict and the server is the only author of it.
 */
export function fleetView(r: FleetReport): FleetView {
  const d = (r.detail ?? {}) as Record<string, unknown>
  const n = (k: string): number | null => (typeof d[k] === 'number' ? (d[k] as number) : null)
  switch (r.verdict) {
    case 'not_computable':
      return {
        headline: VERDICT_WORD.not_computable,
        tone: 'warn',
        sentence:
          typeof r.detail === 'string'
            ? `${r.detail} Nothing can be said about whether the simulator matches the fleet.`
            : 'Nothing can be said about whether the simulator matches the fleet.',
      }
    case 'no_divergence':
      return {
        headline: VERDICT_WORD.no_divergence,
        tone: 'pos',
        sentence: `${n('measurable') ?? 'An unreported number of'} deployments measured over ${n('bars') ?? 'an unreported number of'} bars sit inside the tolerance.`,
      }
    case 'idiosyncratic':
      return {
        headline: VERDICT_WORD.idiosyncratic,
        tone: 'warn',
        sentence: `${n('diverging') ?? 'Some'} of ${n('measurable') ?? 'the measured'} deployments diverge, not enough in one direction to be a finding about the simulator.`,
      }
    case 'common_mode':
      return {
        headline: VERDICT_WORD.common_mode,
        tone: 'neg',
        sentence:
          typeof d.observed === 'string'
            ? d.observed
            : 'The fleet diverges the same way: one finding about the simulator, not several strategy failures.',
      }
  }
}

/** A divergence per bar, as basis points; `null` stays `not measured`. */
export function bpsWords(v: number | null | undefined, dp = 2): string {
  if (v === null || v === undefined || !Number.isFinite(v)) return 'not measured'
  return `${(v * 10_000).toFixed(dp)} bps`
}
