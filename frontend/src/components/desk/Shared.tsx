// The desk's shared components (AGENT-012 §6; task S-6.13, S-6.23).
//
// Built on `@/components/primitives` and the `desk.css` component layer. Nothing
// here uses a back-compat Tailwind alias, and a test asserts it.
//
// Meridian principle 3 applies throughout: **never colour alone**. Every state
// that is signalled with colour also carries the word or the number.
import type { ReactNode } from 'react'
import { Badge } from '@/components/primitives/Badge'
import { Tooltip } from '@/components/primitives/Overlay'
import type {
  BudgetView,
  Confidence,
  EvidenceClass,
  ProvenanceMark,
  RiskClass,
  Stage,
} from '@/api/desk'
import { STAGE_ORDER } from '@/api/desk'

// ── §6.1 Tier badge ──────────────────────────────────────────────────────────

/**
 * `unmeasured` is not a styling variant, it is the truth about a profile nobody
 * has run a scorecard against. AGENT-009 §5: a profile with no calibration row
 * renders `unmeasured` rather than a default, because a default is a claim.
 */
export function TierBadge({
  tier,
  measured,
  detail,
}: {
  tier: string
  measured: boolean
  detail?: string
}) {
  const badge = (
    <span style={{ display: 'inline-flex', gap: 4, alignItems: 'center' }}>
      <Badge tone={tier === 'frontier' ? 'accent' : 'neutral'}>{tier}</Badge>
      {!measured && <Badge tone="warn">unmeasured</Badge>}
    </span>
  )
  return detail ? <Tooltip content={detail}>{badge}</Tooltip> : badge
}

// ── §6.2 Budget meter ────────────────────────────────────────────────────────

export interface BudgetCaps {
  steps: number
  tokens: number
  usdTenthCents: number
  trials: number
}

function pct(used: number, cap: number): number {
  if (cap <= 0) return 0
  return Math.max(0, Math.min(100, (used / cap) * 100))
}

function tone(p: number): '' | 'warn' | 'neg' {
  if (p >= 90) return 'neg'
  if (p >= 75) return 'warn'
  return ''
}

function BudgetBar({
  label,
  used,
  cap,
  format,
}: {
  label: string
  used: number
  cap: number
  format: (n: number) => string
}) {
  const p = pct(used, cap)
  const t = tone(p)
  return (
    <div className="budget-row">
      <span className="lbl-t">{label}</span>
      <div
        className="budget-track"
        role="meter"
        aria-label={`${label} used`}
        aria-valuenow={Math.round(p)}
        aria-valuemin={0}
        aria-valuemax={100}
      >
        <i className={t} style={{ width: `${p}%` }} />
        {/* The 90% wrap-up line, drawn so the cap is visible before it is hit. */}
        <b style={{ left: '90%' }} aria-hidden />
      </div>
      {/* Always the text value: Meridian principle 3 forbids colour alone. */}
      <span className="val">
        {format(used)} / {format(cap)}
        {t && ` · ${t === 'neg' ? '90%+' : '75%+'}`}
      </span>
    </div>
  )
}

export function BudgetMeter({ budget, caps }: { budget: BudgetView; caps?: Partial<BudgetCaps> }) {
  const stepsCap = caps?.steps ?? budget.steps_total
  const stepsUsed = Math.max(0, stepsCap - budget.steps_remaining)
  const tokensCap = caps?.tokens ?? budget.tokens_remaining
  const tokensUsed = Math.max(0, tokensCap - budget.tokens_remaining)
  const usdCap = caps?.usdTenthCents ?? budget.usd_tenth_cents_remaining
  const usdUsed = Math.max(0, usdCap - budget.usd_tenth_cents_remaining)
  const trialsCap = caps?.trials ?? Math.max(budget.trials_spent, 1)

  const money = (tenthCents: number) => `$${(tenthCents / 1000).toFixed(2)}`
  return (
    <div className="budget-meter">
      <BudgetBar label="steps" used={stepsUsed} cap={stepsCap} format={(n) => String(Math.round(n))} />
      <BudgetBar label="tokens" used={tokensUsed} cap={tokensCap} format={(n) => Math.round(n).toLocaleString()} />
      <BudgetBar label="$" used={usdUsed} cap={usdCap} format={money} />
      <BudgetBar label="trials" used={budget.trials_spent} cap={trialsCap} format={(n) => String(Math.round(n))} />
      <span className="val">
        n_eff {budget.n_eff === null ? '(not computed)' : budget.n_eff.toFixed(1)}
      </span>
    </div>
  )
}

// ── §6.3 Evidence chip ───────────────────────────────────────────────────────

/** Resolves an id to something openable, or returns null if it does not exist. */
export type IdResolver = (id: string) => { label: string; kind: string } | null

/**
 * The UI's honesty check on the agent's honesty check (§6.3).
 *
 * An id that cannot be resolved renders **struck through** with "not found". The
 * failure this prevents is small and fatal: a fabricated id rendered as live
 * monospace text looks exactly like a real one, and a human promotes a strategy
 * on it. Asserted by `EvidenceChip.test.tsx`.
 */
export function EvidenceChip({
  id,
  resolve,
  onOpen,
}: {
  id: string
  resolve?: IdResolver
  onOpen?: (id: string) => void
}) {
  const hit = resolve ? resolve(id) : null
  if (!hit) {
    return (
      <Tooltip content="This id resolves to nothing the platform holds.">
        <span className="ev-chip missing" data-testid={`ev-${id}`} data-resolved="false">
          <s>{id}</s>
          <span className="sr-only"> not found</span>
          <span aria-hidden>· not found</span>
        </span>
      </Tooltip>
    )
  }
  return (
    <button
      type="button"
      className="ev-chip"
      data-testid={`ev-${id}`}
      data-resolved="true"
      onClick={() => onOpen?.(id)}
      title={hit.label}
    >
      {id}
    </button>
  )
}

// ── §6.3a Provenance mark — the fence has to reach the human ─────────────────

const CLASS_WORDS: Record<EvidenceClass, string> = {
  direct_stat: 'measured',
  derived: 'derived',
  model_summary: 'model summary',
  human: 'human',
}

/**
 * A `model_summary` must never look like a measurement, and content fetched from
 * the web must never look like the platform's own bars. Both are rendered as
 * words, not as a hue.
 */
export function ProvenanceBadge({ mark }: { mark: ProvenanceMark | null | undefined }) {
  if (!mark) return null
  const cls = mark.evidence_class
  return (
    <>
      {mark.provenance === 'untrusted' && (
        <Tooltip
          content={`Fetched from ${mark.source ?? 'an external source'}${
            mark.fetched_at ? ` at ${mark.fetched_at}` : ''
          }. Treated as data, never as instruction.`}
        >
          <span className="prov-mark" data-testid="provenance-untrusted">
            <span aria-hidden>⚑</span> fetched · {mark.source ?? 'external'}
          </span>
        </Tooltip>
      )}
      {cls && (
        <Badge tone={cls === 'direct_stat' ? 'info' : cls === 'model_summary' ? 'warn' : 'neutral'}>
          {CLASS_WORDS[cls]}
        </Badge>
      )}
    </>
  )
}

/**
 * Untrusted text, rendered as text.
 *
 * No `dangerouslySetInnerHTML`, no auto-linking, no image loading. A fetched page
 * cannot put a clickable link in front of the operator, because nothing here
 * turns a string into an element. React escapes the content; the `pre-wrap` block
 * makes it obvious that it is quoted material rather than the desk speaking.
 */
export function InertText({ children }: { children: string }) {
  return (
    <span className="inert-text" data-testid="inert-text">
      {children}
    </span>
  )
}

// ── §6.4 Confidence pill · §6.5 Risk badge ───────────────────────────────────

export function ConfidencePill({ value }: { value: Confidence }) {
  const tone = value === 'established' ? 'pos' : value === 'refuted' ? 'neg' : value === 'stale' ? 'neutral' : 'warn'
  return <Badge tone={tone}>{value}</Badge>
}

export function RiskBadge({ value }: { value: RiskClass }) {
  const icon = value === 'destructive' ? '⚠' : value === 'outbound' ? '↗' : null
  const tone = value === 'destructive' ? 'neg' : value === 'outbound' ? 'warn' : value === 'write' ? 'info' : 'neutral'
  return (
    <Badge tone={tone}>
      {icon && <span aria-hidden>{icon}</span>}
      {value}
    </Badge>
  )
}

// ── §6.7 Envelope banner ─────────────────────────────────────────────────────

export function EnvelopeBanner({
  envelope,
  changes,
  reversible,
}: {
  envelope: string
  changes: string
  reversible: boolean
}) {
  return (
    <div className="envelope" role="note">
      <strong>{envelope}</strong> · {changes} ·{' '}
      {reversible ? 'reversible' : 'not reversible — confirmation is two steps'}
    </div>
  )
}

// ── §6.8 Job progress ────────────────────────────────────────────────────────

/**
 * Deterministic progress, never an indeterminate spinner over content
 * (Meridian §3.22). When the job service cannot say how far along it is, the
 * card says *that* rather than animating a lie.
 */
export function JobProgress({
  done,
  planned,
  eta,
}: {
  done: number | null
  planned: number | null
  eta?: string | null
}) {
  if (done === null || planned === null || planned <= 0) {
    return <span className="sub">progress not reported by the job service</span>
  }
  const p = Math.round((done / planned) * 100)
  return (
    <div className="job-progress">
      <div className="meter" role="progressbar" aria-valuenow={p} aria-valuemin={0} aria-valuemax={100}>
        <i style={{ width: `${p}%` }} />
      </div>
      <span className="val">
        {done}/{planned}
        {eta ? ` · ${eta}` : ''}
      </span>
    </div>
  )
}

// ── §6.6 Step header ─────────────────────────────────────────────────────────

export function StepHeader({
  step,
  label,
  tools,
  seconds,
  open,
  onToggle,
  rawCount,
}: {
  step: number
  label: string
  tools: number
  seconds: number | null
  open: boolean
  onToggle: () => void
  rawCount: number
}) {
  return (
    <button type="button" className="desk-step-hd" onClick={onToggle} aria-expanded={open}>
      <span>Step {step}</span>
      <span>·</span>
      <span className="truncate-1" style={{ maxWidth: '38ch' }}>
        {label}
      </span>
      <span>·</span>
      <span>
        {tools} {tools === 1 ? 'tool' : 'tools'}
      </span>
      {seconds !== null && (
        <>
          <span>·</span>
          <span>{seconds.toFixed(0)} s</span>
        </>
      )}
      <span style={{ marginLeft: 'auto' }}>
        {rawCount > 0 ? `${open ? 'hide' : 'show'} ${rawCount} raw` : ''}
      </span>
    </button>
  )
}

// ── The ten-stage strip (AGENT-013 §5) ───────────────────────────────────────

export function StageStrip({ stage }: { stage: Stage }) {
  const at = STAGE_ORDER.indexOf(stage)
  return (
    <div className="stage-strip" aria-label={`lifecycle stage: ${stage}`}>
      {STAGE_ORDER.map((s, i) => (
        <span key={s} className={i === at ? 'on' : i < at ? 'past' : undefined}>
          {s}
        </span>
      ))}
    </div>
  )
}

// ── Small helpers used across the desk ───────────────────────────────────────

export function Row({ children, gap = 2 }: { children: ReactNode; gap?: number }) {
  return (
    <div style={{ display: 'flex', flexWrap: 'wrap', alignItems: 'center', gap: `var(--s-${gap})` }}>
      {children}
    </div>
  )
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
