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
 * | Pause / Resume | `…/runs/{}/pause`, `…/resume` → 404 | deleted |
 * | Escalate… | `…/runs/{}/escalate` → 404 | deleted |
 * | Stop | `…/conversations/{}/cancel` → 401 | kept |
 * | Save as case | `GET /api/evals`, `…/trajectory` → 404 | deleted |
 * | approval `note` | `/api/approvals/{}/answer` → 401 | wired |
 *
 * WHICH OF THESE FAILED BEFORE THE CHANGE. Every test below was run against
 * the previous code. The ones that failed are marked `[was failing]`; the ones
 * that would have passed anyway are marked `[held before]` and are here to stop
 * the fix from being undone, not to claim credit for finding it.
 */
import { render, screen, fireEvent, waitFor } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { describe, expect, it, vi } from 'vitest'

import type { DeskCard } from '@/api/desk'
import { Composer, SLASH_COMMANDS } from '../Composer'
import { ControlsRow, useDeskKeymap, type DeskControls } from '../ControlsRow'
import { EvidenceTab, MemoryTab, type EvidenceEntry } from '../RightPanel'
import { RunDetail } from '../RunDetail'
import { Card } from '../Cards'
import { ApprovalCard } from '../ApprovalsInbox'

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

  it('[was failing] lists only commands that are answered, and never the pause', () => {
    // `/pause` posted to a route that is not mounted; it left the list with the
    // handler.
    expect([...SLASH_COMMANDS]).not.toContain('/pause')
    expect([...SLASH_COMMANDS]).not.toContain('/pin')
    expect([...SLASH_COMMANDS]).toEqual(['/steer', '/why', '/budget'])
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
  }
  const props: DeskControls = {
    busy: true,
    disconnected: false,
    hasPendingApproval: true,
    approvalNeedsReason: false,
    detailsOpen: false,
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
  it('[was failing] draws no Pause, because nothing answers a pause', () => {
    // The worst of the three. `onPause` set a local flag *before* the request,
    // so the header badge changed to "paused" over a run that was still
    // spending trials, and the request itself went through `optional()`, which
    // turns the 404 into `null`. A false instrument reading, bound to `space`.
    controls()
    expect(screen.queryByTestId('pause')).toBeNull()
    expect(screen.queryByTestId('resume')).toBeNull()
    expect(screen.queryByText(/^Pause/)).toBeNull()
  })

  it('[was failing] draws no Escalate…, for the same reason', () => {
    controls()
    expect(screen.queryByText(/escalate/i)).toBeNull()
  })

  it('[held before, and only vacuously] does not bind space to a pause that cannot pause', () => {
    // Honest accounting: this one passed against the old code too, because the
    // old `space` branch called `k.onPause()` and this harness no longer has an
    // `onPause` for it to call — it threw inside the listener instead of
    // failing an assertion. The test that actually caught the pause is the
    // first one in this block. This is here to keep the binding gone.
    const s = controls()
    fireEvent.keyDown(document, { key: ' ' })
    // Nothing at all should have fired; `space` no longer means anything here.
    for (const fn of Object.values(s)) expect(fn).not.toHaveBeenCalled()
  })

  it('[held before] keeps Stop, which reaches a route that is mounted', () => {
    const s = controls()
    expect(screen.getByTestId('stop')).toBeTruthy()
    fireEvent.keyDown(document, { key: 'Escape' })
    expect(s.onStop).toHaveBeenCalled()
  })

  it('[was failing] does not offer a pause as the gentler alternative to stopping', () => {
    // The stop dialog talked the person out of the control that works into the
    // one that does not: "Pause instead".
    controls()
    fireEvent.click(screen.getByTestId('stop'))
    // The dialog is really open — otherwise the assertion below is vacuous.
    expect(screen.getByText(/Stop this run\?/i)).toBeTruthy()
    expect(screen.queryByText(/pause instead/i)).toBeNull()
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
    render(<MemoryTab />)
    expect(screen.getByText(/No endpoint reports what this run's brief contained/i)).toBeTruthy()
  })

  it('[was failing] takes no items, so nothing can hand it the wrong ones', () => {
    // `onForget` could not have worked either: it was `(id: string) => void`
    // and the delete route needs the bucket as well, which `BriefItem` carried
    // and the callback dropped.
    expect(MemoryTab.length).toBe(0)
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
    await waitFor(() => expect(screen.getByText(/No trajectory is stored/i)).toBeTruthy())
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
