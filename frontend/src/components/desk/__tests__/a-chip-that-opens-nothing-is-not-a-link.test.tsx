/**
 * A resolved evidence id was a `<button>` on four surfaces that cannot open one
 * (AGENT-012 §3.5, §6.3, principle 9; task S-6.9).
 *
 * `EvidenceChip`'s resolved branch rendered `<button onClick={() => onOpen?.(id)}>`
 * — the optional call is the whole defect. With no `onOpen` the element still got
 * a pointer cursor, a focus ring and `role=button`, and a click did nothing at
 * all: no panel, no error, no console line. The reading a person takes from that
 * is "the platform is broken", not "this surface has no viewer".
 *
 * Four call sites pass no `onOpen`, and they are not obscure ones:
 *
 *  - `Conversation.tsx`'s `<EvidenceTab>` — the Evidence panel, whose entire job
 *    is to list every id the run produced. `StateTab` gets
 *    `onOpenId={() => setTab('evidence')}`, which is a real destination; the
 *    Evidence tab has nowhere further to send anyone, so it was never given one.
 *  - `ApprovalsInbox.tsx:117` — the ids attached to a waiting approval.
 *  - `Dossier.tsx:137` — the evidence table of the document whose whole claim is
 *    that its evidence is checkable.
 *  - `Dossier.tsx:191` — the funnel tickets on the same page.
 *
 * The sibling argument was already written down one branch up, for the chip whose
 * resolver never ran: *"a `<button>` here would invite the click that an
 * unverified id must not get — the resolved chip is the one that opens things."*
 * It holds just as well for a resolved chip that nothing can open.
 *
 * The fix does not hide the resolution: the chip still renders, still carries its
 * provenance mark and evidence class, and still says what produced it. Only the
 * promise is withdrawn. `data-openable` is the attribute the two cases differ on,
 * so a call site that silently loses its `onOpen` is visible rather than inert.
 *
 * What would actually let this clause close is a route, and there is none: no
 * endpoint returns the tool result behind an id, which is why `EvidenceEntry.view`
 * was deleted rather than fed (see `RightPanel.tsx`, `EvidenceEntry`).
 */
import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'

import { EvidenceChip } from '../Shared'
import { EvidenceTab } from '../RightPanel'

const resolved = () => ({ label: 'from run_sweep, step 4', kind: 'evidence' })

describe('a resolved chip only looks openable where something opens it', () => {
  it('is not a button when no surface wired it', () => {
    const { container } = render(<EvidenceChip id="exp_1" resolve={resolved} />)
    expect(container.querySelector('button')).toBeNull()
    const chip = screen.getByTestId('ev-exp_1')
    expect(chip.getAttribute('data-resolved')).toBe('true')
    expect(chip.getAttribute('data-openable')).toBe('false')
  })

  it('is a button when something is', () => {
    const onOpen = vi.fn()
    const { container } = render(<EvidenceChip id="exp_2" resolve={resolved} onOpen={onOpen} />)
    expect(container.querySelector('button')).not.toBeNull()
    const chip = screen.getByTestId('ev-exp_2')
    expect(chip.getAttribute('data-openable')).toBe('true')
    fireEvent.click(chip)
    expect(onOpen).toHaveBeenCalledWith('exp_2')
  })

  it('keeps the resolution readable when it withdraws the click', () => {
    // Withdrawing the promise must not withdraw the answer. The id, what produced
    // it, and both §6.3a marks all still render — otherwise this fix would trade
    // one silent panel for another.
    render(
      <EvidenceChip
        id="exp_3"
        resolve={() => ({
          label: 'returned by fetch_page',
          kind: 'tool result',
          mark: {
            provenance: 'untrusted',
            source: 'seekingalpha.example',
            evidence_class: 'model_summary',
          },
        })}
      />,
    )
    const chip = screen.getByTestId('ev-exp_3')
    expect(chip.textContent).toContain('exp_3')
    expect(chip.textContent).toContain('fetched')
    expect(chip.textContent).toContain('seekingalpha.example')
    expect(chip.textContent).toContain('model summary')
    expect(chip.getAttribute('data-provenance')).toBe('untrusted')
    expect(chip.className).toContain('untrusted')
    expect(chip.getAttribute('title')).toContain('returned by fetch_page')
  })

  it('gives the four cases four renderings', () => {
    // Nobody looked · looked and found nothing · resolved and openable · resolved
    // with nowhere to open. Three of these were two.
    const unchecked = render(<EvidenceChip id="a" />).container
    const missing = render(<EvidenceChip id="b" resolve={() => null} />).container
    const openable = render(<EvidenceChip id="c" resolve={resolved} onOpen={() => undefined} />).container
    const inert = render(<EvidenceChip id="d" resolve={resolved} />).container
    const shape = (c: HTMLElement) => {
      const el = c.querySelector('[data-resolved]')
      return [
        el?.tagName,
        el?.getAttribute('data-resolved'),
        el?.getAttribute('data-openable') ?? 'absent',
      ].join('/')
    }
    const shapes = [unchecked, missing, openable, inert].map(shape)
    expect(new Set(shapes).size, shapes.join(' | ')).toBe(4)
  })
})

describe('the Evidence panel does not promise a viewer it has not got', () => {
  const entries = [{ id: 'exp_9', type: 'exp', summary: 'produced by run_sweep at step 4' }]

  it('renders its ids as chips rather than as buttons, because nothing passes onOpenId', () => {
    // This is the live call site: `Conversation.tsx` renders `<EvidenceTab>` with
    // `resolve`, `absence` and `entries` and no `onOpenId`.
    const { container } = render(<EvidenceTab entries={entries} resolve={resolved} />)
    expect(container.querySelector('button')).toBeNull()
    expect(screen.getByTestId('ev-exp_9').getAttribute('data-openable')).toBe('false')
    // Still listed, still resolved, still says what produced it.
    expect(screen.getByText('produced by run_sweep at step 4')).toBeInTheDocument()
    expect(screen.getByTestId('ev-exp_9').getAttribute('data-resolved')).toBe('true')
  })

  it('draws a button the day a caller has somewhere to send the click', () => {
    const onOpenId = vi.fn()
    render(<EvidenceTab entries={entries} resolve={resolved} onOpenId={onOpenId} />)
    fireEvent.click(screen.getByTestId('ev-exp_9'))
    expect(onOpenId).toHaveBeenCalledWith('exp_9')
    expect(screen.getByTestId('ev-exp_9').getAttribute('data-openable')).toBe('true')
  })
})
