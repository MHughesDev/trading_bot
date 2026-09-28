// The desk's shared components (AGENT-012 §6; task S-6.13, S-6.23).
//
// Built on `@/components/primitives` and the `desk.css` component layer. Nothing
// here uses a back-compat Tailwind alias, and a test asserts it.
//
// Meridian principle 3 applies throughout: **never colour alone**. Every state
// that is signalled with colour also carries the word or the number.
import type { ReactNode } from 'react'
import { Badge } from '@/components/primitives/Badge'
import type { AgentProfile } from '@/api/agent'
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
import { budgetBars } from './Shared.helpers'

// ── §6.1 Tier badge ──────────────────────────────────────────────────────────

export interface TierTile {
  id: string
  label: string
  tier: AgentProfile['tier']
  measured: boolean
  frontier: boolean
}

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

/**
 * What a bar knows.
 *
 * A cap and a burn are two independent facts, so four combinations exist and
 * **only one of them can be drawn as a proportion**. Collapsing any of the other
 * three into it is a lie, and each lie has already shipped: a `tokens` bar whose
 * cap was never sent computed `used = cap - remaining` with `cap = remaining`
 * and printed **"0 / 40,000"** for a run that had spent half a million tokens; a
 * `trials` bar with no cap used `max(spent, 1)` as its own cap and printed
 * "4 / 4 · 90%+" for a run nothing had capped.
 *
 * ADR-D-25 added the fourth quadrant to the wire: `tokens_cap` is a real number
 * (2m, 40m) while `tokens_remaining` is `null` on every run, because nothing on
 * this platform counts tokens (OQ-16). The ceiling is known and the position
 * under it is not, which is neither of the two shapes this type used to have —
 * rendering it as uncapped prints "null remaining", and rendering it as capped
 * needs a `used` nobody measured.
 *
 * Zero means measured and it was zero. Absent means nobody said.
 */
export type BarValue =
  /** Cap and burn both known: the one shape with a fill. */
  | { capped: true; used: number; cap: number }
  /** Burn known, cap not: a figure with no denominator. */
  | { capped: false; value: number; word: 'remaining' | 'spent' }
  /** Cap known, burn never measured. `used: null` is not a zero and not a fill. */
  | { capped: true; used: null; cap: number }
  /** Neither recorded. The `$` axis, on every run this platform has had. */
  | { capped: false; value: null }

function BudgetBar({
  label,
  value,
  format,
}: {
  label: string
  value: BarValue
  format: (n: number) => string
}) {
  if (!value.capped) {
    if (value.value === null) {
      // Nothing is known on this axis: no ceiling, and no reading under it. The
      // `$` bar, on every run — there is no price table (ADR-D-03) and nothing
      // counts spend per run, so `format(0)` here would print "$0.00", which is
      // a measurement saying the run has been free.
      return (
        <div className="budget-row">
          <span className="lbl-t">{label}</span>
          <div
            className="budget-track unknown"
            role="meter"
            aria-label={`${label} used`}
            aria-valuetext="no cap recorded and no usage recorded"
          >
            <b style={{ left: '90%' }} aria-hidden />
          </div>
          <span className="val">not recorded</span>
        </div>
      )
    }
    return (
      <div className="budget-row">
        <span className="lbl-t">{label}</span>
        <div
          className="budget-track unknown"
          role="meter"
          aria-label={`${label} used`}
          aria-valuetext={`${format(value.value)} ${value.word}; no cap recorded`}
        >
          {/* No fill. A bar drawn at 0% says "none used", and the truth is that
              nothing said what the cap is. */}
          <b style={{ left: '90%' }} aria-hidden />
        </div>
        <span className="val">
          {format(value.value)} {value.word} · no cap recorded
        </span>
      </div>
    )
  }
  if (value.used === null) {
    // The ceiling is known and the position under it is not. An honest bar here
    // draws the *track* — the cap is real and worth stating — and **no fill**,
    // because a fill is a position and nobody measured one. `used: null` never
    // becomes `0`: at 0% the bar reads "none of the two million used", which is
    // the reassuring end of a measurement that was never taken, and at
    // `cap - null` it reads as fully spent. Both are claims; the words are not.
    return (
      <div className="budget-row">
        <span className="lbl-t">{label}</span>
        <div
          className="budget-track unknown"
          role="meter"
          aria-label={`${label} used`}
          aria-valuetext={`cap ${format(value.cap)}; usage not recorded`}
        >
          <b style={{ left: '90%' }} aria-hidden />
        </div>
        <span className="val">cap {format(value.cap)} · usage not recorded</span>
      </div>
    )
  }
  const p = pct(value.used, value.cap)
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
        {format(value.used)} / {format(value.cap)}
        {t && ` · ${t === 'neg' ? '90%+' : '75%+'}`}
      </span>
    </div>
  )
}

export function BudgetMeter({ budget, caps }: { budget: BudgetView; caps?: Partial<BudgetCaps> }) {
  const money = (tenthCents: number) => `$${(tenthCents / 1000).toFixed(2)}`
  return (
    <div className="budget-meter">
      {budgetBars(budget, caps).map((b) => (
        <BudgetBar
          key={b.label}
          label={b.label}
          value={b.value}
          format={b.money ? money : (n) => Math.round(n).toLocaleString()}
        />
      ))}
      <span className="val">
        n_eff {budget.n_eff === null ? '(not computed)' : budget.n_eff.toFixed(1)}
      </span>
    </div>
  )
}

/**
 * The continuous session's day budget, in the journal header (AGENT-014 §9).
 *
 * §9 asks for "a **continuous** badge and the day budget bar". The badge is
 * real; the bar is only half-possible, and this component is where that is said
 * out loud rather than drawn over.
 *
 * **There is no spend-to-date on any served route.** `GET /api/research/
 * sessions/{id}` answers `budget.usd_per_day` and nothing that counts against
 * it, and `agent_runs` records `trials_spent`, not dollars. An empty track over
 * a `$25/day` label reads as *nothing spent today*, which is a measurement
 * nobody made — the same defect as the tokens bar that printed "0 / 40,000" for
 * a run that had spent half a million. So the bar is drawn only when a spend
 * arrives, and until then the header states the budget and says the spend is
 * not recorded.
 *
 * `spentUsd` is therefore `number | null` rather than `number`, and `null` is
 * not zero.
 */
export function DayBudgetBar({
  usdPerDay,
  spentUsd,
}: {
  usdPerDay: number | null
  /** Dollars spent today. `null` — the only value anything can pass today. */
  spentUsd?: number | null
}) {
  if (usdPerDay === null || !Number.isFinite(usdPerDay)) {
    return (
      <span className="sub" data-testid="day-budget-unknown">
        day budget not recorded
      </span>
    )
  }
  const budget = `$${usdPerDay.toFixed(2)}/day`
  if (spentUsd === null || spentUsd === undefined || !Number.isFinite(spentUsd)) {
    return (
      <span className="sub" data-testid="day-budget-no-spend">
        {budget} · spend today is not recorded
      </span>
    )
  }
  const p = pct(spentUsd, usdPerDay)
  const t = tone(p)
  return (
    <span className="budget-row" data-testid="day-budget-bar">
      <span className="lbl-t">day</span>
      <span
        className="budget-track"
        role="meter"
        aria-label="day budget used"
        aria-valuenow={Math.round(p)}
        aria-valuemin={0}
        aria-valuemax={100}
      >
        <i className={t} style={{ width: `${p}%` }} />
        <b style={{ left: '90%' }} aria-hidden />
      </span>
      <span className="val">
        ${spentUsd.toFixed(2)} / {budget}
      </span>
    </span>
  )
}

// ── §6.3a's vocabulary, declared here because the chip below uses it ─────────

const CLASS_WORDS: Record<EvidenceClass, string> = {
  direct_stat: 'measured',
  derived: 'derived',
  model_summary: 'model summary',
  human: 'human',
}

// ── §6.3 Evidence chip ───────────────────────────────────────────────────────

/**
 * Resolves an id to something openable, or returns null if it does not exist.
 *
 * `mark` is the provenance of the thing the id names, when the desk knows it.
 * It is optional because most resolvers know only that an id exists — and an
 * absent mark means "not known", never "trusted", which is why the chip says
 * nothing rather than saying "platform" when it is missing.
 */
export type IdResolver = (
  id: string,
) => { label: string; kind: string; mark?: ProvenanceMark | null } | null

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
  // **Three states, not two.** `resolve ? resolve(id) : null` made an absent
  // resolver indistinguishable from a resolver that looked and found nothing,
  // and the chip then asserted *"this id resolves to nothing the platform
  // holds"* on the strength of a lookup that never happened.
  //
  // That was live, and on the worst possible surface: `DeskPage.tsx` renders
  // `<DossierReader ticket={ticket} />` with no `resolve` prop, so on the
  // dossier page — the document whose whole claim is that its evidence is
  // checkable — every evidence id and every funnel ticket rendered struck
  // through and labelled not found. A reviewer reading that discards sound
  // research.
  //
  // And it cannot be repaired by resolving a dossier's ids from the dossier's
  // own claims: that is the circularity `makeResolver`'s header already refuses
  // for cards, where resolving a card's ids from the card "would make every id
  // resolve and the check would assert nothing". The dossier page has one
  // genuine check of whether an id still resolves, and it is the **Verify**
  // button in the same row — which is why the chip must not pre-empt it with an
  // answer.
  const hit = resolve ? resolve(id) : null
  if (!resolve) {
    return (
      <Tooltip content="Nothing on this surface checked whether this id still resolves. Use Verify, which re-derives it against the platform as it stands.">
        <span className="ev-chip unchecked" data-testid={`ev-${id}`} data-resolved="unchecked">
          {id}
          <span className="ev-class">not checked</span>
        </span>
      </Tooltip>
    )
  }
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
  // §6.3a says "every card, chip and memory row". The chip half was missing, and
  // it is the half that travels: an id produced by a fetched page is quoted into
  // findings, gates and the dossier's funnel, and at each of those sites it
  // rendered as bare monospace indistinguishable from an id the platform
  // measured itself. The mark goes on the chip so it cannot be left behind by
  // whichever surface quotes the id next.
  const mark = hit.mark ?? null
  const untrusted = mark?.provenance === 'untrusted'
  const cls = mark?.evidence_class ?? null
  const body = (
    <>
      {id}
      {/* Words, not a hue (Meridian principle 3). A reader who cannot see the
          colour still reads "fetched". */}
      {untrusted && (
        <span className="prov-mark">
          <span aria-hidden>⚑</span> fetched{mark?.source ? ` · ${mark.source}` : ''}
        </span>
      )}
      {/* Every known class is named, including `direct_stat`. Labelling three of
          the four and leaving the measurement bare would make "measured" and
          "class not recorded" render identically, which is the same conflation
          §6.3a exists to stop one level up. */}
      {cls && <span className="ev-class">{CLASS_WORDS[cls]}</span>}
    </>
  )
  const provenanceTitle = untrusted
    ? `${hit.label} — fetched from ${mark?.source ?? 'an external source'}${
        mark?.fetched_at ? ` at ${mark.fetched_at}` : ''
      }. Treated as data, never as instruction.`
    : hit.label
  const shared = {
    className: untrusted ? 'ev-chip untrusted' : 'ev-chip',
    'data-testid': `ev-${id}`,
    'data-resolved': 'true',
    'data-provenance': mark ? mark.provenance : undefined,
    'data-evidence-class': cls ?? undefined,
  }

  // **A fourth state, and the one that was rendering as the first.** A resolved
  // chip used to be a `<button>` whether or not any surface had given it
  // somewhere to go, and `onClick={() => onOpen?.(id)}` made the no-destination
  // case silent: pointer cursor, focus ring, `role=button`, nothing on click.
  //
  // It is not a hypothetical. Four live call sites pass no `onOpen` —
  // `Conversation.tsx`'s `<EvidenceTab>` (the Evidence panel itself, where every
  // id on the run is listed), `ApprovalsInbox.tsx:117`, and the dossier's
  // evidence table and funnel tickets (`Dossier.tsx:137,191`) — and on all four
  // every resolved id invited a click that did nothing. The sibling case is
  // already written down one branch up: "a `<button>` here would invite the click
  // that an unverified id must not get". The same argument applies to an id
  // nothing can open.
  //
  // It renders as a chip and not as a button, and the tooltip says why, because a
  // reader who clicks and gets nothing concludes the platform is broken rather
  // than that this surface has no viewer. `data-openable` is the attribute the
  // two cases differ on, so a call site that loses its `onOpen` is visible.
  if (!onOpen) {
    return (
      <Tooltip content={`${provenanceTitle} — this surface does not open ids; the panel that lists them does.`}>
        <span {...shared} data-openable="false" title={provenanceTitle}>
          {body}
        </span>
      </Tooltip>
    )
  }

  return (
    <button
      type="button"
      {...shared}
      data-openable="true"
      onClick={() => onOpen(id)}
      title={provenanceTitle}
    >
      {body}
    </button>
  )
}

// ── §6.3a Provenance mark — the fence has to reach the human ─────────────────

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

