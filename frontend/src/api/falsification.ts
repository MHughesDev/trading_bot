import { api as client } from '@/lib/api'

/**
 * The falsification ledger (EXTERNAL-REVIEW-2026-09 §4; ADR-P6-05, ADR-P6-06).
 *
 * Two kinds of report, and the ordering between them is the design:
 *
 * - `evaluator_defect` — the platform itself is wrong. Scores **above** a
 *   promotion. Without that ordering an agent that discovers the simulator lets
 *   it buy the intrabar low exploits it silently, because reporting it would
 *   destroy its best result.
 * - `kill` — a candidate is refuted. The dense signal: most strategies are
 *   noise, so this fires constantly and is usually right.
 *
 * Every field below exists to keep a claim checkable. `expected` and `observed`
 * must differ — the database refuses a report where they match, which is what
 * makes "the Sharpe was low" unwritable — and `steps` must be non-empty.
 */

export type ReportKind = 'kill' | 'evaluator_defect'
export type Verification = 'unverified' | 'verified' | 'refuted'

/**
 * The closed defect vocabulary. There is no entry for underperformance on
 * purpose: a low score is a result, not a defect.
 */
export type DefectClass =
  | 'lookahead'
  | 'data_artifact'
  | 'below_cost_floor'
  | 'below_tradeable_frontier'
  | 'fill_model_exploit'
  | 'duplicate_of_existing'
  | 'invalid_metric'

export interface FalsificationReport {
  falsification_id: string
  report_kind: ReportKind
  /** Present on a kill. */
  target_trial_id: string | null
  /** Present on a defect: which part of the platform is wrong. */
  component: string | null
  /** Present on a defect: what that component claims to guarantee. */
  claimed_guarantee: string | null
  /** Present on a kill. */
  defect_class: DefectClass | null
  artifact_ref: string
  seed: number
  expected: string
  observed: string
  steps: string[]
  verification: Verification
  verified_by: string | null
  /** The test that closed the hole. Verification alone only closes the question. */
  regression_test: string | null
  reported_by: string
  reporter_kind: string
  reported_at: string
  /**
   * Assigned by `mlops.falsification_score()`, not recomputed here. 3 = verified
   * evaluator defect, 2 = promotion, 1 = verified kill, 0 = unverified or
   * refuted. Reading it back is deliberate: if the database and the Rust type
   * ever disagree, a surface that recomputed would hide it.
   */
  score: number
  is_discharged: boolean
}

export interface FalsificationCounts {
  total: number
  /** Verified defects with no regression test. The work queue. */
  undischarged_defects: number
  /** Claims nobody has re-run. Not the same thing as an empty queue. */
  unverified: number
}

export interface FalsificationLedger {
  reports: FalsificationReport[]
  counts: FalsificationCounts
}

/** Human labels. Kept beside the type so a new variant fails the build here. */
export const DEFECT_CLASS_LABEL: Record<DefectClass, string> = {
  lookahead: 'Look-ahead',
  data_artifact: 'Data artifact',
  below_cost_floor: 'Below cost floor',
  below_tradeable_frontier: 'Below tradeable frontier',
  fill_model_exploit: 'Fill-model exploit',
  duplicate_of_existing: 'Duplicate',
  invalid_metric: 'Invalid metric',
}

export const VERIFICATION_LABEL: Record<Verification, string> = {
  unverified: 'Unverified',
  verified: 'Verified',
  refuted: 'Refuted',
}

/**
 * What a report is worth, in words.
 *
 * `scoreLabel` never renders a bare number for an unverified report: zero and
 * "nobody has checked this yet" are the same number and different facts, and
 * the whole mechanism depends on the second one not reading as the first.
 */
export function scoreLabel(r: FalsificationReport): string {
  if (r.verification === 'unverified') return 'Unscored — not yet reproduced'
  if (r.verification === 'refuted') return 'Zero — reproduction failed'
  return r.report_kind === 'evaluator_defect'
    ? 'Evaluator defect — outranks a promotion'
    : 'Kill'
}

export const falsificationApi = {
  list: () => client.get<FalsificationLedger>('/api/falsification'),
}
