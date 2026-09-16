// The card stream's card types (AGENT-012 §3.2; tasks S-6.4, S-6.5, S-6.23).
//
// One renderer per meaningful event, and the summary is always prose. The rule
// that shapes every card here: a tool card shows the result *summary*, never a
// raw table, because a raw table is the thing a person has to read the way they
// had to read the old transcript.
import { useState } from 'react'
import type { ReactNode } from 'react'
import { Badge } from '@/components/primitives/Badge'
import { Button } from '@/components/primitives/Button'
import type { Confidence, DeskCard, RiskClass, Stage } from '@/api/desk'
import {
  ConfidencePill,
  EnvelopeBanner,
  EvidenceChip,
  InertText,
  JobProgress,
  ProvenanceBadge,
  RiskBadge,
  StageStrip,
  ago,
  type IdResolver,
} from './Shared'

export interface CardActions {
  onOpenId?: (id: string) => void
  onSteer?: (text: string) => void
  onApprove?: (id: string, note?: string) => void
  onDeny?: (id: string) => void
  onAnswer?: (id: string, text: string) => void
  onStartResearch?: (question: string) => void
  onOpenDossier?: (ticket: string) => void
}

function str(v: unknown): string | null {
  return typeof v === 'string' && v.length > 0 ? v : null
}
function num(v: unknown): number | null {
  return typeof v === 'number' && Number.isFinite(v) ? v : null
}
function bool(v: unknown): boolean {
  return v === true
}

/**
 * A number without its sample size is not evidence (§19.1, principle 9).
 * Rendering "Sharpe 1.8" with no `n` beside it is the single easiest way for a
 * surface to launder a guess into a measurement, so the absence is printed.
 */
export function statLine(detail: Record<string, unknown> | null | undefined): string | null {
  if (!detail) return null
  const n = num(detail.n)
  const se = num(detail.se)
  if (n === null && se === null) return null
  const parts: string[] = []
  parts.push(n === null ? '(n unknown)' : `n=${n}`)
  if (se !== null) parts.push(`se=${se}`)
  return parts.join(' · ')
}

/** True when this card's content came off the wire as untrusted (§6.3a). */
function isUntrusted(card: DeskCard): boolean {
  return card.mark?.provenance === 'untrusted'
}

/**
 * Body text for a card.
 *
 * Untrusted text goes through `InertText`, which renders it as text and nothing
 * else — no markup, no auto-linking, no image loading. Trusted text is still
 * plain React children, so neither path can produce an element from a string.
 */
function Body({ card, text }: { card: DeskCard; text: string | null }) {
  if (!text) return null
  if (isUntrusted(card)) return <InertText>{text}</InertText>
  return <p>{text}</p>
}

function Ids({ card, resolve, onOpen }: { card: DeskCard; resolve?: IdResolver; onOpen?: (id: string) => void }) {
  if (!card.ids || card.ids.length === 0) return null
  return (
    <div className="desk-actions">
      {card.ids.map((id) => (
        <EvidenceChip key={id} id={id} resolve={resolve} onOpen={onOpen} />
      ))}
    </div>
  )
}

function Shell({
  card,
  kindLabel,
  tone,
  children,
  attn,
}: {
  card: DeskCard
  kindLabel: string
  tone?: 'you' | 'raw'
  children: ReactNode
  attn?: boolean
}) {
  return (
    <article
      className={['desk-card', tone ?? '', attn ? 'attn' : ''].filter(Boolean).join(' ')}
      tabIndex={0}
      data-card-kind={card.kind}
      aria-label={`${kindLabel}: ${card.headline}`}
    >
      <div className="desk-card-hd">
        <span className="desk-card-kind">{kindLabel}</span>
        <ProvenanceBadge mark={card.mark} />
        <span className="sub" style={{ marginLeft: 'auto' }}>
          {ago(card.at)} ago
        </span>
      </div>
      {children}
    </article>
  )
}

// ── The irreversible two-step (Meridian §5.5, §6.7) ──────────────────────────

/** Envelopes whose actions cannot be undone, so one click must never commit. */
const IRREVERSIBLE = new Set(['SealedHoldout', 'Promotion', 'Tier3Memory'])

export function isIrreversibleEnvelope(envelope: string | null | undefined): boolean {
  return IRREVERSIBLE.has(envelope ?? '')
}

function ApproveControls({
  id,
  envelope,
  actions,
}: {
  id: string
  envelope: string | null
  actions: CardActions
}) {
  const twoStep = isIrreversibleEnvelope(envelope)
  const [armed, setArmed] = useState(false)
  // Why this was approved. Optional, and offered only on the irreversible path:
  // a one-click approval that stopped to ask for prose would not be one click,
  // and the reversible ones are the ones you take back rather than explain.
  //
  // It is offered at all because the platform asks for it on the other side —
  // `spend_holdout_look` refuses without a reason of at least twenty characters,
  // written into an append-only log that outlives the run. Until this field
  // existed the desk had nowhere to type one, and `onApprove`'s `note` parameter
  // was declared in the type and passed by nobody.
  const [note, setNote] = useState('')
  return (
    <div className="desk-actions">
      <Button
        size="xs"
        variant={armed ? 'danger' : 'primary'}
        data-testid="approve"
        onClick={() => {
          if (twoStep && !armed) {
            setArmed(true)
            return
          }
          actions.onApprove?.(id, note.trim() || undefined)
        }}
      >
        {twoStep ? (armed ? `Confirm — ${envelope} cannot be undone` : 'Approve…') : 'Approve'}
      </Button>
      {armed && (
        <input
          className="desk-approve-note"
          data-testid="approve-note"
          type="text"
          value={note}
          placeholder="why — recorded permanently"
          onChange={(e) => setNote(e.target.value)}
        />
      )}
      {armed && (
        <Button size="xs" variant="ghost" onClick={() => setArmed(false)}>
          Cancel
        </Button>
      )}
      <Button size="xs" variant="secondary" data-testid="deny" onClick={() => actions.onDeny?.(id)}>
        Deny
      </Button>
    </div>
  )
}

// ── The dispatcher ───────────────────────────────────────────────────────────

export function Card({
  card,
  resolve,
  actions = {},
}: {
  card: DeskCard
  resolve?: IdResolver
  actions?: CardActions
}) {
  const d = card.detail ?? {}
  const open = actions.onOpenId

  switch (card.kind) {
    case 'you':
      return (
        <Shell card={card} kindLabel="you" tone="you">
          <Body card={card} text={card.headline} />
        </Shell>
      )

    case 'agent':
      return (
        <Shell card={card} kindLabel="agent">
          <Body card={card} text={card.headline} />
          <Ids card={card} resolve={resolve} onOpen={open} />
        </Shell>
      )

    case 'steer':
      return (
        <Shell card={card} kindLabel="steer">
          <Body card={card} text={card.headline} />
          <p className="sub">Sent into the running turn rather than queued behind it.</p>
        </Shell>
      )

    case 'plan': {
      const steps = Array.isArray(d.steps) ? (d.steps as Array<Record<string, unknown>>) : []
      const glyph: Record<string, string> = {
        pending: '[ ]',
        active: '[>]',
        done: '[x]',
        skipped: '[-]',
        failed: '[!]',
      }
      return (
        <Shell card={card} kindLabel="plan">
          <Body card={card} text={card.headline} />
          <ol style={{ margin: 'var(--s-2) 0 0', paddingLeft: 'var(--s-4)' }}>
            {steps.map((s, i) => {
              const status = str((s.status as Record<string, unknown>)?.status) ?? str(s.status) ?? 'pending'
              const kind = (s.kind ?? {}) as Record<string, unknown>
              return (
                <li key={i} style={{ fontSize: 'var(--t-12)' }}>
                  <span className="mono">{glyph[status] ?? '[ ]'}</span> {str(s.goal) ?? 'step'}{' '}
                  {str(kind.name) && <Badge tone="outline">{str(kind.name)}</Badge>}
                </li>
              )
            })}
          </ol>
          {steps.length === 0 && <p className="sub">The plan carried no steps.</p>}
          <div className="desk-actions">
            <Button size="xs" variant="ghost" onClick={() => actions.onSteer?.('why this plan?')}>
              Why this plan?
            </Button>
          </div>
        </Shell>
      )
    }

    case 'tool': {
      const args = d.arguments ?? d.args ?? null
      const summary = str(d.summary) ?? str(d.result_summary) ?? null
      const stats = statLine(d)
      const duration = num(d.duration_ms)
      const handle = str(d.handle)
      return (
        <Shell card={card} kindLabel="tool">
          <div className="desk-card-hd">
            <strong className="mono" style={{ fontSize: 'var(--t-12)' }}>
              {card.headline}
            </strong>
            {str(d.namespace) && <Badge tone="outline">{str(d.namespace)}</Badge>}
            <RiskBadge value={(str(d.risk) as RiskClass) ?? 'read'} />
            {duration !== null && <span className="sub">{(duration / 1000).toFixed(1)} s</span>}
          </div>
          {/* The summary as prose. Never the raw table — that is what the handle is for. */}
          <Body card={card} text={summary ?? 'This tool returned no summary.'} />
          {stats && <p className="sub">{stats}</p>}
          <Ids card={card} resolve={resolve} onOpen={open} />
          {args !== null && (
            <details style={{ marginTop: 'var(--s-2)' }}>
              <summary className="sub">arguments</summary>
              <pre className="mono well" style={{ fontSize: 'var(--t-11)', overflow: 'auto', maxHeight: 200 }}>
                {JSON.stringify(args, null, 2)}
              </pre>
            </details>
          )}
          {handle && (
            <div className="desk-actions">
              <Button size="xs" variant="ghost" onClick={() => open?.(handle)}>
                Open full result
              </Button>
            </div>
          )}
        </Shell>
      )
    }

    case 'finding': {
      const conf = (str(d.confidence) as Confidence) ?? 'suggestive'
      const stats = statLine(d)
      return (
        <Shell card={card} kindLabel="finding">
          <div className="desk-card-hd">
            <ConfidencePill value={conf} />
            {bool(d.cited_later) && <Badge tone="info">cited later</Badge>}
          </div>
          <Body card={card} text={card.headline} />
          {stats && <p className="sub">{stats}</p>}
          {str(d.window) && <p className="sub">window {str(d.window)}</p>}
          <Ids card={card} resolve={resolve} onOpen={open} />
          <div className="desk-actions">
            <Button size="xs" variant="ghost" onClick={() => actions.onSteer?.(`dispute: ${card.headline}`)}>
              Dispute
            </Button>
          </div>
        </Shell>
      )
    }

    case 'negative':
      return (
        <Shell card={card} kindLabel="negative result">
          <Body card={card} text={card.headline} />
          {str(d.failure_class) && <p className="sub">class: {str(d.failure_class)}</p>}
          {str(d.retry_when) && <p className="sub">retry when: {str(d.retry_when)}</p>}
        </Shell>
      )

    case 'decision':
      return (
        <Shell card={card} kindLabel="decision">
          <Body card={card} text={card.headline} />
          {str(d.reason) && <p className="sub">because {str(d.reason)}</p>}
          <div className="desk-actions">
            <Button
              size="xs"
              variant="ghost"
              onClick={() => actions.onSteer?.(`override: ${str(d.question) ?? card.headline}`)}
            >
              Override
            </Button>
          </div>
        </Shell>
      )

    case 'question':
      return (
        <QuestionCard card={card} actions={actions} />
      )

    case 'approval':
    case 'proposal': {
      const envelope = str(d.envelope)
      const id = str(d.approval_id) ?? str(d.id) ?? String(card.seq)
      return (
        <Shell card={card} kindLabel={card.kind === 'proposal' ? 'proposal' : 'approval'} attn>
          <Body card={card} text={card.headline} />
          {envelope && (
            <EnvelopeBanner
              envelope={envelope}
              changes={str(d.changes) ?? 'see the action above'}
              reversible={!isIrreversibleEnvelope(envelope)}
            />
          )}
          {card.kind === 'proposal' && str(d.ring) && <p className="sub">ring: {str(d.ring)}</p>}
          <Ids card={card} resolve={resolve} onOpen={open} />
          <ApproveControls id={id} envelope={envelope} actions={actions} />
        </Shell>
      )
    }

    case 'job':
      return (
        <Shell card={card} kindLabel="job">
          <Body card={card} text={card.headline} />
          <JobProgress done={num(d.done)} planned={num(d.planned)} eta={str(d.eta)} />
          <Ids card={card} resolve={resolve} onOpen={open} />
        </Shell>
      )

    case 'gate': {
      const licensed = Array.isArray(d.tickets) ? (d.tickets as string[]) : []
      const missing = Array.isArray(d.missing) ? (d.missing as string[]) : []
      return (
        <Shell card={card} kindLabel="gate" attn={missing.length > 0}>
          <Body card={card} text={card.headline} />
          {str(d.position) && <p className="sub">funnel position: {str(d.position)}</p>}
          <p className="sub">licensed by</p>
          <div className="desk-actions">
            {licensed.map((t) => (
              <EvidenceChip key={t} id={t} resolve={resolve} onOpen={open} />
            ))}
            {licensed.length === 0 && <span className="sub">nothing yet</span>}
          </div>
          {missing.length > 0 && <p className="sub">missing: {missing.join(', ')}</p>}
        </Shell>
      )
    }

    case 'escalation':
      return (
        <Shell card={card} kindLabel="escalation" attn>
          <Body card={card} text={card.headline} />
          {str(d.why) && <p className="sub">{str(d.why)}</p>}
        </Shell>
      )

    case 'correction':
      return (
        <Shell card={card} kindLabel="correction">
          <Body card={card} text={card.headline} />
          {str(d.fix) && <p className="sub">what it did next: {str(d.fix)}</p>}
        </Shell>
      )

    case 'compaction':
      return (
        <Shell card={card} kindLabel="compaction">
          <Body card={card} text={card.headline} />
          <Ids card={card} resolve={resolve} onOpen={open} />
        </Shell>
      )

    case 'report':
      return (
        <Shell card={card} kindLabel="report">
          <Body card={card} text={card.headline} />
          {str(d.assessment) && <p className="sub">{str(d.assessment)}</p>}
          <Ids card={card} resolve={resolve} onOpen={open} />
          {str(d.ticket) && (
            <div className="desk-actions">
              <Button size="xs" onClick={() => actions.onOpenDossier?.(str(d.ticket) as string)}>
                Open dossier
              </Button>
            </div>
          )}
        </Shell>
      )

    case 'stage': {
      const stage = (str(d.stage) as Stage) ?? 'explore'
      return (
        <Shell card={card} kindLabel="stage">
          <StageStrip stage={stage} />
          <p className="sub">
            {str(d.blocking) ? `what would move it: ${str(d.blocking)}` : 'nothing is blocking this stage'}
          </p>
        </Shell>
      )
    }

    case 'prereg':
      return (
        <Shell card={card} kindLabel="pre-registration">
          <Body card={card} text={card.headline} />
          <dl className="kv" style={{ fontSize: 'var(--t-11)' }}>
            {(
              [
                ['benchmark', str(d.benchmark)],
                ['delta_practical', str(d.delta_practical)],
                ['window', str(d.window)],
                ['embargo', str(d.embargo)],
                ['null family', str(d.null_family)],
                ['trial budget', num(d.trial_budget)?.toString() ?? null],
                ['falsifier', str(d.falsifier)],
                ['hash', str(d.hash)],
              ] as Array<[string, string | null]>
            ).map(([k, v]) => (
              <div key={k}>
                <span className="lbl">{k}</span>{' '}
                <span className="mono">{v ?? 'not recorded'}</span>
              </div>
            ))}
          </dl>
          <p className="sub">Sealed before any trial was spent.</p>
        </Shell>
      )

    case 'offer':
      return (
        <Shell card={card} kindLabel="this needs a research run" attn>
          <Body card={card} text={card.headline} />
          <p className="sub">
            {str(d.workflow) ? `workflow: ${str(d.workflow)} · ` : ''}
            {num(d.est_usd) !== null ? `about $${(num(d.est_usd) as number).toFixed(2)} · ` : ''}
            {num(d.est_trials) !== null ? `${num(d.est_trials)} trials` : 'trial count not estimated'}
          </p>
          <div className="desk-actions">
            <Button size="xs" onClick={() => actions.onStartResearch?.(card.headline)}>
              Start
            </Button>
          </div>
        </Shell>
      )

    case 'halt':
      return (
        <Shell card={card} kindLabel="halted" attn>
          <Body card={card} text={card.headline} />
          <p className="sub">
            {str(d.what_you_can_do) ?? 'Resume when the cause is cleared, or escalate to another tier.'}
          </p>
        </Shell>
      )

    default:
      return (
        <Shell card={card} kindLabel={card.headline} tone="raw">
          <pre className="mono well" style={{ fontSize: 'var(--t-11)', overflow: 'auto', maxHeight: 180 }}>
            {JSON.stringify(card.detail ?? {}, null, 2)}
          </pre>
        </Shell>
      )
  }
}

function QuestionCard({ card, actions }: { card: DeskCard; actions: CardActions }) {
  const d = card.detail ?? {}
  const options = Array.isArray(d.options) ? (d.options as string[]) : []
  const id = str(d.question_id) ?? String(card.seq)
  const [text, setText] = useState('')
  return (
    <Shell card={card} kindLabel="question" attn>
      <Body card={card} text={card.headline} />
      <div className="desk-actions">
        {options.map((o) => (
          <Button key={o} size="xs" variant="secondary" onClick={() => actions.onAnswer?.(id, o)}>
            {o}
          </Button>
        ))}
      </div>
      <div className="desk-actions">
        <input
          className="input"
          aria-label="Answer the agent"
          value={text}
          onChange={(e) => setText(e.target.value)}
        />
        <Button size="xs" onClick={() => actions.onAnswer?.(id, text)} disabled={!text.trim()}>
          Answer
        </Button>
      </div>
    </Shell>
  )
}
