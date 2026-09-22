// Run detail and the replay scrubber (AGENT-012 §4; task S-6.16).
//
// The acceptance (§19.4) is the whole design: **the scrubber reproduces the
// prompt the model saw at step N byte-for-byte**. Not approximately — the point
// is that a human can see exactly what the model saw, which is the only way to
// debug a reasoning failure rather than guess at one.
//
// So the prompt is rendered from the stored string in a `<pre>` with `white-space:
// pre`, and the layer table is a *view* over it. `assemblePrompt` exists to let
// the UI check its own claim: when the layers do not reassemble to the stored
// prompt, the panel says so instead of showing a plausible reconstruction.
import { useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { Badge } from '@/components/primitives/Badge'
import { Button } from '@/components/primitives/Button'
import { Panel, PanelBody, PanelHeader } from '@/components/primitives/Panel'
import { Segmented } from '@/components/primitives/Segmented'
import { EmptyState } from '@/components/primitives/States'
import type { TrajectoryStep } from '@/api/desk'
import { deskApi, promptIsFaithful } from '@/api/desk'
import { StateTab } from './RightPanel'

type View = 'prompt' | 'reply' | 'diff'

/** A line diff, enough to see what changed between two steps' prompts. */
export function diffLines(previous: string, current: string): Array<{ sign: '+' | '-' | ' '; text: string }> {
  const a = previous.split('\n')
  const b = current.split('\n')
  const out: Array<{ sign: '+' | '-' | ' '; text: string }> = []
  const seen = new Set(a)
  const kept = new Set(b)
  for (const line of a) if (!kept.has(line)) out.push({ sign: '-', text: line })
  for (const line of b) out.push({ sign: seen.has(line) ? ' ' : '+', text: line })
  return out
}

export function RunDetail({ runId, onBack }: { runId: string; onBack: () => void }) {
  const [at, setAt] = useState(0)
  const [view, setView] = useState<View>('prompt')

  const traj = useQuery({
    queryKey: ['desk', 'trajectory', runId],
    queryFn: () => deskApi.trajectory(runId),
  })

  if (traj.isLoading) return <div className="desk-scroll">Loading the trace…</div>
  const steps: TrajectoryStep[] = traj.data?.steps ?? []
  if (steps.length === 0) {
    // **Two absences, and only one of them is about the run.**
    //
    // `optional()` answers `null` for a 404 and the decoded body for anything
    // else, so `traj.data === null` is "the route is not mounted" and
    // `traj.data` with an empty `steps` is "the route answered and this run has
    // no stored steps". The panel printed *"No trajectory is stored for this
    // run"* over both — a claim about the run, from a component with no source
    // for it.
    //
    // Verified against the live platform on 2026-09-22:
    // `GET /api/agent/runs/{id}/trajectory` answers **404** unauthenticated,
    // where every mounted route on :7080 answers 401. So on this deployment the
    // first branch is the only one that renders, and the sentence it replaces
    // was wrong on every run.
    const unmounted = traj.data == null
    return (
      <div className="desk-scroll">
        <EmptyState
          message={unmounted ? 'Nothing here can report what this run did' : 'This run has no stored steps'}
          detail={
            unmounted
              ? 'The scrubber reads `agent_events` through GET /api/agent/runs/{id}/trajectory, which AGENT-010 §5 adds and this platform does not mount — it answers 404 where every mounted route answers 401. An empty scrubber is this panel having no source, not a run that took no steps, and the desk will not reconstruct a prompt it cannot prove.'
              : 'The trajectory route answered and holds no step for this run. That is a fact about the run rather than about the platform.'
          }
          action={
            <Button size="sm" onClick={onBack}>
              Back to the conversation
            </Button>
          }
        />
      </div>
    )
  }

  const step = steps[Math.min(at, steps.length - 1)]
  const faithful = promptIsFaithful(step)
  const previous = at > 0 ? steps[at - 1] : null

  return (
    <div className="desk-scroll" style={{ display: 'flex', flexDirection: 'column', gap: 'var(--s-3)' }}>
      <div className="desk-hd">
        <Button size="xs" variant="ghost" onClick={onBack}>
          ← Conversation
        </Button>
        <span className="title">Run {runId}</span>
        {/* **Save as case was deleted, not wired.** Its whole implementation
            was `window.alert('Trajectory written to evals/ …')` — a dialog that
            reported a file had been written by a handler that wrote nothing and
            called nothing. There is no route to call: `GET /api/evals` and
            `POST /api/evals/cases` both answer 404, and so does
            `GET /api/agent/runs/{id}/trajectory`, the read this page is built
            on, which is why the empty state below is the only thing this screen
            renders on the live platform. A confirmation of work that did not
            happen is the one failure worse than a button that does nothing,
            because the person stops looking for the file.
            Asserted by `nothing_here_claims_to_have_written_a_file`. */}
      </div>

      <div
        className="replay-strip"
        role="group"
        aria-label="Steps"
        onKeyDown={(e) => {
          if (e.key === 'ArrowLeft') setAt((i) => Math.max(0, i - 1))
          if (e.key === 'ArrowRight') setAt((i) => Math.min(steps.length - 1, i + 1))
        }}
      >
        {steps.map((s, i) => (
          <button key={s.step} type="button" aria-current={i === at} onClick={() => setAt(i)}>
            {s.step}
          </button>
        ))}
      </div>

      <Panel>
        <PanelHeader
          title={`Step ${step.step}`}
          badges={
            faithful ? (
              <Badge tone="pos">layers reassemble byte-for-byte</Badge>
            ) : (
              <Badge tone="neg">layers do not reassemble — showing the stored prompt</Badge>
            )
          }
          actions={
            <Segmented<View>
              value={view}
              onChange={setView}
              ariaLabel="View"
              options={[
                { value: 'prompt', label: 'prompt' },
                { value: 'reply', label: 'reply' },
                { value: 'diff', label: 'diff' },
              ]}
            />
          }
        />
        <PanelBody>
          {view === 'prompt' && (
            <>
              <table className="tbl">
                <thead>
                  <tr>
                    <th style={{ textAlign: 'left' }}>layer</th>
                    <th>tokens</th>
                    <th>bytes</th>
                  </tr>
                </thead>
                <tbody>
                  {step.sections.map((s) => (
                    <tr key={s.layer}>
                      <td style={{ textAlign: 'left' }}>{s.layer}</td>
                      <td>{s.tokens}</td>
                      <td>{new TextEncoder().encode(s.text).length}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
              {/* The stored string, untouched. Nothing here trims, wraps or
                  re-indents it — that is what byte-for-byte means. */}
              <pre className="replay-prompt" data-testid="replay-prompt">
                {step.prompt}
              </pre>
            </>
          )}
          {view === 'reply' && (
            <pre className="replay-prompt" data-testid="replay-reply">
              {step.reply}
            </pre>
          )}
          {view === 'diff' && (
            <pre className="replay-prompt" data-testid="replay-diff">
              {previous
                ? diffLines(previous.prompt, step.prompt)
                    .map((l) => `${l.sign}${l.text}`)
                    .join('\n')
                : 'This is the first step; there is nothing to diff against.'}
            </pre>
          )}
        </PanelBody>
      </Panel>

      <Panel>
        <PanelHeader title="Working state, as it was at this step" />
        <PanelBody>
          <StateTab state={step.state} />
        </PanelBody>
      </Panel>
    </div>
  )
}
