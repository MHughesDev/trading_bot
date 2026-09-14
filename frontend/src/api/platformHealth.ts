import { api as client } from '@/lib/api'

/**
 * The platform's view of its own judgment (SPEC §16.2, ADR-P5-01).
 *
 * The five states are the point of this type. `not_fitted` and `not_applicable`
 * are not "zero" and not "ok" — they are "the model behind this number does not
 * exist yet" and "this platform does not have that thing". A dashboard that
 * renders either as 0 % is lying about the one layer whose job is to say whether
 * the rest is working, which is exactly the failure §16.2 calls "the layer
 * nobody builds".
 */
export type SignalState =
  | 'ok'
  | 'alarm'
  | 'not_fitted'
  | 'not_applicable'
  | 'unavailable'

export type Severity = 'p1' | 'p2'

export interface Signal {
  /** Stable id. Key on this, never on the title. */
  id: string
  title: string
  state: SignalState
  severity: Severity
  /**
   * `null` in every state except `ok` and `alarm`. Rendering a number when this
   * is null is the bug the backend's type exists to prevent, and the frontend
   * must not undo it by coercing null to 0.
   */
  value: number | null
  threshold: number | null
  /** Always present — in the valueless states it says why there is no value. */
  detail: string
  spec_ref: string
}

export interface HealthReport {
  tenant: string
  generated_at: string
  window_days: number
  signals: Signal[]
}

/** Whether a state carries a number at all. */
export function hasValue(state: SignalState): boolean {
  return state === 'ok' || state === 'alarm'
}

/** What to show where a number would go. Never an empty string, never a zero. */
export function valueLabel(signal: Signal): string {
  if (!hasValue(signal.state) || signal.value === null) {
    switch (signal.state) {
      case 'not_fitted':
        return 'not fitted'
      case 'not_applicable':
        return 'n/a'
      default:
        return 'unavailable'
    }
  }
  const v = signal.value
  // Fractions read as percentages; counts and ratios read as themselves.
  if (Math.abs(v) <= 1 && signal.threshold !== null && Math.abs(signal.threshold) <= 1) {
    return `${(v * 100).toFixed(1)}%`
  }
  return Math.abs(v) >= 1000 ? v.toFixed(0) : v.toFixed(3)
}

export const platformHealthApi = {
  get: () => client.get<HealthReport>('/api/platform/health'),
}
