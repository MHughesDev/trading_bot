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
  CharterView,
  DeskCard,
  Mode,
  ToolCard,
  WorkingState,
} from '@/api/desk'
import type { CallCost, ExposureNote, ToolSearch } from '@/lib/desk/events'
import { toolsVisibleInMode, whyHidden } from '@/lib/desk/rings'
import {
  BudgetMeter,
  EvidenceChip,
  ProvenanceBadge,
  RiskBadge,
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
          {/*
            The mode is labelled for the reason the ring is: `mode` and `kind`
            both take the value "research" and both rendered as a bare badge, so
            the panel showed two identical chips meaning different things. That
            is not a test artefact — a person reading "research · falsify ·
            research" cannot tell which is which either.
          */}
          <Badge tone="outline">mode: {state.mode}</Badge>
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
        message="No endpoint reports what this run's brief contained"
        detail={
          <>
            The memory <em>store</em> is served and browsable — `/api/memory/&#123;bucket&#125;`
            answers, and the Memory page reads it. What has no source is the <em>brief</em>: which
            entries retrieval put into this run's prompt, why each was chosen, and whether the run
            cited it. That is a per-run record nothing writes, so the desk cannot say, and it will
            not show the store here and let it read as the brief.
          </>
        }
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
  catalogueLoading = false,
  searchedFor,
  exposure,
  onPin,
}: {
  mode: Mode
  /** What the harness's `Note::Exposure` says this step actually saw. */
  exposed: string[] | null
  /** `null` means the endpoint did not answer; `[]` means it answered with none. */
  catalogue: ToolCard[] | null
  catalogueLoading?: boolean
  /**
   * Every `search_tools` call on this run's stream (S-6.11).
   *
   * Read off `Note::CoreTool`, which has carried it all along; this prop was a
   * literal `[]` at the only call site, so the section never rendered.
   */
  searchedFor: ToolSearch[]
  /**
   * The rest of that same note — why *that* set and not another.
   *
   * Optional because `exposed` is the answer and this is the explanation; a
   * caller that has one has both, and the two tests that predate this pass only
   * the list.
   */
  exposure?: ExposureNote | null
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
          <p className="sub" data-testid="exposure-absent">
            This run has emitted no exposure note. The harness writes one per step and the desk
            reads the latest off this conversation's own stream, so an absent note means no step
            has run — not that the model was shown nothing.
          </p>
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
        {/* The note's own answer to "why that set": an exposure without its
            budget and namespaces is unexplainable after the fact, and exposure
            is where the harness's worst observed behaviours started. */}
        {exposure && (
          <p className="sub" data-testid="exposure-why">
            {exposure.exposed.length} of a {exposure.budget ?? 'unstated'} budget
            {exposure.namespaces.length > 0 && <> · from {exposure.namespaces.join(', ')}</>}
            {exposure.pinned.length > 0 && <> · held because the model named them: {exposure.pinned.join(', ')}</>}
            {exposure.plan_step && <> · serving “{exposure.plan_step}”</>}
          </p>
        )}
      </section>

      <section>
        <span className="lbl">searched for</span>
        {searchedFor.length === 0 ? (
          <p className="sub" data-testid="no-tool-search">
            Nothing on this conversation's stream records a `search_tools` call. The harness
            answers that tool itself and writes a note per search, so this means the model has not
            gone looking — not that searches go unrecorded.
          </p>
        ) : (
          <ul data-testid="tool-searches" style={{ fontSize: 'var(--t-12)', paddingLeft: 'var(--s-4)' }}>
            {searchedFor.map((s, i) => (
              <li key={`${s.step}-${i}`}>
                {s.query === null ? (
                  <>a search this panel could not read: {s.detail}</>
                ) : (
                  <>
                    “{s.query}” →{' '}
                    {s.found === null ? (
                      'what it returned could not be read'
                    ) : s.found.length === 0 ? (
                      /* The case worth finding. A search that matched nothing is
                         why a model goes round in circles naming tools it was
                         never offered, and it is invisible in the exposure
                         note. */
                      <span data-testid="search-matched-nothing">no tool matched</span>
                    ) : (
                      s.found.join(', ')
                    )}
                  </>
                )}
              </li>
            ))}
          </ul>
        )}
      </section>

      <section>
        <span className="lbl">catalogue</span>
        {/* Three states, not two. This panel used to say "the catalogue endpoint
            is not built yet" — which stopped being true when S-6.19 shipped
            `/api/catalogue`, and the panel went on saying it because nothing
            ever called the endpoint. A surface that explains its own emptiness
            with a stale reason is worse than one that says nothing, because it
            tells the reader not to look. */}
        {catalogueLoading ? (
          <p className="sub">Reading the catalogue…</p>
        ) : catalogue === null ? (
          <p className="sub" data-testid="catalogue-unavailable">
            /api/catalogue is served but did not answer, so this is the request failing rather
            than the surface being unbuilt. The tools below are not "none" — they are unknown.
          </p>
        ) : catalogue.length === 0 ? (
          <p className="sub" data-testid="catalogue-empty">
            The catalogue answered with no tools at all, which is a fact about the registry
            rather than about this mode.
          </p>
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
                  {/* §6.5. The risk was on the wire and on the type and was the
                      one property of a tool a person needs before pinning it. */}
                  <RiskBadge value={t.risk} />
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
            {filtered.length === 0 && q.length > 0 && (
              <p className="sub">No tool this mode can see matches “{q}”.</p>
            )}
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

/**
 * The Cost tab.
 *
 * It renders the `usage` note the harness writes once per model call, which has
 * been on the conversation's own stream since the local driver was written. The
 * tab was passed `calls={[]}` and said **"No model call has been accounted
 * yet"** over a run that had made a dozen — the panel and the endpoint were both
 * fine and nothing joined them, and the sentence told the reader to stop
 * looking.
 *
 * Three fields the note does not carry stay absent rather than becoming zeros,
 * and each says so in its own words: the per-layer split (§3.5's four columns)
 * is emitted by nothing, the money needs a price table the platform does not
 * have (OQ-16), and a cache share needs a prompt total the provider may not
 * report.
 */
export function CostTab({
  calls,
  bytesPerTokenCalibrated,
  bytesPerTokenEstimate,
}: {
  /** `null` means nobody asked; `[]` means the run has made no model call. */
  calls: CallCost[] | null
  bytesPerTokenCalibrated: number | null
  bytesPerTokenEstimate: number | null
}) {
  if (calls === null) {
    return (
      <EmptyState
        message="This conversation's cost has not been read"
        detail="The tab renders the harness's per-call usage notes off the run's event stream. There is no run on this conversation to read them from yet."
      />
    )
  }
  if (calls.length === 0) {
    return (
      <EmptyState
        message="This run has made no model call yet"
        detail="One row appears per call, from the usage note the harness writes when the reply lands."
      />
    )
  }
  const priced = calls.filter((c) => c.usd_tenth_cents !== null)
  const cumulativeUsd = priced.reduce((a, c) => a + (c.usd_tenth_cents ?? 0), 0)
  // Summed over the calls that reported, and `null` when none did — a total of
  // `0` over calls that all answered "not reported" is the same lie one level up.
  const sum = (pick: (c: CallCost) => number | null) => {
    const seen = calls.map(pick).filter((v): v is number => v !== null)
    return seen.length === 0 ? null : seen.reduce((a, b) => a + b, 0)
  }
  const promptTotal = sum((c) => c.provider_prompt_tokens)
  const outputTotal = sum((c) => c.output_tokens)
  const withLayers = calls.filter((c) => c.layers !== null)
  const withCache = calls.filter((c) => c.cache_hit_pct !== null)
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--s-3)' }}>
      <section>
        <span className="lbl">cumulative · {calls.length} {calls.length === 1 ? 'call' : 'calls'}</span>
        <p className="mono">
          {promptTotal === null ? 'prompt not reported' : `${promptTotal.toLocaleString()} prompt`} ·{' '}
          {outputTotal === null ? 'output not reported' : `${outputTotal.toLocaleString()} output`}
        </p>
        {/* Not `$0.000`. A run with no price attached to it is unpriced, and a
            zero here is the single most reassuring wrong number this panel
            could print. */}
        {priced.length === 0 ? (
          <p className="sub" data-testid="cost-unpriced">
            not priced — the platform has no price table, so these calls have a token count and no
            dollar figure (OQ-16). This is not $0.
          </p>
        ) : (
          <p className="mono">${(cumulativeUsd / 1000).toFixed(3)}</p>
        )}
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
        {withCache.length === 0 ? (
          <p className="sub" data-testid="cache-unreported">
            no call reported a prompt total to take a cache share of, so the hit rate is unknown
            rather than nil.
          </p>
        ) : (
          <div
            style={{ display: 'flex', alignItems: 'flex-end', gap: 2, height: 32 }}
            aria-label="cache hit per call"
          >
            {calls.map((c, i) => (
              <Tooltip
                key={`${c.step}-${i}`}
                content={
                  c.cache_hit_pct === null
                    ? `step ${c.step}: not reported`
                    : `step ${c.step}: ${c.cache_hit_pct.toFixed(0)}%`
                }
              >
                <i
                  style={{
                    display: 'block',
                    width: 6,
                    // An unreported call is drawn as a hairline in the muted
                    // colour, never as a bar of height zero in the accent — a
                    // zero-height accent bar is "no cache hits", which is a
                    // claim.
                    height: c.cache_hit_pct === null ? '2px' : `${Math.max(2, c.cache_hit_pct * 0.3)}px`,
                    background: c.cache_hit_pct === null ? 'var(--fg-muted)' : 'var(--bg-accent)',
                  }}
                />
              </Tooltip>
            ))}
          </div>
        )}
      </section>
      <section>
        <span className="lbl">tokens by call</span>
        {withLayers.length === 0 && (
          <p className="sub" data-testid="layers-unsplit">
            the four-layer split (charter · schemas · body · output) is not on the wire — the usage
            note records one prompt total, so the columns below are what was measured. The split is
            unbuilt, not zero.
          </p>
        )}
        <table className="tbl">
          <thead>
            <tr>
              <th style={{ textAlign: 'left' }}>step</th>
              <th>provider prompt</th>
              <th>harness estimate</th>
              <th>cache read</th>
              <th>output</th>
            </tr>
          </thead>
          <tbody>
            {calls.map((c, i) => {
              // The estimate against the provider's count, which is the drift
              // §3.5 asks for. Absent on either side means no drift is known —
              // not a drift of zero.
              const drift =
                c.provider_prompt_tokens === null || c.estimated_prompt_tokens === null
                  ? null
                  : c.estimated_prompt_tokens - c.provider_prompt_tokens
              return (
                <tr key={`${c.step}-${i}`}>
                  <td style={{ textAlign: 'left' }}>{c.step}</td>
                  <td>{c.provider_prompt_tokens ?? 'not reported'}</td>
                  <td>
                    {c.estimated_prompt_tokens ?? 'not reported'}
                    {drift !== null && drift !== 0 ? ` (${drift > 0 ? '+' : ''}${drift})` : ''}
                  </td>
                  <td>{c.cache_read_tokens ?? 'not reported'}</td>
                  <td>{c.output_tokens ?? 'not reported'}</td>
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
  loading = false,
}: {
  items: AgendaItem[] | null
  onVeto?: (id: string) => void
  onApprove?: (id: string) => void
  onAdd?: () => void
  onOpenChild?: (conversationId: string) => void
  loading?: boolean
}) {
  if (loading) {
    return <EmptyState message="Reading the agenda…" />
  }
  if (items === null) {
    // Three states, not two: this tab is only reachable on a continuous session,
    // so a `null` here is the request failing rather than "no board". An empty
    // board is `[]` and renders the five columns with nothing in them, which is
    // a different sentence and has to look different.
    return (
      <EmptyState
        message="The agenda request did not answer"
        detail="/api/research/sessions/{id}/agenda is served, so this is the call failing rather than the board being unbuilt. The agenda is unknown, not empty."
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
                  {/* The true status, which the column alone does not say: `expired`
                      and a verdict both sit in `done`. Only shown when it differs
                      from the column, so the common case stays quiet. */}
                  {i.status && i.status !== i.state && <Badge tone="warn">{i.status}</Badge>}
                  {i.sources.map((s, n) => (
                    <Badge key={`${s}-${n}`} tone="neutral">
                      {s}
                    </Badge>
                  ))}
                  {i.exploration_pick && <Badge tone="info">exploration</Badge>}
                </div>
                <p>{i.question}</p>
                {i.score ? (
                  <p className="sub">
                    value {i.score.value} · novelty {i.score.novelty} · feasibility {i.score.feasibility} · cost{' '}
                    {i.score.cost} → {i.score.total}
                  </p>
                ) : (
                  // Not "score 0". An unscored item is one the scorer has not
                  // reached, and showing a zero would read as "worthless".
                  <p className="sub">
                    not scored yet
                    {i.bypasses_scoring ? ' — you asked for this one, so it goes next' : ''}
                  </p>
                )}
                {i.rationale && <p className="sub">{i.rationale}</p>}
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

/** One scope line, with the server's reading of an empty list. */
function ScopeLine({ label, values }: { label: string; values: string[] }) {
  return (
    <p className="sub" style={{ margin: 0 }}>
      {label}:{' '}
      {values.length > 0 ? (
        values.join(', ')
      ) : (
        // Not `—`. `Scope::admits` returns true for every subject when the list
        // is empty, so a charter naming no instruments admits all of them. The
        // dash said the opposite of what the session will do.
        <em>any — none named, and an unnamed scope admits everything</em>
      )}
    </p>
  )
}

export function CharterTab({
  charter,
  amendments,
  loading = false,
}: {
  /** Decoded by `charterView`; `null` when the session has none or was not read. */
  charter: CharterView | null
  /**
   * The amendment texts, when something can list them.
   *
   * `PUT …/{id}/charter` appends an amendment and bumps the version, and there
   * is no route that reads the amendments back — so this is `[]` from every
   * caller today, and the version is what the tab reports instead.
   */
  amendments: string[]
  loading?: boolean
}) {
  if (loading) {
    return <EmptyState message="Reading the charter…" />
  }
  if (!charter) {
    return (
      <EmptyState
        message="This conversation has no charter"
        detail="A charter belongs to a continuous session. Turn on Continuous in the composer to give one to a new session; an existing conversation cannot be given one after the fact."
      />
    )
  }
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--s-3)' }}>
      <section>
        <span className="lbl">
          objective{charter.version !== null ? ` · version ${charter.version}` : ''}
        </span>
        <p>{charter.objective}</p>
      </section>

      <section>
        <span className="lbl">scope</span>
        {charter.scope_is_any && (
          <p className="sub" data-testid="scope-any">
            Nothing is named, so this charter is in scope for everything.
          </p>
        )}
        <ScopeLine label="instruments" values={charter.scope.instruments} />
        <ScopeLine label="asset classes" values={charter.scope.asset_classes} />
        <ScopeLine label="families" values={charter.scope.families} />
        <ScopeLine label="timeframes" values={charter.scope.timeframes} />
      </section>

      <section>
        <span className="lbl">workflows</span>
        <p className="sub">
          {charter.workflows_are_all ? (
            // Same reading as the scope, for the same reason: `Charter::permits`
            // answers yes to everything when the list is empty.
            <span data-testid="workflows-all">all the workflows this tier supports — none were named</span>
          ) : (
            charter.permitted_workflows.join(' · ')
          )}
        </p>
        {charter.exclusions.length > 0 && (
          <p className="sub">excluded: {charter.exclusions.join(' · ')}</p>
        )}
      </section>

      <section>
        <span className="lbl">budget</span>
        <p className="sub">
          {charter.usd_per_day === null ? 'no day budget recorded' : `$${charter.usd_per_day}/day`}
          {charter.usd_total !== null && ` · $${charter.usd_total} total`}
          {charter.max_runs !== null && ` · ${charter.max_runs} runs`}
        </p>
        <p className="sub">
          {/* `trials_total` arrives as 0 for a blank form field, so a "0 trials"
              here would describe a session that may never run. */}
          {charter.trials_total === null
            ? 'no trial cap recorded — not a cap of zero'
            : `${charter.trials_total} trials`}
        </p>
      </section>

      <section>
        <span className="lbl">how it runs</span>
        <p className="sub">
          autonomy {charter.autonomy ?? 'not recorded'}
          {charter.max_concurrent_runs !== null && ` · ${charter.max_concurrent_runs} at a time`}
          {charter.exploration_share !== null &&
            ` · ${Math.round(charter.exploration_share * 100)}% exploration`}
        </p>
        <p className="sub">
          {charter.replenish === null
            ? 'replenish not recorded'
            : charter.replenish
              ? 'keeps generating questions'
              : 'stops when the agenda empties'}
          {charter.review_every &&
            (charter.review_every.kind === 'runs'
              ? ` · digest every ${charter.review_every.runs} runs`
              : ' · digest every window')}
        </p>
        <p className="sub">
          {charter.always_open ? (
            // `Schedule::default` is no windows, and no windows means always
            // open — the same empty-is-everything reading as the scope.
            <span data-testid="schedule-any">any time — no window was set</span>
          ) : (
            charter.windows.map((w) => `${w.days.join(',')} ${w.start}–${w.end} ${w.tz}`).join(' · ')
          )}
        </p>
        {charter.delta_practical !== null && (
          <p className="sub">
            delta_practical {charter.delta_practical} — a suggestion; each run seals its own
          </p>
        )}
      </section>

      {charter.standing_constraints.length > 0 && (
        <section>
          <span className="lbl">standing constraints</span>
          <ul style={{ margin: 'var(--s-1) 0 0', paddingLeft: 'var(--s-4)', fontSize: 'var(--t-12)' }}>
            {charter.standing_constraints.map((c) => (
              <li key={c}>{c}</li>
            ))}
          </ul>
        </section>
      )}

      {charter.seeds.length > 0 && (
        <section>
          <span className="lbl">seeds · {charter.seeds.length}</span>
          <ul style={{ margin: 'var(--s-1) 0 0', paddingLeft: 'var(--s-4)', fontSize: 'var(--t-12)' }}>
            {charter.seeds.map((s, i) => (
              <li key={i}>{s}</li>
            ))}
          </ul>
        </section>
      )}

      <section>
        <span className="lbl">amendments</span>
        {amendments.length > 0 ? (
          <ol style={{ paddingLeft: 'var(--s-4)', fontSize: 'var(--t-12)' }}>
            {amendments.map((a, i) => (
              <li key={i}>{a}</li>
            ))}
          </ol>
        ) : charter.version !== null && charter.version > 1 ? (
          // The version says there were some. Saying "none" under a version 3
          // charter would be the panel contradicting the number above it.
          <p className="sub" data-testid="amendments-unlistable">
            this charter is at version {charter.version}, so it has been amended{' '}
            {charter.version - 1} {charter.version === 2 ? 'time' : 'times'} — and no route reads
            the amendments back, so the desk cannot show what changed.
          </p>
        ) : (
          <p className="sub">none</p>
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
