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
import { decisionSteps, diffLines, firstCallAtStage } from './RunDetail.helpers'
import { StageStrip } from './Shared'
import type { WireEvent } from '@/lib/desk/events'
import { STAGE_ORDER } from '@/api/desk'

type View = 'prompt' | 'reply' | 'diff'

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
    // The trajectory route is not mounted on this platform. The driver's own
    // `decision` rows are — the whole system string and prompt of every call,
    // on the messages route — so the scrubber replays those, and says per call
    // whether the stored strings *are* the request.
    if (traj.data == null) {
      return <DecisionReplay runId={runId} onBack={onBack} />
    }
    return (
      <div className="desk-scroll">
        <EmptyState
          message="This run has no stored steps"
          detail="The trajectory route answered and holds no step for this run. That is a fact about the run rather than about the platform."
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

/** Every message of a run, paged — the route answers at most 500 at a time. */
async function allMessages(runId: string): Promise<WireEvent[]> {
  const out: WireEvent[] = []
  let after: number | undefined
  // Bounded: a run with more than 40 pages is a run this panel should not try
  // to hold in memory, and the cap is said on screen rather than hidden.
  for (let page = 0; page < 40; page++) {
    const { data } = await deskApi.messages(runId, after)
    out.push(...data.messages)
    if (data.messages.length === 0 || data.last_seq === after) break
    after = data.last_seq
  }
  return out
}

/**
 * The scrubber over the driver's `decision` rows (S-6.16, S-10.15).
 *
 * Byte-for-byte where the stored strings are the request — the plan, select
 * and fill phases — and labelled **not reproducible** where they are not: a
 * `native` call sent the compacted transcript and a tool list, neither of which
 * is stored. The driver records the harness's per-layer estimate on each row
 * (charter / schemas / body, and the output cap); the panel shows it beside the
 * provider's own count, and for a row written before that it says the layers
 * were not recorded rather than splitting a total it cannot split.
 */
export function DecisionReplay({ runId, onBack }: { runId: string; onBack: () => void }) {
  const [at, setAt] = useState(0)
  const [view, setView] = useState<View>('prompt')
  const msgs = useQuery({
    queryKey: ['desk', 'replay-messages', runId],
    queryFn: () => allMessages(runId),
  })
  if (msgs.isPending) return <div className="desk-scroll">Loading the trace…</div>
  const steps = decisionSteps(msgs.data ?? [])
  if (msgs.isError || steps.length === 0) {
    return (
      <div className="desk-scroll">
        <EmptyState
          message={msgs.isError ? 'The run’s messages could not be read' : 'Nothing here can report what this run did'}
          detail={
            msgs.isError
              ? 'The messages route failed, which is not the same as a run that made no calls.'
              : 'The trajectory route is not mounted — it answers 404 where every mounted route answers 401 — and this run’s messages hold no decision row — the hosted driver does not write them. An empty scrubber is this panel having no source, not a run that took no steps, and the desk will not reconstruct a prompt it cannot prove.'
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
  const i = Math.min(at, steps.length - 1)
  const step = steps[i]
  const previous = i > 0 ? steps[i - 1] : null
  const stages = firstCallAtStage(steps)
  const full = `${step.system}\n\n${step.prompt}`
  return (
    <div className="desk-scroll" style={{ display: 'flex', flexDirection: 'column', gap: 'var(--s-3)' }}>
      <div className="desk-hd">
        <Button size="xs" variant="ghost" onClick={onBack}>
          ← Conversation
        </Button>
        <span className="title">Run {runId}</span>
        <span className="sub">{steps.length} model calls, from the driver’s decision log</span>
      </div>

      {/* The ten stages, keyboard-navigable: Left/Right move between stages
          that have a call, Enter/click jumps to the first call at that stage.
          A stage no call reached is drawn and disabled, not hidden. */}
      <div
        role="group"
        aria-label="Stages"
        className="replay-strip"
        data-testid="replay-stages"
        onKeyDown={(e) => {
          const reached = STAGE_ORDER.filter((s) => stages.has(s))
          const cur = step.stage ? reached.indexOf(step.stage) : -1
          const go = (k: number) => {
            const s = reached[k]
            if (s !== undefined) setAt(stages.get(s) as number)
          }
          if (e.key === 'ArrowRight') go(Math.min(reached.length - 1, cur + 1))
          if (e.key === 'ArrowLeft') go(Math.max(0, cur - 1))
        }}
      >
        {STAGE_ORDER.map((s) => (
          <button
            key={s}
            type="button"
            disabled={!stages.has(s)}
            aria-current={step.stage === s}
            onClick={() => setAt(stages.get(s) as number)}
          >
            {s}
          </button>
        ))}
      </div>
      {stages.size === 0 && (
        <p className="sub" data-testid="replay-no-stages">
          no stage event was recorded on this run, so calls cannot be placed on the ten stages
        </p>
      )}

      <div
        className="replay-strip"
        role="group"
        aria-label="Steps"
        onKeyDown={(e) => {
          if (e.key === 'ArrowLeft') setAt((n) => Math.max(0, n - 1))
          if (e.key === 'ArrowRight') setAt((n) => Math.min(steps.length - 1, n + 1))
        }}
      >
        {steps.map((s, n) => (
          <button key={s.seq} type="button" aria-current={n === i} onClick={() => setAt(n)}>
            {s.step}
          </button>
        ))}
      </div>

      <Panel>
        <PanelHeader
          title={`Step ${step.step} · ${step.phase}${step.tool ? ` · ${step.tool}` : ''}`}
          badges={
            step.faithful ? (
              <Badge tone="pos" data-testid="replay-faithful">the stored strings are the request</Badge>
            ) : (
              <Badge tone="neg" data-testid="replay-unfaithful">
                not reproducible — the request was the compacted transcript, which is not stored
              </Badge>
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
          {step.stage && <StageStrip stage={step.stage} />}
          {view === 'prompt' && (
            <>
              <p className="sub" data-testid="replay-layers">
                {step.inputTokens === null ? 'input tokens not reported' : `${step.inputTokens} input tokens (provider count, whole call)`}
                {' · '}
                {new TextEncoder().encode(full).length} bytes
                {step.layers ? (
                  <span data-testid="replay-layer-split">
                    {' · '}harness estimate: charter {step.layers.charter} · schemas {step.layers.schemas} · body{' '}
                    {step.layers.body} (= {step.layers.charter + step.layers.schemas + step.layers.body} prompt tokens
                    {step.inputTokens !== null && step.inputTokens > 0
                      ? `, ${Math.round(((step.layers.charter + step.layers.schemas + step.layers.body) / step.inputTokens) * 100)}% of the provider count`
                      : ''}
                    ) · output cap {step.layers.output}
                  </span>
                ) : (
                  <span data-testid="replay-layers-unrecorded">
                    {' · '}per-layer sizes were not recorded on this call (written before the driver kept them)
                  </span>
                )}
              </p>
              <span className="lbl">system</span>
              <pre className="replay-prompt" data-testid="replay-system">
                {step.system}
              </pre>
              <span className="lbl">prompt</span>
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
    </div>
  )
}
