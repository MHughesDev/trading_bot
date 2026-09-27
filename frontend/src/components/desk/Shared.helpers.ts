import type { AgentProfile } from '@/api/agent'
import type { BudgetView } from '@/api/desk'
import type { BarValue, BudgetCaps, TierTile } from './Shared'

/**
 * Whether the platform holds a calibration row for this profile.
 *
 * `multi_step_evidence` is the only measurement this platform records today. No
 * evidence means unmeasured; it never means "probably fine".
 *
 * Three surfaces drew this badge and **all three** spelled the rule out
 * themselves as `p.multi_step_evidence !== null || !p.is_local`, so every hosted
 * profile was declared measured on the strength of being hosted. S-6.20 fixed
 * the Models page; desk home and the conversation header kept it, and the badge
 * is the half a person reads at a glance. It lives beside `TierBadge` now
 * because the badge is the only thing that consumes it, and a rule restated per
 * caller is a rule that gets fixed per caller.
 *
 * Whether a model is hosted is an argument about whether it *needs* measuring.
 * It is not evidence that it was.
 */
export function profileIsMeasured(p: Pick<AgentProfile, 'multi_step_evidence'>): boolean {
  return p.multi_step_evidence !== null
}

/**
 * The profile tiles desk home and the conversation header both render.
 *
 * `undefined` in means the profile list has not answered, and the answer is an
 * empty list rather than an invented row — the callers own the difference
 * between "no profile is configured" and "nobody could read the list", which are
 * opposite claims about a platform that cannot run anything without one.
 */
export function tierTiles(profiles: AgentProfile[] | undefined): TierTile[] {
  return (profiles ?? []).map((p) => ({
    id: p.model_id,
    label: p.model_id,
    tier: p.tier,
    measured: profileIsMeasured(p),
    frontier: p.tier === 'frontier',
  }))
}

/**
 * What each of the four bars can honestly say, given a snapshot and whatever
 * caps the caller was told.
 *
 * Exported because the rule is worth a test and the rendering is not.
 *
 * A cap may now arrive on the snapshot itself (`tokens_cap`, ADR-D-25) as well
 * as from the caller, and the caller wins — it is the more specific fact, and
 * `?? ` keeps a caller's `0` rather than discarding it. Everything else stays
 * as it was: `steps_total: 0` reads as "no total, not a cap of zero", and an
 * axis nothing capped renders uncapped rather than borrowing a number that
 * happens to be lying around.
 */
export function budgetBars(budget: BudgetView, caps?: Partial<BudgetCaps>): Array<{ label: string; value: BarValue; money?: boolean }> {
  const stepsCap = caps?.steps ?? (budget.steps_total > 0 ? budget.steps_total : null)
  const tokensCap = caps?.tokens ?? budget.tokens_cap
  const usdCap = caps?.usdTenthCents ?? budget.usd_tenth_cents_cap
  const trialsCap = caps?.trials ?? budget.trials_cap
  return [
    {
      label: 'steps',
      value:
        stepsCap === null
          ? { capped: false, value: budget.steps_remaining, word: 'remaining' }
          : { capped: true, used: Math.max(0, stepsCap - budget.steps_remaining), cap: stepsCap },
    },
    {
      label: 'tokens',
      value: fromCapAndRemaining(tokensCap, budget.tokens_remaining, 'remaining'),
    },
    {
      label: '$',
      money: true,
      value: fromCapAndRemaining(usdCap, budget.usd_tenth_cents_remaining, 'remaining'),
    },
    {
      // Trials are counted upward, so `trials_spent` *is* the burn: it needs no
      // remainder, and it is never absent. Only the cap can be missing here.
      label: 'trials',
      value:
        trialsCap === null
          ? { capped: false, value: budget.trials_spent, word: 'spent' }
          : { capped: true, used: budget.trials_spent, cap: trialsCap },
    },
  ]
}

export function ago(iso: string, now = Date.now()): string {
  const t = Date.parse(iso)
  if (!Number.isFinite(t)) return '—'
  const s = Math.max(0, Math.round((now - t) / 1000))
  if (s < 60) return `${s}s`
  if (s < 3600) return `${Math.round(s / 60)}m`
  if (s < 86400) return `${Math.round(s / 3600)}h`
  return `${Math.round(s / 86400)}d`
}

/**
 * The one place a cap and a remainder become a bar, for the three axes where
 * either half can be absent.
 *
 * `cap` and `remaining` are independent, so this is a two-by-two and not a
 * fallback chain — there is deliberately no `??` between them in either
 * direction, because every historical bug on this component was one half
 * standing in for the other.
 */
export function fromCapAndRemaining(
  cap: number | null,
  remaining: number | null,
  word: 'remaining' | 'spent',
): BarValue {
  if (cap === null) {
    return remaining === null ? { capped: false, value: null } : { capped: false, value: remaining, word }
  }
  if (remaining === null) return { capped: true, used: null, cap }
  return { capped: true, used: Math.max(0, cap - remaining), cap }
}
