// The right-hand tabbed panes (COMP-006 §3).
//
// Jobs, Notebook and Report are the three that have data behind them today.
// Artifacts and Experiments are listed but marked as what they are: the Experiments
// pane embeds the Set J workbench (Set K consolidation), and the Artifacts pane
// needs the lineage viewer. A tab that renders plausible-looking empty state for a
// thing that does not exist is worse than one that says so — the first time anyone
// finds out is when they trust it.

import { useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { Loader2, ExternalLink } from 'lucide-react'
import { researchApi, type Job } from '@/api/research'

// ── Jobs (COMP-005 §6) ──────────────────────────────────────────────────────

const TERMINAL = new Set(['succeeded', 'failed', 'cancelled'])

function JobRow({ job }: { job: Job }) {
  const pct = job.progress?.pct
  return (
    <div className="rounded-lg border border-border bg-surface px-3 py-2">
      <div className="flex items-center gap-2">
        <span className="font-mono text-[11px] text-text-muted">{job.kind}</span>
        <span
          className={`rounded px-1.5 py-0.5 text-[10px] uppercase tracking-wide ${
            job.state === 'failed'
              ? 'bg-pnl-down/15 text-pnl-down'
              : job.state === 'succeeded'
                ? 'bg-pnl-up/15 text-pnl-up'
                : 'bg-surface-2 text-text-dim'
          }`}
        >
          {job.state}
        </span>
        {!TERMINAL.has(job.state) && (
          <Loader2 className="h-3 w-3 animate-spin text-text-dim" />
        )}
        <span className="ml-auto font-mono text-[10px] text-text-dim">{job.job_id}</span>
      </div>
      {typeof pct === 'number' && !TERMINAL.has(job.state) && (
        <div className="mt-1.5 h-1 overflow-hidden rounded bg-surface-2">
          <div
            className="h-full bg-accent transition-all"
            style={{ width: `${Math.round(pct * 100)}%` }}
          />
        </div>
      )}
      {job.result_summary && (
        <p className="mt-1 text-[11px] text-text-dim">{job.result_summary}</p>
      )}
      {/* A failed job shows its `fix`, not just its code. A failure a reader cannot
          act on becomes a shrug. */}
      {job.error && (
        <p className="mt-1 text-[11px] text-pnl-down">
          {job.error.code}
          {job.error.fix ? ` — ${job.error.fix}` : ''}
        </p>
      )}
    </div>
  )
}

export function JobsPane({ projectId }: { projectId?: string }) {
  const { data: jobs = [], isLoading } = useQuery({
    queryKey: ['research', 'jobs', projectId ?? null],
    queryFn: async () => (await researchApi.jobs(projectId)).data.jobs ?? [],
    refetchInterval: 5_000,
  })

  if (isLoading)
    return <div className="text-xs text-text-dim">Loading jobs…</div>
  if (jobs.length === 0)
    return <div className="py-6 text-center text-xs text-text-dim">No jobs yet.</div>
  return (
    <div className="flex flex-col gap-1.5">
      {jobs.map((j) => (
        <JobRow key={j.job_id} job={j} />
      ))}
    </div>
  )
}

// ── Notebook (UI-04) ────────────────────────────────────────────────────────

const NOTEBOOK_FILES = ['NOTEBOOK.md', 'RESEARCH_PLAN.json', 'GIT_LOG.txt'] as const

export function NotebookPane({ projectId }: { projectId: string }) {
  const [file, setFile] = useState<(typeof NOTEBOOK_FILES)[number]>('NOTEBOOK.md')

  // Keyed on (project, file), so switching files does not need an effect to clear
  // the previous contents: the query key changes and react-query hands back the
  // right data or nothing. Clearing state inside an effect is how a pane ends up
  // briefly showing one file's text under another file's tab.
  const { data, isLoading } = useQuery({
    queryKey: ['research', 'workspace-file', projectId, file],
    queryFn: async () => {
      try {
        const { data } = await researchApi.workspaceFile(projectId, file)
        return { content: data.content, handle: data.handle }
      } catch (e) {
        // The server sends a `fix` on the empty case. Surfacing it is the difference
        // between "broken" and "nothing has happened yet" - and a non-JSON body
        // (a bare 401, say) must still produce a readable message rather than an
        // empty pane.
        const body = (e as { response?: { data?: { fix?: string; message?: string } } })
          .response?.data
        return {
          message: body?.fix ?? body?.message ?? 'could not read that file',
        }
      }
    },
    retry: false,
  })

  const content = data && 'content' in data ? data.content : null
  const handle = data && 'handle' in data ? data.handle : null
  const message = isLoading
    ? 'Loading…'
    : data && 'message' in data
      ? data.message
      : null

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-2">
      <div className="flex gap-1">
        {NOTEBOOK_FILES.map((f) => (
          <button
            key={f}
            type="button"
            onClick={() => setFile(f)}
            className={`rounded px-2 py-1 font-mono text-[11px] ${
              f === file ? 'bg-surface-2 text-text' : 'text-text-dim hover:text-text'
            }`}
          >
            {f}
          </button>
        ))}
        {handle && content !== null && (
          <span
            className="ml-auto self-center font-mono text-[10px] text-text-dim"
            title="The content-addressed snapshot this pane is showing. The plan as it was, not as it is now."
          >
            {handle}
          </span>
        )}
      </div>
      {message && <div className="text-xs text-text-dim">{message}</div>}
      {content !== null && (
        <pre className="min-h-0 flex-1 overflow-auto whitespace-pre-wrap rounded-lg border border-border bg-surface p-3 text-[11px] leading-relaxed text-text">
          {content}
        </pre>
      )}
    </div>
  )
}

// ── Report (UI-08) ──────────────────────────────────────────────────────────

interface Claim {
  text: string
  value?: number | null
  unit?: string | null
  evidence?: string[]
  class?: string
}

interface FinalReport {
  answer: string
  outcome: string
  claims?: Claim[]
  caveats?: string[]
  exploration_ledger_ref?: string | null
  candidates?: Array<{
    strategy_ref: string
    experiment_id: string
    gate_reached: string
    trials?: number
    dossier_ref?: string | null
  }>
}

/**
 * UI-08: every claim links to its evidence.
 *
 * A claim with a number and no link is rendered as a warning rather than as text.
 * The report validator refuses to accept one, so it should not be possible — and if
 * it ever is, the reader must see that this figure is unsupported rather than read
 * it as a measurement.
 */
function ClaimRow({ claim }: { claim: Claim }) {
  const evidence = claim.evidence ?? []
  const unsupported = claim.value !== null && claim.value !== undefined && evidence.length === 0
  return (
    <li className="border-b border-border py-2 last:border-0">
      <div className="flex items-baseline gap-2">
        <span className="flex-1 text-sm text-text">{claim.text}</span>
        {claim.value !== null && claim.value !== undefined && (
          <span className="font-mono text-sm text-text">
            {claim.value}
            {claim.unit ? ` ${claim.unit}` : ''}
          </span>
        )}
      </div>
      <div className="mt-1 flex flex-wrap items-center gap-1.5">
        {claim.class && (
          <span className="rounded bg-surface-2 px-1.5 py-0.5 text-[10px] uppercase tracking-wide text-text-dim">
            {claim.class}
          </span>
        )}
        {evidence.map((h) => (
          <a
            key={h}
            href={`/api/artifacts/${h}`}
            target="_blank"
            rel="noreferrer"
            className="flex items-center gap-1 font-mono text-[10px] text-text-muted hover:text-accent"
          >
            {h}
            <ExternalLink className="h-2.5 w-2.5" />
          </a>
        ))}
        {unsupported && (
          <span className="text-[10px] text-pnl-down">
            this number cites nothing — it is not evidence
          </span>
        )}
      </div>
    </li>
  )
}

export function ReportPane({ reportId }: { reportId?: string | null }) {
  const { data, isLoading } = useQuery({
    queryKey: ['research', 'report', reportId ?? null],
    // `enabled` rather than an effect branch: a session with no report is a normal
    // state, not a failed fetch, and the query simply does not run.
    enabled: !!reportId,
    retry: false,
    queryFn: async () => {
      try {
        const { data } = await researchApi.report(reportId as string)
        return { report: data as FinalReport }
      } catch (e) {
        const body = (e as { response?: { data?: { message?: string } } }).response?.data
        return { message: body?.message ?? 'could not read the report' }
      }
    },
  })

  const report = data && 'report' in data ? data.report : null
  const message = !reportId
    ? 'This session has not filed a report.'
    : isLoading
      ? null
      : data && 'message' in data
        ? data.message
        : null

  if (message) return <div className="py-6 text-center text-xs text-text-dim">{message}</div>
  if (!report) return <div className="text-xs text-text-dim">Loading report…</div>

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto">
      <div>
        <div className="mb-1 text-[10px] uppercase tracking-wide text-text-dim">
          {report.outcome}
        </div>
        <p className="whitespace-pre-wrap text-sm text-text">{report.answer}</p>
      </div>

      {(report.claims ?? []).length > 0 && (
        <div>
          <h4 className="mb-1 text-[11px] font-semibold uppercase tracking-wide text-text-dim">
            Claims
          </h4>
          <ul className="rounded-lg border border-border bg-surface px-3">
            {(report.claims ?? []).map((c, i) => (
              <ClaimRow key={i} claim={c} />
            ))}
          </ul>
        </div>
      )}

      {(report.candidates ?? []).length > 0 && (
        <div>
          <h4 className="mb-1 text-[11px] font-semibold uppercase tracking-wide text-text-dim">
            Candidates
          </h4>
          <div className="flex flex-col gap-1.5">
            {(report.candidates ?? []).map((c) => (
              <div
                key={c.experiment_id}
                className="flex items-center gap-2 rounded-lg border border-border bg-surface px-3 py-2 text-xs"
              >
                <span className="font-mono text-text">{c.strategy_ref}</span>
                <span className="rounded bg-surface-2 px-1.5 py-0.5 text-[10px] text-text-dim">
                  {c.gate_reached}
                </span>
                {/* The trial count is the denominator of any significance claim
                    (INV-3). Shown next to the gate, because a G3 on 400 trials and a
                    G3 on 4 are not the same statement. */}
                <span className="text-[11px] text-text-dim">
                  {c.trials ?? 0} trials
                </span>
                {c.dossier_ref && (
                  <a
                    href={`/api/artifacts/${c.dossier_ref}`}
                    target="_blank"
                    rel="noreferrer"
                    className="ml-auto font-mono text-[10px] text-text-muted hover:text-accent"
                  >
                    dossier
                  </a>
                )}
              </div>
            ))}
          </div>
        </div>
      )}

      {(report.caveats ?? []).length > 0 && (
        <div>
          <h4 className="mb-1 text-[11px] font-semibold uppercase tracking-wide text-text-dim">
            Caveats
          </h4>
          <ul className="list-inside list-disc text-xs text-text-muted">
            {(report.caveats ?? []).map((c, i) => (
              <li key={i}>{c}</li>
            ))}
          </ul>
        </div>
      )}

      {report.exploration_ledger_ref && (
        <a
          href={`/api/artifacts/${report.exploration_ledger_ref}`}
          target="_blank"
          rel="noreferrer"
          className="text-[11px] text-text-muted hover:text-accent"
          title="How much searching preceded this verdict. A report without it is missing its denominator."
        >
          Exploration ledger: {report.exploration_ledger_ref}
        </a>
      )}
    </div>
  )
}

// ── Not yet built ───────────────────────────────────────────────────────────

export function PendingPane({ name, needs }: { name: string; needs: string }) {
  return (
    <div className="py-8 text-center text-xs text-text-dim">
      <p className="mb-1 text-text-muted">{name} is not built yet.</p>
      <p>{needs}</p>
    </div>
  )
}
