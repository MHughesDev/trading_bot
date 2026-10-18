// The deployments page's words (Set T T-1.3). Pure, so each sentence the page
// says about a deployment is one a test can hold.
import type { DeploymentRow, DeploymentProposal } from '@/api/deployments'

/** The ramp's size. `halvings` is 0 (full) to 4 (zero): ADR-T2-01's ladder. */
export function sizeWords(halvings: number): { word: string; tone: 'pos' | 'warn' | 'neg' } {
  if (halvings <= 0) return { word: 'full size', tone: 'pos' }
  if (halvings >= 4) return { word: 'size zero', tone: 'neg' }
  return { word: `1/${2 ** halvings} size`, tone: 'warn' }
}

/** The gate verdict in one line: how many gates passed, and which ones did not. */
export function verdictWords(
  row: Pick<DeploymentRow, 'verdict' | 'gates_passed' | 'proposal_id'>,
): { headline: string; tone: 'pos' | 'neg' | 'warn' | 'neutral'; failed: number[] } {
  if (row.proposal_id === null || row.gates_passed === null) {
    return { headline: 'no gate verdict', tone: 'neutral', failed: [] }
  }
  const failed = (row.verdict ?? [])
    .filter((g) => g.passed === false && typeof g.gate_no === 'number')
    .map((g) => g.gate_no as number)
  const total = row.verdict?.length ?? null
  const headline =
    total === null ? `${row.gates_passed} gates passed` : `${row.gates_passed} of ${total} gates passed`
  return { headline, tone: failed.length > 0 ? 'warn' : 'pos', failed }
}

/**
 * The review period as a sentence. A period that has ended says so: a countdown
 * that went negative would read as "in 0 days" and hide a review that is due.
 */
export function periodWords(
  row: Pick<DeploymentRow, 'review_period_no' | 'review_period_ends_at'>,
  now = Date.now(),
): string {
  if (row.review_period_ends_at === null) return 'no review period recorded'
  const ends = Date.parse(row.review_period_ends_at)
  if (Number.isNaN(ends)) return 'review period end unreadable'
  const days = (ends - now) / 86_400_000
  const no = row.review_period_no === null ? '' : `period ${row.review_period_no}: `
  if (days <= 0) return `${no}ended ${Math.max(1, Math.round(-days))} d ago, review due`
  if (days < 1) return `${no}ends in under a day`
  return `${no}ends in ${Math.ceil(days)} d`
}

/**
 * The divergence state. `null` halts means no forward recorder feeds the
 * deployment, which is "not measured", never "no divergence".
 */
export function divergenceWords(
  row: Pick<DeploymentRow, 'divergence_halts' | 'realised_vs_expected' | 'unmeasured'>,
): { word: string; tone: 'pos' | 'neg' | 'warn' | 'neutral'; detail: string } {
  if (row.divergence_halts === null || row.unmeasured) {
    return {
      word: 'not measured',
      tone: 'warn',
      detail:
        'No forward recorder feeds this deployment, so its divergence from the backtest is unknown, not nil.',
    }
  }
  const n = row.realised_vs_expected.length
  const mean =
    n === 0 ? null : row.realised_vs_expected.reduce((a, p) => a + (p.realised - p.expected), 0) / n
  const series =
    mean === null
      ? 'no observed bars yet'
      : `${n} observed bars, mean realised minus expected ${(mean * 10_000).toFixed(1)} bps`
  if (row.divergence_halts > 0) {
    return {
      word: `${row.divergence_halts} ${row.divergence_halts === 1 ? 'halt' : 'halts'}`,
      tone: 'neg',
      detail: series,
    }
  }
  return { word: 'no halts', tone: 'pos', detail: series }
}

/** What the person may do to a proposal. */
export function proposalAction(
  p: Pick<DeploymentProposal, 'decision' | 'deployment_id'>,
): 'decide' | 'deploy' | 'none' {
  if (p.decision === 'pending') return 'decide'
  if (p.decision === 'approved' && p.deployment_id === null) return 'deploy'
  return 'none'
}

export function shortRevision(rev: string): string {
  return rev.length > 10 ? rev.slice(0, 10) : rev
}
