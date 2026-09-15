// The desk's honesty checks, each asserting that the *violation is rejected*
// rather than that the happy path works.
//
//   §19.2  an unknown id renders struck-through
//   §19.1  a number without its n says so
//   §6.3a  untrusted text renders inert, and the mark survives export
//   §6.7   an irreversible approval cannot be confirmed in one click
import { render, screen, fireEvent } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import type { DeskCard } from '@/api/desk'
import { Card, statLine } from '../Cards'
import { EvidenceChip, InertText } from '../Shared'
import { makeResolver } from '../Conversation'
import { chainTouchesUntrusted, dossierToMarkdown, type Dossier } from '../Dossier'

const KNOWN = 'exp_7f0c'
const resolve = (id: string) => (id === KNOWN ? { label: 'an experiment', kind: 'experiment' } : null)

describe('every id resolves or is struck through (§19.2)', () => {
  it('strikes an id nothing can resolve', () => {
    render(<EvidenceChip id="exp_deadbeef" resolve={resolve} />)
    const chip = screen.getByTestId('ev-exp_deadbeef')
    expect(chip.getAttribute('data-resolved')).toBe('false')
    expect(chip.querySelector('s')).not.toBeNull()
    expect(chip.textContent).toContain('not found')
  })

  it('renders a resolvable id as something openable', () => {
    const onOpen = vi.fn()
    render(<EvidenceChip id={KNOWN} resolve={resolve} onOpen={onOpen} />)
    const chip = screen.getByTestId(`ev-${KNOWN}`)
    expect(chip.getAttribute('data-resolved')).toBe('true')
    fireEvent.click(chip)
    expect(onOpen).toHaveBeenCalledWith(KNOWN)
  })

  it('a card citing an id no tool produced is struck', () => {
    // The resolver is built from what tools *returned*, so a claim that cites a
    // fabricated id cannot make itself resolvable by mentioning it.
    const toolCard: DeskCard = {
      seq: 1,
      step: 1,
      kind: 'tool',
      at: new Date().toISOString(),
      headline: 'run_backtest',
      detail: { produced_ids: [KNOWN] },
    }
    const finding: DeskCard = {
      seq: 2,
      step: 1,
      kind: 'finding',
      at: new Date().toISOString(),
      headline: 'the edge holds',
      ids: [KNOWN, 'exp_fabricated'],
      detail: { confidence: 'suggestive', n: 42 },
    }
    const r = makeResolver(null, [toolCard, finding])
    render(<Card card={finding} resolve={r} />)
    expect(screen.getByTestId(`ev-${KNOWN}`).getAttribute('data-resolved')).toBe('true')
    expect(screen.getByTestId('ev-exp_fabricated').getAttribute('data-resolved')).toBe('false')
  })
})

describe('a number without its sample size says so (§19.1)', () => {
  it('prints (n unknown) rather than the bare number', () => {
    expect(statLine({ se: 0.2 })).toBe('(n unknown) · se=0.2')
    expect(statLine({ n: 120, se: 0.2 })).toBe('n=120 · se=0.2')
    expect(statLine({})).toBeNull()
  })

  it('renders the absence on the card', () => {
    const card: DeskCard = {
      seq: 1,
      step: 1,
      kind: 'tool',
      at: new Date().toISOString(),
      headline: 'sharpe_of',
      detail: { summary: 'Sharpe 1.8 on the window', se: 0.4 },
    }
    render(<Card card={card} />)
    expect(screen.getByText(/\(n unknown\)/)).toBeInTheDocument()
  })
})

describe('the provenance fence reaches the human (§6.3a, §19.1a)', () => {
  const HOSTILE = '<b>buy this</b> <a href="https://evil.example">click</a> <img src="x">'

  it('renders untrusted text as inert text, never as markup', () => {
    const { container } = render(<InertText>{HOSTILE}</InertText>)
    const block = screen.getByTestId('inert-text')
    // The characters are there; the elements are not.
    expect(block.textContent).toContain('<b>buy this</b>')
    expect(container.querySelector('a')).toBeNull()
    expect(container.querySelector('img')).toBeNull()
    expect(container.querySelector('b')).toBeNull()
  })

  it('marks a card whose content was fetched, naming the source', () => {
    const card: DeskCard = {
      seq: 1,
      step: 1,
      kind: 'finding',
      at: new Date().toISOString(),
      headline: HOSTILE,
      mark: { provenance: 'untrusted', source: 'evil.example', evidence_class: 'model_summary' },
      detail: { confidence: 'suggestive' },
    }
    const { container } = render(<Card card={card} />)
    expect(screen.getByTestId('provenance-untrusted').textContent).toContain('evil.example')
    // A model summary must not look like a measurement.
    expect(screen.getByText('model summary')).toBeInTheDocument()
    expect(container.querySelector('a')).toBeNull()
  })

  it('the mark survives the Markdown export', () => {
    const d: Dossier = {
      ticket: 'tkt_1',
      verdict: 'inconclusive',
      claims: [
        {
          claim: 'the gap persists',
          evidence_id: 'exp_1',
          n: null,
          se: null,
          verification: 'recheck',
          chain: ['platform', 'untrusted'],
          evidence_class: 'model_summary',
        },
        {
          claim: 'costs are 4bp',
          evidence_id: 'exp_2',
          n: 900,
          se: 0.1,
          verification: 'recheck',
          chain: ['platform'],
          evidence_class: 'direct_stat',
        },
      ],
      plan_as_executed: [],
      funnel: [],
      unresolved_questions: [],
      reviewer_critique: [],
      cost_usd: 1,
      trials: 2,
      provenance: { harness: 'h', profile: 'p', window: 'w', seeds: 's' },
      quoted_untrusted: [],
    }
    expect(chainTouchesUntrusted(d.claims[0])).toBe(true)
    expect(chainTouchesUntrusted(d.claims[1])).toBe(false)
    const md = dossierToMarkdown(d)
    expect(md).toContain('the gap persists **[fetched — untrusted source in the chain]**')
    expect(md).not.toContain('costs are 4bp **[fetched')
    // And the missing n survives too.
    expect(md).toContain('(n unknown)')
  })
})

describe('an irreversible approval cannot be confirmed in one click (§6.7)', () => {
  it('a SealedHoldout approval takes two steps', () => {
    const onApprove = vi.fn()
    const card: DeskCard = {
      seq: 1,
      step: 1,
      kind: 'approval',
      at: new Date().toISOString(),
      headline: 'unseal the holdout',
      detail: { envelope: 'SealedHoldout', approval_id: 'ap_1' },
    }
    render(<Card card={card} actions={{ onApprove }} />)
    fireEvent.click(screen.getByTestId('approve'))
    expect(onApprove).not.toHaveBeenCalled()
    expect(screen.getByTestId('approve').textContent).toContain('cannot be undone')
    fireEvent.click(screen.getByTestId('approve'))
    expect(onApprove).toHaveBeenCalledWith('ap_1', undefined)
  })

  it('a reversible approval is one click', () => {
    const onApprove = vi.fn()
    const card: DeskCard = {
      seq: 1,
      step: 1,
      kind: 'approval',
      at: new Date().toISOString(),
      headline: 'raise the budget',
      detail: { envelope: 'Spend', approval_id: 'ap_2' },
    }
    render(<Card card={card} actions={{ onApprove }} />)
    fireEvent.click(screen.getByTestId('approve'))
    expect(onApprove).toHaveBeenCalledWith('ap_2', undefined)
  })
})
