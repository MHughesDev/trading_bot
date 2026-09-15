// The right panel (AGENT-012 §3.5; tasks S-6.8 … S-6.12, plus §2b's two tabs).
//
// The acceptance that shapes this file is §19.1: **a running turn is
// understandable from the State tab alone, in under a minute** — goal kind,
// current step, evidence count, budget and what it is waiting on, without
// opening a card. Everything in `StateTab` is ordered by that sentence, and
// anything that would need a click to become legible is not in it.
//
// State renders from the latest `state_snapshot`, never by replaying cards
// (§16). Replaying cards is how the old surfaces got a state that disagreed with
// the agent's.
import { useMemo, useState } from 'react'
import { Badge } from '@/components/primitives/Badge'
import { Button } from '@/components/primitives/Button'
import { SearchInput } from '@/components/primitives/Field'
import { Tabs } from '@/components/primitives/Segmented'
import { EmptyState } from '@/components/primitives/States'
import { Tooltip } from '@/components/primitives/Overlay'
import type {
  AgendaItem,
  Charter,
  DeskCard,
  Mode,
  ToolCard,
  WorkingState,
} from '@/api/desk'
import { toolsVisibleInMode, whyHidden } from '@/lib/desk/rings'
import {
  BudgetMeter,
  EvidenceChip,
  ProvenanceBadge,
  StageStrip,
  type IdResolver,
} from './Shared'

export type PanelTab = 'state' | 'evidence' | 'memory' | 'tools' | 'cost' | 'agenda' | 'charter'

// ── S-6.8 State ──────────────────────────────────────────────────────────────

const STEP_GLYPH: Record<string, string> = {
  pending: '[ ]',
  active: '[>]',
  done: '[x]',
  skipped: '[-]',
  failed: '[!]',
}

/** What the run is waiting on, in the words a person would use. */
export function waitingOn(state: WorkingState, opts: { jobPct?: number | null; pendingApproval?: boolean; paused?: boolean }): string {
  if (opts.paused) return 'paused — waiting on you to resume'
  if (opts.pendingApproval) return 'waiting on you to approve or deny'
  if (opts.jobPct !== null && opts.jobPct !== undefined) return `waiting on a job · ${Math.round(opts.jobPct)}%`
  const active = state.plan.find((s) => s.status.status === 'active')
  if (active) return `running step: ${active.goal}`
  const next = state.plan.find((s) => s.status.status === 'pending')
  if (next) return `about to run: ${next.goal}`
  if (state.last_error) return `recovering from: ${state.last_error.code}`
  return 'waiting on the model'
}

export function StateTab({
  state,
  resolve,
  onOpenId,
  jobPct,
  pendingApproval,
  paused,
}: {
  state: WorkingState | null
  resolve?: IdResolver
  onOpenId?: (id: string) => void
  jobPct?: number | null
  pendingApproval?: boolean
  paused?: boolean
}) {
  if (!state) {
    return (
      <EmptyState
        message="No working state yet"
        detail="The State tab renders the harness's latest state snapshot. It appears as soon as the run's first step lands."
      />
    )
  }
  const activeIndex = state.plan.findIndex((s) => s.status.status === 'active')
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--s-3)' }}>
      {/* 1. What is this, and what is it doing right now. */}
      <div>
        <div style={{ display: 'flex', gap: 'var(--s-2)', flexWrap: 'wrap', marginBottom: 'var(--s-2)' }}>
          <Badge tone="outline">{state.mode}</Badge>
          <Badge tone="outline">{state.workflow}</Badge>
          <Badge tone="outline">{state.kind}</Badge>
          <Badge tone={state.ring === 'research' ? 'neutral' : 'warn'}>ring: {state.ring}</Badge>
        </div>
        <p style={{ margin: 0, fontSize: 'var(--t-13)' }}>{state.goal || 'no goal recorded'}</p>
      </div>

      <StageStrip stage={state.stage} />

      <div className="well">
        <span className="lbl">waiting on</span>
        <p style={{ margin: 0, fontSize: 'var(--t-12)' }}>
          {waitingOn(state, { jobPct, pendingApproval, paused })}
        </p>
      </div>

      {/* 2. What "done" means, and how far along it is. */}
      <section>
        <span className="lbl">acceptance</span>
        {state.acceptance.length === 0 ? (
          <p className="sub">No acceptance criteria were derived for this goal.</p>
        ) : (
          <ul style={{ margin: 'var(--s-1) 0 0', paddingLeft: 'var(--s-4)', fontSize: 'var(--t-12)' }}>
            {state.acceptance.map((a) => (
              <li key={a}>{a}</li>
            ))}
          </ul>
        )}
      </section>

      {state.hypothesis && (
        <section className="well">
          <span className="lbl">hypothesis · {state.hypothesis.status}</span>
          <p style={{ margin: 0, fontSize: 'var(--t-12)' }}>{state.hypothesis.statement}</p>
          <p className="sub">falsifier: {state.hypothesis.falsifier}</p>
        </section>
      )}

      {state.prereg && (
        <section>
          <span className="lbl">pre-registration</span>
          <p className="sub mono">{state.prereg.hash}</p>
        </section>
      )}

      <section>
        <span className="lbl">plan · step {activeIndex >= 0 ? activeIndex + 1 : '—'} of {state.plan.length}</span>
        <ol style={{ margin: 'var(--s-1) 0 0', paddingLeft: 'var(--s-4)', fontSize: 'var(--t-12)' }}>
          {state.plan.map((s, i) => (
            <li key={i}>
              <span className="mono">{STEP_GLYPH[s.status.status] ?? '[ ]'}</span> {s.goal}
              {s.kind.name && <> · <span className="mono">{s.kind.name}</span></>}
              {s.status.why && <span className="sub"> — {s.status.why}</span>}
            </li>
          ))}
          {state.plan.length === 0 && <span className="sub">no plan yet</span>}
        </ol>
      </section>

      {/* 3. What it has produced, by count first so it reads at a glance. */}
      <section>
        <span className="lbl">evidence · {state.evidence.length}</span>
        <div className="desk-actions">
          {state.evidence.map((e) => (
            <Tooltip key={e.id} content={`from ${e.tool} at step ${e.step}`}>
              <span>
                <EvidenceChip id={e.id} resolve={resolve} onOpen={onOpenId} />
              </span>
            </Tooltip>
          ))}
          {state.evidence.length === 0 && <span className="sub">nothing yet</span>}
        </div>
      </section>

      <section>
        <span className="lbl">findings · {state.findings.length}</span>
        <ul style={{ margin: 'var(--s-1) 0 0', paddingLeft: 'var(--s-4)', fontSize: 'var(--t-12)' }}>
          {state.findings.map((f) => (
            <li key={f.id}>
              {f.headline} <EvidenceChip id={f.id} resolve={resolve} onOpen={onOpenId} />
            </li>
          ))}
          {state.findings.length === 0 && <span className="sub">nothing yet</span>}
        </ul>
      </section>

      {state.open_questions.length > 0 && (
        <section>
          <span className="lbl">open questions</span>
          <ul style={{ margin: 'var(--s-1) 0 0', paddingLeft: 'var(--s-4)', fontSize: 'var(--t-12)' }}>
            {state.open_questions.map((q) => (
              <li key={q}>{q}</li>
            ))}
          </ul>
        </section>
      )}

      {state.decided.length > 0 && (
        <section>
          <span className="lbl">decided</span>
          <ul style={{ margin: 'var(--s-1) 0 0', paddingLeft: 'var(--s-4)', fontSize: 'var(--t-12)' }}>
            {state.decided.map((d) => (
              <li key={d.question}>
                <strong>{d.question}</strong> → {d.choice}
                <span className="sub"> ({d.reason}, step {d.step})</span>
              </li>
            ))}
          </ul>
        </section>
      )}

      {state.last_error && (
        <section className="well">
          <span className="lbl">last error</span>
          <p style={{ margin: 0, fontSize: 'var(--t-12)' }}>{state.last_error.message}</p>
          <p className="sub">fix: {state.last_error.fix}</p>
        </section>
      )}

      {/* 4. What it has spent. */}
      <section>
        <span className="lbl">budget</span>
        <BudgetMeter budget={state.budget} />
      </section>
    </div>
  )
}

// ── S-6.9 Evidence ───────────────────────────────────────────────────────────

export interface EvidenceEntry {
  id: string
  type: string
  summary: string
  /** Rendered in place when the id is opened. */
  view?: React.ReactNode
}

export function EvidenceTab({
  entries,
  resolve,
  onOpenId,
}: {
  entries: EvidenceEntry[]
  resolve?: IdResolver
  onOpenId?: (id: string) => void
}) {
  const [openId, setOpenId] = useState<string | null>(null)
  const grouped = useMemo(() => {
    const m = new Map<string, EvidenceEntry[]>()
    for (const e of entries) {
      const list = m.get(e.type) ?? []
      list.push(e)
      m.set(e.type, list)
    }
    return [...m.entries()]
  }, [entries])

  if (entries.length === 0) {
    return <EmptyState message="This run has produced no ids yet" detail="Every id a tool returns appears here, grouped by what it is." />
  }

  return (
    <div>
      {grouped.map(([type, list]) => (
        <section key={type} style={{ marginBottom: 'var(--s-3)' }}>
          <span className="lbl">
            {type} · {list.length}
          </span>
          {list.map((e) => (
            <div key={e.id} className="desk-card">
              <div className="desk-card-hd">
                <EvidenceChip
                  id={e.id}
                  resolve={resolve}
                  onOpen={(id) => {
                    setOpenId((cur) => (cur === id ? null : id))
                    onOpenId?.(id)
                  }}
                />
              </div>
              <p className="sub">{e.summary}</p>
              {openId === e.id && e.view}
            </div>
          ))}
        </section>
      ))}
    </div>
  )
}

// ── S-6.10 Memory ────────────────────────────────────────────────────────────

export interface BriefItem {
  id: string
  bucket: string
  text: string
  /** Why the retriever chose it; shown on hover, never hidden entirely. */
  why_retrieved: string
  cited: boolean
  mark?: Parameters<typeof ProvenanceBadge>[0]['mark']
}

export function MemoryTab({
  items,
  onForget,
  onAdd,
}: {
  items: BriefItem[] | null
  onForget?: (id: string) => void
  onAdd?: () => void
}) {
  if (items === null) {
    return (
      <EmptyState
        message="The brief is not available yet"
        detail="Memory retrieval lands with Set S phase 4. Until then the desk cannot say what the brief contained, and it will not guess."
      />
    )
  }
  if (items.length === 0) {
    return <EmptyState message="The brief was empty" detail="Nothing in memory matched this goal." />
  }
  return (
    <div>
      {items.map((i) => (
        <div key={i.id} className="desk-card">
          <div className="desk-card-hd">
            <Badge tone="outline">{i.bucket}</Badge>
            <Badge tone={i.cited ? 'pos' : 'neutral'}>{i.cited ? 'cited' : 'not cited'}</Badge>
            <ProvenanceBadge mark={i.mark} />
          </div>
          <Tooltip content={`why retrieved: ${i.why_retrieved}`}>
            <p>{i.text}</p>
          </Tooltip>
          <p className="sub">why retrieved: {i.why_retrieved}</p>
          <div className="desk-actions">
            <Button size="xs" variant="ghost" onClick={() => onForget?.(i.id)}>
              Forget
            </Button>
          </div>
        </div>
      ))}
      <Button size="xs" variant="secondary" onClick={onAdd}>
        Add to memory
      </Button>
    </div>
  )
}

// ── S-6.11 Tools ─────────────────────────────────────────────────────────────

/**
 * The tool surface, filtered by mode.
 *
 * This is the desk's own ring door (see `lib/desk/rings.ts`): a Chat turn must
 * not be able to surface a trial-spending tool from here, and a tool that is
 * hidden says why rather than vanishing without explanation.
 */
export function ToolsTab({
  mode,
  exposed,
  catalogue,
  searchedFor,
  onPin,
}: {
  mode: Mode
  /** What the harness's `Note::Exposure` says this step actually saw. */
  exposed: string[] | null
  catalogue: ToolCard[] | null
  searchedFor: string[]
  onPin?: (name: string) => void
}) {
  const [q, setQ] = useState('')
  const all = catalogue ?? []
  const visible = toolsVisibleInMode(all, mode)
  const hidden = all.filter((t) => !visible.includes(t))
  const filtered = visible.filter((t) => t.name.toLowerCase().includes(q.toLowerCase()))

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--s-3)' }}>
      <section>
        <span className="lbl">exposed this step</span>
        {exposed === null ? (
          <p className="sub">The harness has not reported an exposure note for this step.</p>
        ) : (
          <div className="desk-actions">
            {exposed.map((n) => (
              <span key={n} className="chip">
                {n}
              </span>
            ))}
            {exposed.length === 0 && <span className="sub">nothing exposed</span>}
          </div>
        )}
      </section>

      {searchedFor.length > 0 && (
        <section>
          <span className="lbl">searched for</span>
          <p className="sub">{searchedFor.join(' · ')}</p>
        </section>
      )}

      <section>
        <span className="lbl">catalogue</span>
        {catalogue === null ? (
          <p className="sub">The catalogue endpoint is not built yet (AGENT-006, Set S phase 3).</p>
        ) : (
          <>
            <SearchInput value={q} onValueChange={setQ} placeholder="Search tools" />
            {filtered.map((t) => (
              <div key={t.name} className="desk-card">
                <div className="desk-card-hd">
                  <strong className="mono" style={{ fontSize: 'var(--t-12)' }}>
                    {t.name}
                  </strong>
                  <Badge tone="outline">{t.namespace}</Badge>
                  {t.spends_trial && <Badge tone="warn">spends a trial</Badge>}
                </div>
                {t.notes && <p className="sub">{t.notes}</p>}
                <div className="desk-actions">
                  <Button size="xs" variant="ghost" onClick={() => onPin?.(t.name)}>
                    Pin for this run
                  </Button>
                </div>
              </div>
            ))}
            {hidden.length > 0 && (
              <p className="sub" data-testid="tools-hidden">
                {hidden.length} {hidden.length === 1 ? 'tool is' : 'tools are'} not available in {mode}:{' '}
                {hidden.map((t) => `${t.name} (${whyHidden(mode, t)})`).join('; ')}
              </p>
            )}
          </>
        )}
      </section>
    </div>
  )
}

// ── S-6.12 Cost ──────────────────────────────────────────────────────────────

export interface CallCost {
  step: number
  layers: { charter: number; schemas: number; body: number; output: number }
  cache_hit_pct: number
  /** What the provider actually billed for the prompt. */
  provider_prompt_tokens: number | null
  usd_tenth_cents: number
}

export function CostTab({
  calls,
  bytesPerTokenCalibrated,
  bytesPerTokenEstimate,
}: {
  calls: CallCost[]
  bytesPerTokenCalibrated: number | null
  bytesPerTokenEstimate: number | null
}) {
  if (calls.length === 0) {
    return <EmptyState message="No model call has been accounted yet" detail="Per-layer token accounting appears once the first call returns usage." />
  }
  const cumulative = calls.reduce((a, c) => a + c.usd_tenth_cents, 0)
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--s-3)' }}>
      <section>
        <span className="lbl">cumulative</span>
        <p className="mono">${(cumulative / 1000).toFixed(3)}</p>
      </section>
      <section>
        <span className="lbl">bytes per token</span>
        <p className="sub">
          calibrated {bytesPerTokenCalibrated ?? 'unmeasured'} · harness estimate{' '}
          {bytesPerTokenEstimate ?? 'unmeasured'}
        </p>
      </section>
      <section>
        <span className="lbl">cache hit, per call</span>
        {/* Per call, not per run: a run-level average hides the call where the
            prefix was rebuilt, which is the one worth finding. */}
        <div style={{ display: 'flex', alignItems: 'flex-end', gap: 2, height: 32 }} aria-label="cache hit per call">
          {calls.map((c) => (
            <Tooltip key={c.step} content={`step ${c.step}: ${c.cache_hit_pct}%`}>
              <i
                style={{
                  display: 'block',
                  width: 6,
                  height: `${Math.max(2, c.cache_hit_pct * 0.3)}px`,
                  background: 'var(--bg-accent)',
                }}
              />
            </Tooltip>
          ))}
        </div>
      </section>
      <section>
        <span className="lbl">tokens by layer</span>
        <table className="tbl">
          <thead>
            <tr>
              <th style={{ textAlign: 'left' }}>step</th>
              <th>charter</th>
              <th>schemas</th>
              <th>body</th>
              <th>output</th>
              <th>provider</th>
            </tr>
          </thead>
          <tbody>
            {calls.map((c) => {
              const sum = c.layers.charter + c.layers.schemas + c.layers.body
              const drift =
                c.provider_prompt_tokens === null ? null : Math.abs(sum - c.provider_prompt_tokens)
              return (
                <tr key={c.step}>
                  <td style={{ textAlign: 'left' }}>{c.step}</td>
                  <td>{c.layers.charter}</td>
                  <td>{c.layers.schemas}</td>
                  <td>{c.layers.body}</td>
                  <td>{c.layers.output}</td>
                  <td>
                    {c.provider_prompt_tokens ?? 'not reported'}
                    {drift !== null && drift > 0 ? ` (±${drift})` : ''}
                  </td>
                </tr>
              )
            })}
          </tbody>
        </table>
      </section>
    </div>
  )
}

// ── §2b Agenda and Charter, tabs inside the session's own conversation ───────

export function AgendaTab({
  items,
  onVeto,
  onApprove,
  onAdd,
  onOpenChild,
}: {
  items: AgendaItem[] | null
  onVeto?: (id: string) => void
  onApprove?: (id: string) => void
  onAdd?: () => void
  onOpenChild?: (conversationId: string) => void
}) {
  if (items === null) {
    return (
      <EmptyState
        message="The agenda is not available yet"
        detail="Continuous sessions land with Set S phase 11. The tab is wired and will fill the day the endpoint exists."
      />
    )
  }
  const columns: AgendaItem['state'][] = ['proposed', 'queued', 'running', 'done', 'vetoed']
  return (
    <div>
      <Button size="xs" variant="secondary" onClick={onAdd}>
        Add a question
      </Button>
      {columns.map((col) => {
        const list = items.filter((i) => i.state === col)
        return (
          <section key={col} style={{ marginTop: 'var(--s-3)' }}>
            <span className="lbl">
              {col} · {list.length}
            </span>
            {list.map((i) => (
              <div key={i.item_id} className="desk-card">
                <div className="desk-card-hd">
                  <Badge tone="outline">{i.workflow}</Badge>
                  {i.sources.map((s, n) => (
                    <Badge key={`${s}-${n}`} tone="neutral">
                      {s}
                    </Badge>
                  ))}
                  {i.exploration_pick && <Badge tone="info">exploration</Badge>}
                </div>
                <p>{i.question}</p>
                <p className="sub">
                  value {i.score.value} · novelty {i.score.novelty} · feasibility {i.score.feasibility} · cost{' '}
                  {i.score.cost} → {i.score.total}
                </p>
                <p className="sub">{i.rationale}</p>
                {i.child_conversation_id && (
                  <Button size="xs" variant="ghost" onClick={() => onOpenChild?.(i.child_conversation_id as string)}>
                    Open the run
                  </Button>
                )}
                {(col === 'proposed' || col === 'queued') && (
                  <div className="desk-actions">
                    <Button size="xs" variant="secondary" onClick={() => onApprove?.(i.item_id)}>
                      Approve
                    </Button>
                    <Button size="xs" variant="ghost" onClick={() => onVeto?.(i.item_id)}>
                      Veto
                    </Button>
                  </div>
                )}
              </div>
            ))}
            {list.length === 0 && <p className="sub">nothing</p>}
          </section>
        )
      })}
    </div>
  )
}

export function CharterTab({ charter, amendments }: { charter: Charter | null; amendments: string[] }) {
  if (!charter) {
    return <EmptyState message="This conversation has no charter" detail="Turn on Continuous to give the session one." />
  }
  return (
    <div>
      <section>
        <span className="lbl">objective</span>
        <p>{charter.objective}</p>
      </section>
      <section>
        <span className="lbl">scope</span>
        <p className="sub">
          {[charter.instruments, charter.asset_classes, charter.families, charter.timeframes]
            .map((l) => (l.length ? l.join(', ') : '—'))
            .join(' · ')}
        </p>
      </section>
      <section>
        <span className="lbl">budget</span>
        <p className="sub">
          ${charter.budget_usd_day}/day
          {charter.budget_usd_total !== null ? ` · $${charter.budget_usd_total} total` : ''} · autonomy{' '}
          {charter.autonomy} · {charter.replenish ? 'keeps generating questions' : 'stops when the agenda empties'}
        </p>
      </section>
      <section>
        <span className="lbl">amendments</span>
        {amendments.length === 0 ? (
          <p className="sub">none</p>
        ) : (
          <ol style={{ paddingLeft: 'var(--s-4)', fontSize: 'var(--t-12)' }}>
            {amendments.map((a, i) => (
              <li key={i}>{a}</li>
            ))}
          </ol>
        )}
      </section>
    </div>
  )
}

// ── The panel itself ─────────────────────────────────────────────────────────

export function RightPanel({
  tab,
  onTab,
  continuous,
  children,
}: {
  tab: PanelTab
  onTab: (t: PanelTab) => void
  continuous: boolean
  children: React.ReactNode
}) {
  const tabs: Array<{ value: PanelTab; label: string }> = [
    { value: 'state', label: 'State' },
    { value: 'evidence', label: 'Evidence' },
    { value: 'memory', label: 'Memory' },
    { value: 'tools', label: 'Tools' },
    { value: 'cost', label: 'Cost' },
  ]
  if (continuous) {
    tabs.push({ value: 'agenda', label: 'Agenda' }, { value: 'charter', label: 'Charter' })
  }
  return (
    <>
      <div className="desk-side-hd">
        <Tabs<PanelTab> value={tab} onChange={onTab} tabs={tabs} ariaLabel="Right panel" />
      </div>
      <div className="desk-side-bd">{children}</div>
    </>
  )
}

/** Cards that carry a pending decision, used to light `a`/`x` in the controls row. */
export function pendingApprovals(cards: DeskCard[]): DeskCard[] {
  return cards.filter((c) => c.kind === 'approval' || c.kind === 'proposal')
}
