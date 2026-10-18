// How the studio reads a usage-rule verdict (N-3.2, rendered by N-5.4/5.7/5.8).
//
// The rule the server enforces and this file must not soften (ADR-N-05): an
// unmeasured thing is not a passed thing. A rule that applies and is `fail` OR
// `not_computable` blocks. Only `pass` and `not_applicable` do not.
import type { PromotionProposal, RuleStatus, UsageRule, Verdict } from '@/api/modelsStudio'
import type { BadgeTone } from '@/components/primitives/Badge'

export const RULE_TITLES: Record<UsageRule['rule'], string> = {
  return_forecast: 'Return forecast: out-of-sample R-squared above zero, Clark-West significant',
  vol_forecast: 'Volatility forecast: beats HARQ on QLIKE',
  tail_risk: 'Tail risk (VaR / ES): passes an ES backtest',
}

export function ruleBlocks(rule: UsageRule): boolean {
  if (!rule.applies) return false
  return rule.status === 'fail' || rule.status === 'not_computable'
}

export function blockingRules(v: Verdict): UsageRule[] {
  return v.rules.filter(ruleBlocks)
}

export function ruleTone(status: RuleStatus): BadgeTone {
  switch (status) {
    case 'pass':
      return 'pos'
    case 'fail':
      return 'neg'
    case 'not_computable':
      return 'warn'
    default:
      return 'neutral'
  }
}

/** The status word, with the consequence spelled out where there is one. */
export function ruleLabel(rule: UsageRule): string {
  switch (rule.status) {
    case 'pass':
      return 'Pass'
    case 'fail':
      return 'Fail: blocks'
    case 'not_computable':
      return 'Not computable: blocks'
    default:
      return 'Not applicable'
  }
}

/** A score from a free-form record: a finite number prints, anything else is "not reported". */
export function scoreText(v: unknown, digits = 4): string {
  if (typeof v === 'number' && Number.isFinite(v)) return v.toFixed(digits)
  if (typeof v === 'string' && v !== '') return v
  return 'not reported'
}

/** The union of score names across baselines, in first-seen order, so columns line up. */
export function scoreColumns(rows: Array<{ scores: Record<string, unknown> }>): string[] {
  const seen: string[] = []
  for (const r of rows) {
    for (const k of Object.keys(r.scores)) if (!seen.includes(k)) seen.push(k)
  }
  return seen
}

/** The headline sentence of a verdict: never "all clear" unless the server said `allowed`. */
export function verdictHeadline(v: Verdict): { tone: BadgeTone; text: string } {
  if (v.allowed && blockingRules(v).length === 0) {
    return { tone: 'pos', text: 'Allowed to size a position' }
  }
  const n = blockingRules(v).length
  if (n > 0) {
    return { tone: 'neg', text: `Blocked by ${n} rule${n === 1 ? '' : 's'}` }
  }
  // The server said not allowed but no rule names itself: say so, do not show a pass.
  return { tone: 'warn', text: 'Not allowed: the server reported no passing verdict' }
}

export function pendingProposals(list: PromotionProposal[] | undefined): PromotionProposal[] {
  return (list ?? []).filter((p) => p.status === 'pending')
}
