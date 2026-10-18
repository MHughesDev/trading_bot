/**
 * One defect, six places: **a prop declared in the type, wired in the child,
 * and passed by nobody at the only call site.** The control renders, the click
 * lands on a handler that is `undefined`, and there is no error, no toast and
 * no console line — so the surface looks like it works and the user is the
 * instrument that fails.
 *
 * Each of the six was checked against the live platform on 127.0.0.1:7080
 * before it was decided, where **401 means the route is mounted and 404 means
 * it is not**. The rule applied to each: wire it where the thing it calls
 * exists, delete it where it does not. A control that cannot work must not
 * render as though it can, and an exported handler nobody passes is one the
 * next caller reaches for.
 *
 * What the probe found, and what was done with it:
 *
 * | control | route | result |
 * | --- | --- | --- |
 * | Attach… | no upload route at all | deleted |
 * | Write strategy | `/strategy` page, `/api/strategies` 401 | wired |
 * | `EvidenceEntry.view` | no route returns a tool result by id | deleted |
 * | Pause / Resume | `…/runs/{}/pause`, `…/resume` → 404 | deleted — **re-wired 2026-09-23, see below** |
 * | Escalate… | `…/runs/{}/escalate` → 404 | deleted |
 * | Stop | `…/conversations/{}/cancel` → 401 | kept |
 * | Save as case | `GET /api/evals`, `…/trajectory` → 404 | deleted |
 * | approval `note` | `/api/approvals/{}/answer` → 401 | wired |
 *
 * WHICH OF THESE FAILED BEFORE THE CHANGE. Every test below was run against
 * the previous code. The ones that failed are marked `[was failing]`; the ones
 * that would have passed anyway are marked `[held before]` and are here to stop
 * the fix from being undone, not to claim credit for finding it.
 *
 * **2026-09-23: the pause came back, and the rule did not change.** ADR-D-27
 * mounted `…/runs/{id}/pause` and `…/resume`, so the row draws them again — but
 * only where the server *measured* the pause. `paused` is `true`, `false`, or
 * `null` on every run read, and `null` ("this replica is not driving the run")
 * draws no control at all, because a pause posted into it answers 409. The
 * original defect was a local `paused` flag set before the request; the fix that
 * makes re-wiring safe is that the flag no longer exists — the state comes from
 * the run row. Tests marked `[new]` here cover that three-state behaviour and
 * the deadline label; the Escalate test is unchanged, because `…/escalate` is
 * still 404.
 */
import { render, screen, fireEvent, waitFor, cleanup } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { describe, expect, it, vi } from 'vitest'

import type { DeskCard } from '@/api/desk'
import { Composer } from '../Composer'
import { ControlsRow, type DeskControls } from '../ControlsRow'
import { EvidenceTab, MemoryTab, type EvidenceEntry } from '../RightPanel'
import { RunDetail } from '../RunDetail'
import { Card } from '../Cards'
import { ApprovalCard } from '../ApprovalsInbox'
import { SLASH_COMMANDS } from '@/components/desk/Composer.helpers'
import { slashAction } from '@/lib/desk/sessions'
import { useDeskKeymap } from '@/components/desk/useDeskKeymap'

// The inbox answers through `researchApi`, not `deskApi`.
const answerApproval = vi.hoisted(() => vi.fn(() => Promise.resolve({ data: {} })))
vi.mock('@/api/research', async (orig) => {
  const real = (await orig()) as Record<string, unknown>
  return { ...real, researchApi: { answerApproval } }
})

// `RunDetail` reads the trajectory, which on this platform is a 404 that
// `optional()` turns into `null`. The mock reproduces that, because the point of
// the last test is what the screen does when there is nothing to save.
const trajectory = vi.hoisted(() => vi.fn(() => Promise.resolve(null)))
vi.mock('@/api/desk', async (orig) => {
  const real = (await orig()) as Record<string, unknown>
  return { ...real, deskApi: { trajectory } }
})

const PROFILES = [
  { id: 'p1', label: 'Frontier', tier: 'frontier', measured: true, frontier: true },
]

function composer(over: Partial<React.ComponentProps<typeof Composer>> = {}) {
  const onSubmit = vi.fn()
  const onSlash = vi.fn()
  render(
    <Composer
      running={false}
      profiles={PROFILES}
      model="p1"
      onModelChange={() => {}}
      onSubmit={onSubmit}
      onSlash={onSlash}
      {...over}
    />,
  )
  return { onSubmit, onSlash }
}

// ── 1. The composer ──────────────────────────────────────────────────────────

describe('the composer draws no control that reaches nothing', () => {
  it('[was failing] offers no Attach…, because this platform stores no attachment', () => {
    // It rendered `<Button onClick={onAttach}>Attach…</Button>` and its only
    // caller passed no `onAttach`. There is also nowhere for a file to go: no
    // upload route is mounted, and neither the message POST nor the session
    // POST carries an attachment field. A file picker in front of no store is
    // a promise the desk cannot keep, so the button is gone rather than inert.
    composer()
    expect(screen.queryByText(/attach/i)).toBeNull()
  })

  it('[was failing] draws Write strategy only when something can open the author', () => {
    // Asserted on the label, not the test id: the old button had no test id, so
    // a `queryByTestId` here would have passed against the broken code and
    // proved nothing.
    composer()
    expect(screen.queryByText('Write strategy')).toBeNull()
  })

  it('[was failing] opens the author when it is given a way to', () => {
    const onWriteStrategy = vi.fn()
    composer({ onWriteStrategy })
    fireEvent.click(screen.getByTestId('write-strategy'))
    expect(onWriteStrategy).toHaveBeenCalledTimes(1)
  })
})

describe('a slash command nobody answers does not eat the question', () => {
  it('[was failing] refuses an unhandled command and leaves what was typed', () => {
    // `/pin` was in `SLASH_COMMANDS`, no `onSlash` implementation handled it,
    // and `send()` cleared the box on every command alike. So typing `/pin`
    // deleted the question and did nothing — the most expensive shape of doing
    // nothing, because the work is gone too.
    const { onSlash } = composer()
    const box = screen.getByLabelText('Ask, or describe what to research') as HTMLTextAreaElement
    fireEvent.change(box, { target: { value: '/pin something worth keeping' } })
    fireEvent.click(screen.getByTestId('send'))

    expect(onSlash).not.toHaveBeenCalled()
    expect(box.value).toBe('/pin something worth keeping')
    expect(screen.getByTestId('slash-refused')).toHaveTextContent('/pin')
  })

  it('[held before] still sends the commands something answers', () => {
    const { onSlash } = composer()
    const box = screen.getByLabelText('Ask, or describe what to research') as HTMLTextAreaElement
    fireEvent.change(box, { target: { value: '/steer look at 5m bars' } })
    fireEvent.click(screen.getByTestId('send'))
    expect(onSlash).toHaveBeenCalledWith('/steer', 'look at 5m bars')
    expect(box.value).toBe('')
  })

  it('[was failing] lists only commands that are answered', () => {
    // `/pause` left this list when its route was unmounted, and came back
    // 2026-10-03 with a handler: `slashAction` maps every entry to what it does
    // in this conversation, and an entry it maps to nothing is the defect this
    // whole file is about. `/pin` stays out — nothing records a per-run pin.
    expect([...SLASH_COMMANDS]).not.toContain('/pin')
    for (const cmd of SLASH_COMMANDS) {
      for (const ctx of [
        { continuous: true, hasRun: true },
        { continuous: false, hasRun: true },
        { continuous: false, hasRun: false },
      ]) {
        expect(slashAction(cmd, 'add a question', ctx), `${cmd} in ${JSON.stringify(ctx)}`).not.toBeNull()
      }
    }
    expect(slashAction('/pin', 'x', { continuous: true, hasRun: true })).toBeNull()
  })
})

// ── 2. The controls row ──────────────────────────────────────────────────────

function controls(over: Partial<DeskControls> = {}) {
  const spies = {
    onStop: vi.fn(),
    onSteer: vi.fn(),
    onApprove: vi.fn(),
    onDeny: vi.fn(),
    onBudget: vi.fn(),
    onToggleDetails: vi.fn(),
    onReplay: vi.fn(),
    onFocusComposer: vi.fn(),
    onShortcuts: vi.fn(),
    onPause: vi.fn(),
    onResume: vi.fn(),
  }
  const props: DeskControls = {
    busy: true,
    disconnected: false,
    hasPendingApproval: true,
    approvalNeedsReason: false,
    detailsOpen: false,
    // `unobserved` is the default here on purpose: it is what a run this replica
    // is not driving reports, and the tests below assert that state draws no
    // pause control at all. A default of `running` would make them vacuous.
    pause: { kind: 'unobserved' },
    ...spies,
    ...over,
  }
  function Host() {
    useDeskKeymap(props)
    return <ControlsRow {...props} />
  }
  render(<Host />)
  return spies
}

describe('a run control the platform cannot serve is not drawn', () => {
  // `…/pause` and `…/resume` are mounted now (ADR-D-27) and the pause is wired
  // again — but only where the server *measured* it. These tests moved from "no
  // pause exists" to "no pause is drawn over an unmeasured one", which is the
  // same rule applied to a route that now answers. `…/escalate` is still 404 and
  // still deleted, so its test is unchanged.

  it('[was failing] draws no Pause over a run whose pause nobody measured', () => {
    // `paused: null` means this replica is not driving the run. A Pause button
    // here posts a request that answers 409, which is the deleted control
    // rebuilt one layer down — so `unobserved` draws neither button.
    controls({ pause: { kind: 'unobserved' } })
    expect(screen.queryByTestId('pause')).toBeNull()
    expect(screen.queryByTestId('resume')).toBeNull()
    expect(screen.queryByTestId('paused-until')).toBeNull()
  })

  it('[new] draws Pause for a running run and calls the handler', () => {
    const s = controls({ pause: { kind: 'running' } })
    expect(screen.queryByTestId('resume')).toBeNull()
    fireEvent.click(screen.getByTestId('pause'))
    expect(s.onPause).toHaveBeenCalledTimes(1)
  })

  it('[new] draws Resume for a paused run, with the deadline it was given', () => {
    const until = new Date(Date.now() + 15 * 60_000).toISOString()
    const s = controls({ pause: { kind: 'paused', until } })
    expect(screen.queryByTestId('pause')).toBeNull()
    expect(screen.getByTestId('paused-until')).toHaveTextContent(/held until \d{1,2}:\d{2}/i)
    fireEvent.click(screen.getByTestId('resume'))
    expect(s.onResume).toHaveBeenCalledTimes(1)
  })

  it('[new] says the deadline is unreported rather than inventing one', () => {
    // A pause whose `until` did not arrive is still a pause. Printing a dash or
    // falling back to `now` would both be a deadline the reader is wrong about.
    controls({ pause: { kind: 'paused', until: null } })
    expect(screen.getByTestId('paused-until')).toHaveTextContent('deadline not reported')

    // And the same for a timestamp that arrived unparseable.
    cleanup()
    controls({ pause: { kind: 'paused', until: 'not a date' } })
    expect(screen.getByTestId('paused-until')).toHaveTextContent('deadline not reported')
  })

  it('[held before] draws no Escalate…, because nothing answers an escalate', () => {
    controls({ pause: { kind: 'running' } })
    expect(screen.queryByText(/escalate/i)).toBeNull()
  })

  it('[new] binds space only where the pause was measured', () => {
    // Under `unobserved` the keyboard must do nothing at all, for the same
    // reason the button is absent: a shortcut that silently 409s is the version
    // of this control that was deleted.
    const quiet = controls({ pause: { kind: 'unobserved' } })
    fireEvent.keyDown(document, { key: ' ' })
    for (const fn of Object.values(quiet)) expect(fn).not.toHaveBeenCalled()
    cleanup()

    const live = controls({ pause: { kind: 'running' } })
    fireEvent.keyDown(document, { key: ' ' })
    expect(live.onPause).toHaveBeenCalledTimes(1)
    expect(live.onResume).not.toHaveBeenCalled()
    cleanup()

    const held = controls({ pause: { kind: 'paused', until: null } })
    fireEvent.keyDown(document, { key: ' ' })
    expect(held.onResume).toHaveBeenCalledTimes(1)
    expect(held.onPause).not.toHaveBeenCalled()
  })

  it('[held before] keeps Stop, which reaches a route that is mounted', () => {
    // `esc` opens the confirmation rather than sending the cancel (§3.4,
    // "esc stop (confirm)"; changed 2026-09-27). The point this case was written to
    // make — that Stop, unlike the three deleted controls, is in front of a route
    // that exists — is now made through the dialog's own confirm.
    const s = controls()
    expect(screen.getByTestId('stop')).toBeTruthy()
    fireEvent.keyDown(document, { key: 'Escape' })
    expect(s.onStop).not.toHaveBeenCalled()
    fireEvent.click(screen.getByRole('button', { name: 'Stop' }))
    expect(s.onStop).toHaveBeenCalled()
  })

  it('[held before] does not offer a pause as the gentler alternative to stopping', () => {
    // The stop dialog once talked the person out of the control that works into
    // the one that did not: "Pause instead". A pause exists now and the dialog
    // still must not route anyone into it — it delays by a bounded window and
    // then stops the run anyway, so offering it beside Stop trades a decision
    // for a deferral.
    controls({ pause: { kind: 'running' } })
    fireEvent.click(screen.getByTestId('stop'))
    // The dialog is really open — otherwise the assertion below is vacuous.
    expect(screen.getByText(/Stop this run\?/i)).toBeTruthy()
    expect(screen.queryByText(/pause instead/i)).toBeNull()
  })

  it('[new] the stop dialog no longer claims this platform has no pause', () => {
    // It said "There is no pause on this platform, so this is the only way to
    // make it stop working." True when written, false since ADR-D-27, and the
    // kind of sentence that survives because nobody re-reads a dialog.
    controls({ pause: { kind: 'running' } })
    fireEvent.click(screen.getByTestId('stop'))
    expect(screen.queryByText(/there is no pause on this platform/i)).toBeNull()
  })
})

describe('an approval that needs a reason is not approved from the keyboard', () => {
  it('[was failing] stands down when the platform will refuse without a reason', () => {
    // `spend_holdout_look` refuses an approval without twenty characters of
    // reason. `a` had nowhere to type one and sent anyway, into a `void`, so
    // the refusal was invisible.
    const s = controls({ approvalNeedsReason: true })
    fireEvent.keyDown(document, { key: 'a' })
    expect(s.onApprove).not.toHaveBeenCalled()
    expect((screen.getByTestId('approve-from-controls') as HTMLButtonElement).disabled).toBe(true)
  })

  it('[was failing] still approves the reversible ones in one keystroke', () => {
    const s = controls({ approvalNeedsReason: false })
    fireEvent.keyDown(document, { key: 'a' })
    expect(s.onApprove).toHaveBeenCalled()
    expect((screen.getByTestId('approve-from-controls') as HTMLButtonElement).disabled).toBe(false)
  })

  it('[held before] denying is never gated on a reason, because denial needs none', () => {
    const s = controls({ approvalNeedsReason: true })
    fireEvent.keyDown(document, { key: 'x' })
    expect(s.onDeny).toHaveBeenCalled()
  })
})

// ── 3. The approval note ─────────────────────────────────────────────────────

function approvalCard(envelope: string) {
  const card: DeskCard = {
    seq: 1,
    step: 1,
    kind: 'approval',
    at: new Date().toISOString(),
    headline: 'Look at the sealed holdout?',
    detail: { approval_id: 'ap_1', envelope },
  }
  const onApprove = vi.fn()
  render(<Card card={card} actions={{ onApprove }} />)
  return onApprove
}

describe('the reason typed on an approval reaches the wire', () => {
  it('[held before] the card sends the note as the second argument', () => {
    // This half was already right. It is here because the other half — the
    // caller — dropped the argument, and a test on the sender alone is what
    // let that go unnoticed.
    const onApprove = approvalCard('SealedHoldout')
    fireEvent.click(screen.getByTestId('approve'))
    fireEvent.change(screen.getByTestId('approve-note'), {
      target: { value: 'checked against the pre-registered falsifier' },
    })
    fireEvent.click(screen.getByTestId('approve'))
    expect(onApprove).toHaveBeenCalledWith('ap_1', 'checked against the pre-registered falsifier')
  })

  it('[held before] sends no note where none was typed, rather than an empty string', () => {
    const onApprove = approvalCard('SealedHoldout')
    fireEvent.click(screen.getByTestId('approve'))
    fireEvent.click(screen.getByTestId('approve'))
    expect(onApprove).toHaveBeenCalledWith('ap_1', undefined)
  })
})

// ── 4. Evidence and memory ───────────────────────────────────────────────────

describe('an id that opens nothing does not pretend to open', () => {
  it('[held before, which is the point] clicking an evidence id toggles no empty node', () => {
    // This passed against the old code as well — and that is the defect stated
    // precisely. The old tab toggled `openId` and rendered `e.view`, which was
    // `undefined` on every entry any caller ever built, so the DOM did not
    // change then either. A bug whose whole symptom is "nothing observable
    // happens" cannot be caught by observing the DOM; it is caught by the type
    // assertion below, which `tsc` checks.
    // `EvidenceEntry.view` was declared "rendered in place when the id is
    // opened", the tab kept an `openId` and toggled it on every click, and no
    // caller ever set it. Nothing in the tree could: a viewer needs the tool
    // result behind the id, and no route returns one.
    const before = document.body.innerHTML
    const onOpenId = vi.fn()
    render(
      <EvidenceTab
        entries={[{ id: 'exp_1', type: 'exp', summary: 'produced by run_sim at step 2' }]}
        resolve={() => ({ label: 'from run_sim, step 2', kind: 'evidence' })}
        onOpenId={onOpenId}
      />,
    )
    const html = document.body.innerHTML
    fireEvent.click(screen.getByTestId('ev-exp_1'))
    // The click is still reported upward — that is the live half — but the
    // panel below it does not change, because there was never anything to put
    // there.
    expect(onOpenId).toHaveBeenCalledWith('exp_1')
    expect(document.body.innerHTML).toBe(html)
    expect(html).not.toBe(before)
  })

  it('[was failing under tsc, not under vitest] no entry can carry a view', () => {
    // The deletion, asserted by name. `prioritisedBy` taught this repo that a
    // dead prop is a prop the next caller reaches for, and that the fix that
    // sticks is removing the name.
    const entry: EvidenceEntry = { id: 'exp_1', type: 'exp', summary: 's' }
    expect(Object.keys(entry)).not.toContain('view')
    const rejected: EvidenceEntry = {
      id: 'exp_2',
      type: 'exp',
      summary: 's',
      // @ts-expect-error `view` is not part of EvidenceEntry any more, and this
      // line is the assertion: if the field comes back, tsc stops complaining
      // and this test stops compiling.
      view: 'anything at all',
    }
    expect(rejected.id).toBe('exp_2')
  })
})

describe('the memory tab does not offer the store as the brief', () => {
  it('[was failing] says no endpoint reports the brief, and names the store as a different thing', () => {
    // The tab was `<MemoryTab items={null} />` — a literal — so two thirds of
    // the component were unreachable. The reachable third is correct and
    // stays: the memory *store* is served and the Memory page browses it, but
    // this tab is the *brief* (what retrieval put in this run's prompt, why,
    // and whether it was cited), and nothing records that per run. Filling the
    // panel with store rows would put something that looks like the answer
    // where the answer is not known.
    //
    // 2026-10-03: the per-run record exists after all — `memory_retrievals`,
    // written before every run — and `GET /api/desk/runs/{id}/brief` serves it.
    // While that route is unregistered it answers 404, and the tab still says
    // what it said, rather than showing the store.
    render(<MemoryTab brief={{ served: false }} />)
    expect(screen.getByText(/No endpoint reports what this run's brief contained/i)).toBeTruthy()
  })

  it('[was failing] forget names the bucket, which the delete route needs', async () => {
    // `onForget` could not have worked: it was `(id: string) => void` and the
    // delete route needs the bucket as well. The brief route sends the bucket
    // with each entry, and the callback takes both.
    const onForget = vi.fn(() => Promise.resolve())
    render(
      <MemoryTab
        brief={{
          served: true,
          data: {
            run_id: 'r1',
            entries_readable: true,
            not_recorded: [],
            items: [
              {
                entry_id: 'f_1',
                rank: 1,
                slot: 'finding',
                why: 'same instrument',
                cited: null,
                cited_at: null,
                retrieved_at: '2026-10-03T00:00:00Z',
                entry: { kind: 'finding', bucket: 'findings', subject: 'BTC-USD', claim: 'gap fades', archived: false, superseded: false },
              },
            ],
          },
        }}
        onForget={onForget}
      />,
    )
    fireEvent.click(screen.getByRole('button', { name: 'Forget' }))
    expect(onForget).toHaveBeenCalledWith('findings', 'f_1')
  })
})

// ── 5. Run detail ────────────────────────────────────────────────────────────

describe('nothing on the run detail claims to have written a file', () => {
  it('[held before, because the button was unreachable too] no Save as case on an empty trajectory', async () => {
    // Its entire implementation was `window.alert('Trajectory written to
    // evals/ …')`. A confirmation of work that did not happen is worse than a
    // button that does nothing, because the person stops looking for the file.
    const alerted = vi.spyOn(window, 'alert').mockImplementation(() => {})
    const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })
    render(
      <QueryClientProvider client={qc}>
        <RunDetail runId="run_1" onBack={() => {}} />
      </QueryClientProvider>,
    )
    // Waits on the empty state's action rather than its headline. The headline
    // used to read "No trajectory is stored for this run" in both of this
    // panel's absences, and it now says which absence it is — a route this
    // platform does not mount, or a run with no steps — so it is no longer a
    // stable thing for an unrelated claim to key on. See
    // `an-unmounted-route-is-not-a-run-that-took-no-steps`.
    await waitFor(() => expect(screen.getByRole('button', { name: /Back to the conversation/ })).toBeTruthy())
    expect(screen.queryByText(/save as case/i)).toBeNull()
    expect(alerted).not.toHaveBeenCalled()
    alerted.mockRestore()
  })

  it('[was failing] and none when the trajectory does arrive, which is when it would lie', async () => {
    // The empty-trajectory case above passed against the old code, because the
    // route is 404 here and `RunDetail` returns the empty state before it ever
    // draws its header. That is not a reason to keep the button: it means the
    // lie is dormant, and wakes on the day the route lands. With steps present
    // the old header rendered "Save as case", and clicking it told the person a
    // file had been written to `evals/`.
    trajectory.mockResolvedValueOnce({
      steps: [
        {
          step: 1,
          at: new Date().toISOString(),
          sections: [{ layer: 'charter', text: 'CHARTER', tokens: 2 }],
          prompt: 'CHARTER',
          reply: 'ok',
          state: null,
        },
      ],
    } as never)
    const alerted = vi.spyOn(window, 'alert').mockImplementation(() => {})
    const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })
    render(
      <QueryClientProvider client={qc}>
        <RunDetail runId="run_2" onBack={() => {}} />
      </QueryClientProvider>,
    )
    await waitFor(() => expect(screen.getByTestId('replay-prompt')).toBeTruthy())
    expect(screen.queryByText(/save as case/i)).toBeNull()
    expect(alerted).not.toHaveBeenCalled()
    alerted.mockRestore()
  })
})

// ── 6. The approvals inbox ───────────────────────────────────────────────────

describe('the inbox can say why, because the platform asks it to', () => {
  const approval = {
    approval_id: 'ap_9',
    kind: 'qc_waiver',
    state: 'pending',
    options: ['approve', 'deny'],
    default_option: 'deny',
    timeout_at: null,
    created_at: new Date().toISOString(),
    payload: { envelope: 'SealedHoldout', question: 'Look at the sealed holdout?' },
  }

  it('[was failing] offers a reason field on the irreversible path and sends it', async () => {
    // `researchApi.answerApproval` has taken a third `note` argument since it
    // was written, and this card — the surface whose entire job is deciding
    // these — called it with two and had no field to type one into. For a kind
    // the platform refuses without twenty characters of reason, that is a card
    // that could only ever be refused.
    answerApproval.mockClear()
    render(<ApprovalCard approval={approval as never} />)
    fireEvent.click(screen.getByText('approve'))
    fireEvent.change(screen.getByTestId('inbox-note'), {
      target: { value: 'the pre-registered falsifier was checked first' },
    })
    fireEvent.click(screen.getByText(/Confirm approve/))
    await waitFor(() =>
      expect(answerApproval).toHaveBeenCalledWith(
        'ap_9',
        'approve',
        'the pre-registered falsifier was checked first',
      ),
    )
  })

  it('[was failing] sends no reason where none was typed, rather than no argument', async () => {
    // Fails against the old card too, and for a reason worth keeping: it called
    // `answerApproval(id, option)` with two arguments, so the third was not
    // `undefined`, it was absent. The distinction is invisible on the wire here
    // — axios serialises a missing key and an undefined one the same way — but
    // it is exactly the shape that let the note go missing in the first place.
    answerApproval.mockClear()
    render(<ApprovalCard approval={approval as never} />)
    fireEvent.click(screen.getByText('approve'))
    fireEvent.click(screen.getByText(/Confirm approve/))
    await waitFor(() => expect(answerApproval).toHaveBeenCalledWith('ap_9', 'approve', undefined))
  })
})
