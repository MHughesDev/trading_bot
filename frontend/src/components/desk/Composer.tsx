// The composer (AGENT-012 §3.3 as amended, §2b; ADR-0037; task S-6.7).
//
// The product decision this file carries, and must not soften:
//
//   The user sees exactly two options — **Chat** and **Research** — with a
//   **Continuous** toggle on Research. Not three. Continuous swaps the question
//   field for a charter form and turns on the agenda, the leases, the schedule,
//   the stall detector and the digest. It changes nothing about rings, lifecycle
//   or gates.
//
// Internally both are one object, `ResearchSession { continuous }`, and a bounded
// Research run is the one-item case. That is why toggling does not discard what
// was typed: there is one session being described, in two levels of detail.
import { forwardRef, useImperativeHandle, useRef, useState } from 'react'
import { Badge } from '@/components/primitives/Badge'
import { Button } from '@/components/primitives/Button'
import { Input, Switch, TextArea } from '@/components/primitives/Field'
import { Segmented } from '@/components/primitives/Segmented'
import { Select } from '@/components/primitives/Overlay'
import type { Charter, Mode, Workflow } from '@/api/desk'
import { WORKFLOWS, emptyCharter } from '@/api/desk'
import { modeMaySpendTrials } from '@/lib/desk/rings'
import { CharterForm } from './CharterForm'

/** What a send carries. One shape for both toggle positions (ADR-0037 §3). */
export interface ComposerSubmission {
  mode: Mode
  continuous: boolean
  workflow: Workflow
  /** Present when the toggle is off. */
  question: string
  /** Present when the toggle is on. */
  charter: Charter | null
  constraints: Constraints
  /** True when a run was already going: this is a steer, not a queued turn. */
  steer: boolean
}

export interface Constraints {
  instrument: string
  timeframe: string
  window: string
  balance: string
}

export const EMPTY_CONSTRAINTS: Constraints = { instrument: '', timeframe: '', window: '', balance: '' }

/**
 * A crude classifier, replaced by the harness's own at S-10.2.
 *
 * It exists so the Chat → Research offer card is a real behaviour rather than a
 * placeholder: a Chat turn that would need a trial must end with an offer, not
 * with the agent quietly spending one.
 */
const RESEARCH_SHAPED =
  /\b(backtest|sweep|optimi[sz]e|walk[- ]forward|edge|alpha|does .* predict|is .* profitable|compare .* against|falsif|replicate|hypothes)/i

export function classifyNeedsResearch(question: string): boolean {
  return RESEARCH_SHAPED.test(question)
}

/** The workflow a question's shape defaults to (§3.3, overridable). */
export function defaultWorkflow(question: string): Workflow {
  const q = question.toLowerCase()
  if (/\bfalsif|disprove|refute/.test(q)) return 'falsify'
  if (/\breplicat|reproduce/.test(q)) return 'replicate'
  if (/\bcompare|versus|\bvs\b/.test(q)) return 'compare'
  if (/\bwhy|diagnos|regress|broke/.test(q)) return 'diagnose'
  if (/\bsurvey|which of|across all/.test(q)) return 'survey'
  if (/\boptimi[sz]e|tune|sweep/.test(q)) return 'optimise'
  return 'investigate'
}

/**
 * The slash commands that are actually answered (§3.3).
 *
 * This list used to read `['/steer', '/pause', '/pin', '/why', '/model',
 * '/budget', '/tier']`, it was exported and imported by nothing, and the only
 * `onSlash` implementation in the tree handles three of those seven. The other
 * four went through `send()`'s slash branch, which calls `onSlash` and then
 * **clears the box** — so typing `/pin` deleted what you had written and did
 * nothing else. `/pause` has now gone from the handler too, because the route
 * behind it is not mounted.
 *
 * So the list is the handled set, and `send()` checks against it. An
 * unrecognised command is refused in the composer with the text left alone,
 * which is the difference between "that is not a command" and "your question is
 * gone".
 *
 * Asserted by `a_slash_command_nobody_answers_does_not_eat_the_question`.
 */
export const SLASH_COMMANDS = ['/steer', '/why', '/budget'] as const

export interface ComposerHandle {
  focus: () => void
}

export interface ComposerProps {
  /** A run is active, so a send is a steer. */
  running: boolean
  profiles: Array<{ id: string; label: string; tier: string; measured: boolean; frontier: boolean }>
  model: string
  onModelChange: (id: string) => void
  onSubmit: (s: ComposerSubmission) => void
  onSlash?: (command: string, rest: string) => void
  /**
   * Open the strategy author (ADR-0037 §4). **Optional, and the button is drawn
   * only when it is supplied** — an unpassed handler behind a rendered control
   * is how "Write strategy" came to be a button that ate the click.
   */
  onWriteStrategy?: () => void
  /** Rendered when a Chat question looks like it needs a run (stubbed offer). */
  onOffer?: (question: string) => void
}

export const Composer = forwardRef<ComposerHandle, ComposerProps>(function Composer(
  { running, profiles, model, onModelChange, onSubmit, onSlash, onWriteStrategy, onOffer },
  ref,
) {
  const [mode, setMode] = useState<Mode>('chat')
  const [continuous, setContinuous] = useState(false)
  const [question, setQuestion] = useState('')
  // The charter lives beside the question, not instead of it, so toggling is
  // lossless in both directions (§2b: "toggling off and on again does not lose
  // typed constraints").
  const [charter, setCharter] = useState<Charter>(() => emptyCharter())
  const [constraints, setConstraints] = useState<Constraints>(EMPTY_CONSTRAINTS)
  const [workflow, setWorkflow] = useState<Workflow | null>(null)
  /** The unrecognised command last typed, kept so the refusal can name it. */
  const [slashError, setSlashError] = useState<string | null>(null)
  const area = useRef<HTMLTextAreaElement>(null)

  useImperativeHandle(ref, () => ({ focus: () => area.current?.focus() }), [])

  const profile = profiles.find((p) => p.id === model)
  const effectiveWorkflow = workflow ?? defaultWorkflow(question)
  // AGENT-014 §8 / ADR-0037 §8: continuous research runs on frontier profiles
  // only, and the refusal names the profile rather than hiding the option.
  const continuousRefused = continuous && profile && !profile.frontier ? profile.label : null

  function send() {
    const text = question.trim()
    setSlashError(null)
    if (!continuous) {
      if (text.startsWith('/')) {
        const [cmd, ...rest] = text.split(' ')
        if (!(SLASH_COMMANDS as readonly string[]).includes(cmd)) {
          setSlashError(cmd)
          return
        }
        onSlash?.(cmd, rest.join(' '))
        setQuestion('')
        return
      }
      if (!text) return
      // A Chat turn that would need a trial ends with an offer, never a trial.
      if (mode === 'chat' && classifyNeedsResearch(text)) {
        onOffer?.(text)
        setQuestion('')
        return
      }
    }
    onSubmit({
      mode,
      continuous: mode === 'research' && continuous,
      workflow: effectiveWorkflow,
      question: continuous ? '' : text,
      charter: mode === 'research' && continuous ? charter : null,
      constraints,
      steer: running,
    })
    if (!continuous) setQuestion('')
  }

  return (
    <div className="desk-composer">
      <div className="row">
        <Segmented<Mode>
          value={mode}
          onChange={(m) => {
            setMode(m)
            if (m === 'chat') setContinuous(false)
          }}
          ariaLabel="Mode"
          options={[
            { value: 'chat', label: 'Chat', title: 'Answers a question. Spends no trials, ever.' },
            { value: 'research', label: 'Research', title: 'Runs the lifecycle: pre-registration, trials, a verdict.' },
          ]}
        />
        {mode === 'research' && (
          <label style={{ display: 'inline-flex', alignItems: 'center', gap: 'var(--s-2)', fontSize: 'var(--t-12)' }}>
            <Switch checked={continuous} onCheckedChange={setContinuous} label="Continuous" />
            Continuous
          </label>
        )}
        {mode === 'research' && !continuous && (
          <Select<Workflow>
            value={effectiveWorkflow}
            onChange={setWorkflow}
            ariaLabel="Workflow"
            small
            width={150}
            options={WORKFLOWS.map((w) => ({ value: w, label: w }))}
          />
        )}
        <Select
          value={model}
          onChange={onModelChange}
          ariaLabel="Model"
          small
          width={220}
          options={profiles.map((p) => ({
            value: p.id,
            label: `${p.label} · ${p.tier}${p.measured ? '' : ' · unmeasured'}`,
          }))}
        />
        {!modeMaySpendTrials(mode) && <Badge tone="info">no trial can be spent</Badge>}
        {profile && !profile.measured && <Badge tone="warn">unmeasured</Badge>}
      </div>

      <div className="row">
        {(['instrument', 'timeframe', 'window', 'balance'] as const).map((k) => (
          <Input
            key={k}
            small
            aria-label={k}
            placeholder={k}
            value={constraints[k]}
            style={{ width: 130 }}
            onChange={(e) => setConstraints((c) => ({ ...c, [k]: e.target.value }))}
          />
        ))}
      </div>

      {slashError && (
        <p className="sub" role="alert" data-testid="slash-refused">
          <span className="mono">{slashError}</span> is not a command this desk answers. The ones it
          does are <span className="mono">{SLASH_COMMANDS.join(' ')}</span>. Nothing was sent and
          what you typed is still here.
        </p>
      )}

      {continuousRefused && (
        <p className="sub" role="alert">
          Continuous research runs on frontier profiles only. {continuousRefused} cannot carry a charter.
        </p>
      )}

      {mode === 'research' && continuous ? (
        <CharterForm value={charter} onChange={setCharter} />
      ) : (
        <TextArea
          ref={area}
          rows={3}
          aria-label="Ask, or describe what to research"
          placeholder={running ? 'Sending will steer the running turn' : 'Ask, or describe what to research'}
          value={question}
          onChange={(e) => setQuestion(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter' && !e.shiftKey) {
              e.preventDefault()
              send()
            }
          }}
        />
      )}

      <div className="row">
        <Button size="sm" variant="primary" onClick={send} disabled={!!continuousRefused} data-testid="send">
          {running ? 'Steer' : continuous ? 'Start continuous research' : 'Send'}
        </Button>
        {/* **Attach… was deleted, not wired.** It called `onAttach`, which its
            one caller never passed, so the button opened nothing — and there is
            nothing for it to open: this platform mounts no upload route
            (`GET /api/attachments`, `GET /api/uploads` and `GET /api/files` all answer 404
            where every mounted route answers 401), and neither
            `POST /api/agent/conversations/{id}/messages` nor
            `POST /api/research/sessions` carries an attachment field. A file
            picker in front of no store is a promise the desk cannot keep.
            Asserted by `the_composer_draws_no_control_that_reaches_nothing`. */}
        {/* An action, not a mode (ADR-0037 §4). Drawn only when a caller can
            actually open the author, for the reason above. */}
        {onWriteStrategy && (
          <Button size="sm" variant="ghost" onClick={onWriteStrategy} data-testid="write-strategy">
            Write strategy
          </Button>
        )}
        {running && <span className="sub">Sending will steer the running turn.</span>}
      </div>
    </div>
  )
})
