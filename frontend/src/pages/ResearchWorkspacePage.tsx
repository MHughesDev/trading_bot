// The research workspace (COMP-006 §2, UI-01).
//
// **One layout, no modes.** A Desk question and an overnight campaign are the same
// screen. The alternative — a "quick ask" mode and a "campaign" mode — would mean two
// code paths that drift, and a user who learns one and is surprised by the other.
//
// Three columns: projects and budget on the left, the conversation in the middle,
// tabbed panes on the right. On a narrow screen the outer two collapse behind
// toggles rather than reflowing into a single scroll, because the middle column is
// the one being read and the other two are reference.

import { useMemo, useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { useNavigate, useParams } from 'react-router-dom'
import { FlaskConical, PanelLeft, PanelRight, Lock } from 'lucide-react'
import { researchApi, type Project } from '@/api/research'
import { Timeline } from '@/components/workspace/Timeline'
import { SteeringInput } from '@/components/workspace/SteeringInput'
import { BudgetMeter } from '@/components/workspace/BudgetMeter'
import { ApprovalsInbox } from '@/components/workspace/ApprovalsInbox'
import {
  JobsPane,
  NotebookPane,
  PendingPane,
  ReportPane,
} from '@/components/workspace/Panes'

const PANES = [
  'Jobs',
  'Notebook',
  'Report',
  'Approvals',
  'Artifacts',
  'Experiments',
] as const
type Pane = (typeof PANES)[number]

/** Session states that can still receive steering. Mirrors the API's list. */
const STEERABLE = new Set(['starting', 'running', 'waiting', 'compacting'])

function cutoffLabel(project: Project): string {
  if (project.kind === 'desk') return 'Desk - live data, exploratory only'
  return project.research_cutoff
    ? `Cutoff ${project.research_cutoff.slice(0, 10)}`
    : 'Research project'
}

export function ResearchWorkspacePage() {
  const { projectId, sessionId } = useParams<{
    projectId?: string
    sessionId?: string
  }>()
  const navigate = useNavigate()

  const [pane, setPane] = useState<Pane>('Jobs')
  const [showLeft, setShowLeft] = useState(true)
  const [showRight, setShowRight] = useState(true)

  // react-query rather than an effect-plus-interval. Besides being what the rest of
  // the app uses, it keeps the previous data on screen while a refetch is in
  // flight: a session list that blanks every eight seconds is unusable, and a
  // hand-rolled poll has to remember not to do that.
  const { data: projects = [] } = useQuery({
    queryKey: ['research', 'projects'],
    queryFn: async () => (await researchApi.projects()).data.projects,
  })
  const { data: sessions = [] } = useQuery({
    queryKey: ['research', 'sessions', projectId ?? null],
    queryFn: async () => (await researchApi.sessions(projectId)).data.sessions,
    refetchInterval: 8_000,
  })

  const session = useMemo(
    () => sessions.find((s) => s.session_id === sessionId) ?? null,
    [sessions, sessionId],
  )
  const project = useMemo(
    () => projects.find((p) => p.project_id === projectId) ?? null,
    [projects, projectId],
  )

  return (
    <div className="flex h-full min-h-0 gap-3 p-3">
      {/* ── Left: projects, sessions, budget ── */}
      {showLeft && (
        <aside className="flex w-72 shrink-0 flex-col gap-3 overflow-y-auto">
          <div>
            <h2 className="mb-1.5 px-1 text-[11px] font-semibold uppercase tracking-wide text-text-dim">
              Projects
            </h2>
            <div className="flex flex-col gap-1">
              {projects.map((p) => (
                <button
                  key={p.project_id}
                  type="button"
                  onClick={() => navigate(`/research/${p.project_id}`)}
                  className={`rounded-lg border px-2.5 py-2 text-left ${
                    p.project_id === projectId
                      ? 'border-accent bg-surface'
                      : 'border-border bg-surface hover:border-border-2'
                  }`}
                >
                  <div className="flex items-center gap-1.5">
                    <span className="truncate text-xs font-medium text-text">
                      {p.name}
                    </span>
                    {/* The cutoff is the most important fact about a project, so it
                        is on the card rather than a detail page. A researcher who
                        forgets which project they are in is the person the holdout
                        exists to protect. */}
                    {p.kind === 'research' && (
                      <Lock className="h-3 w-3 shrink-0 text-text-dim" />
                    )}
                  </div>
                  <div className="mt-0.5 text-[10px] text-text-dim">
                    {cutoffLabel(p)}
                  </div>
                </button>
              ))}
              {projects.length === 0 && (
                <p className="px-1 text-[11px] text-text-dim">No projects yet.</p>
              )}
            </div>
          </div>

          <div>
            <h2 className="mb-1.5 px-1 text-[11px] font-semibold uppercase tracking-wide text-text-dim">
              Sessions
            </h2>
            <div className="flex flex-col gap-1">
              {sessions.map((s) => (
                <button
                  key={s.session_id}
                  type="button"
                  onClick={() =>
                    navigate(`/research/${s.project_id}/sessions/${s.session_id}`)
                  }
                  className={`flex items-center gap-2 rounded-lg border px-2.5 py-1.5 text-left ${
                    s.session_id === sessionId
                      ? 'border-accent bg-surface'
                      : 'border-border bg-surface hover:border-border-2'
                  }`}
                >
                  <span
                    className={`h-1.5 w-1.5 shrink-0 rounded-full ${
                      STEERABLE.has(s.state)
                        ? 'bg-accent'
                        : s.state === 'failed'
                          ? 'bg-pnl-down'
                          : 'bg-text-dim'
                    }`}
                  />
                  <span className="min-w-0 flex-1 truncate font-mono text-[11px] text-text-muted">
                    {s.session_id.slice(0, 8)}
                  </span>
                  <span className="text-[10px] text-text-dim">
                    ${s.spend_usd.toFixed(2)}
                  </span>
                </button>
              ))}
              {sessions.length === 0 && (
                <p className="px-1 text-[11px] text-text-dim">No sessions yet.</p>
              )}
            </div>
          </div>

          <BudgetMeter projectId={projectId} sessionId={sessionId} />
        </aside>
      )}

      {/* ── Centre: the conversation ── */}
      <main className="flex min-w-0 flex-1 flex-col gap-2 rounded-xl border border-border bg-background p-3">
        <header className="flex items-center gap-2">
          <button
            type="button"
            onClick={() => setShowLeft((v) => !v)}
            className="text-text-dim hover:text-text"
            title="Toggle the project rail"
          >
            <PanelLeft className="h-4 w-4" />
          </button>
          <FlaskConical className="h-4 w-4 text-accent" />
          <h1 className="truncate text-sm font-semibold text-text">
            {project ? project.name : 'Research'}
          </h1>
          {session && (
            <span className="rounded bg-surface-2 px-1.5 py-0.5 text-[10px] uppercase tracking-wide text-text-dim">
              {session.state}
            </span>
          )}
          <button
            type="button"
            onClick={() => setShowRight((v) => !v)}
            className="ml-auto text-text-dim hover:text-text"
            title="Toggle the panes"
          >
            <PanelRight className="h-4 w-4" />
          </button>
        </header>

        {sessionId ? (
          <>
            <Timeline sessionId={sessionId} />
            <SteeringInput
              sessionId={sessionId}
              steerable={!!session && STEERABLE.has(session.state)}
            />
          </>
        ) : (
          <div className="flex flex-1 items-center justify-center text-sm text-text-dim">
            {projectId
              ? 'Pick a session to watch, or start one.'
              : 'Pick a project to begin.'}
          </div>
        )}
      </main>

      {/* ── Right: panes ── */}
      {showRight && (
        <aside className="flex w-96 shrink-0 flex-col gap-2 rounded-xl border border-border bg-background p-3">
          <nav className="flex flex-wrap gap-1">
            {PANES.map((p) => (
              <button
                key={p}
                type="button"
                onClick={() => setPane(p)}
                className={`rounded px-2 py-1 text-[11px] ${
                  p === pane
                    ? 'bg-surface-2 text-text'
                    : 'text-text-dim hover:text-text'
                }`}
              >
                {p}
              </button>
            ))}
          </nav>
          <div className="flex min-h-0 flex-1 flex-col overflow-y-auto">
            {pane === 'Jobs' && <JobsPane projectId={projectId} />}
            {pane === 'Notebook' &&
              (projectId ? (
                <NotebookPane projectId={projectId} />
              ) : (
                <PendingPane
                  name="Notebook"
                  needs="Pick a project to read its notebook."
                />
              ))}
            {pane === 'Report' && <ReportPane reportId={session?.report_id} />}
            {pane === 'Approvals' && <ApprovalsInbox projectId={projectId} />}
            {pane === 'Artifacts' && (
              <PendingPane
                name="The artifacts browser"
                needs="Needs the lineage viewer and the Parquet preview (COMP-005 §8). Artifact handles in the Report pane already link to the raw manifest."
              />
            )}
            {pane === 'Experiments' && (
              <PendingPane
                name="The experiments workbench"
                needs="Embeds the Set J funnel, sealed distributions and gate ledger once the Set K workbench consolidation lands."
              />
            )}
          </div>
        </aside>
      )}
    </div>
  )
}
