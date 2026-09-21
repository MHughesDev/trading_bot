/**
 * The provenance fence has to reach the *chip* and the *memory row* too
 * (AGENT-012 §6.3a, §19.1a; Set S task S-6.23).
 *
 * `honesty.test.tsx` already covers the two halves that were built: an untrusted
 * card is marked and renders inert, and the mark survives the Markdown export.
 * These cover the two the spec names in the same sentence and the desk did not
 * carry — "every card, **chip and memory row**".
 *
 * The defect each one catches is a mark that is dropped rather than a mark that
 * is wrong, which is the harder kind to see: nothing looks broken. An id that
 * came out of a fetched page was quoted into findings, gates and the dossier's
 * funnel as bare monospace, identical to an id the platform measured itself; and
 * `MemoryBrowser`'s row passed a literal `null` for its mark, so a memory row
 * could not show one however untrusted the entry was. Both render exactly like
 * the honest case, which is why they survived.
 */
import { render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import { readFileSync } from 'node:fs'
import { dirname, resolve as resolvePath } from 'node:path'
import { fileURLToPath } from 'node:url'

import type { DeskCard, ProvenanceMark } from '@/api/desk'
import { EvidenceChip } from '../Shared'
import { makeResolver } from '../Conversation'
import { citedCount, rowMark, type MemoryRow } from '../MemoryBrowser'

const FETCHED: ProvenanceMark = {
  provenance: 'untrusted',
  source: 'seekingalpha.example',
  fetched_at: '2026-09-20T10:00:00Z',
  evidence_class: 'model_summary',
}

function toolCard(over: Partial<DeskCard> = {}): DeskCard {
  return {
    seq: 1,
    step: 1,
    kind: 'tool',
    at: '2026-09-20T10:00:00Z',
    headline: 'fetch_page',
    detail: { produced_ids: ['exp_fetched'] },
    ...over,
  }
}

describe('a chip carries the provenance of the thing it names (§6.3a)', () => {
  it('marks a resolved id whose source was fetched, and names the host', () => {
    render(
      <EvidenceChip
        id="exp_fetched"
        resolve={() => ({ label: 'returned by fetch_page', kind: 'tool result', mark: FETCHED })}
      />,
    )
    const chip = screen.getByTestId('ev-exp_fetched')
    expect(chip.getAttribute('data-provenance')).toBe('untrusted')
    // The word, not only the border — Meridian principle 3.
    expect(chip.textContent).toContain('fetched')
    expect(chip.textContent).toContain('seekingalpha.example')
    // A model summary must not read as a measurement, on a chip either.
    expect(chip.textContent).toContain('model summary')
  })

  it('says nothing at all when the provenance was never recorded', () => {
    // Absent is not "platform". A chip that printed a trusted provenance it was
    // never given would be the surface inventing the claim the fence exists to
    // keep the model from inventing.
    render(<EvidenceChip id="exp_plain" resolve={() => ({ label: 'an experiment', kind: 'experiment' })} />)
    const chip = screen.getByTestId('ev-exp_plain')
    expect(chip.getAttribute('data-provenance')).toBeNull()
    expect(chip.textContent).not.toContain('fetched')
    expect(chip.textContent).not.toContain('platform')
    // …and it is still a chip, so a passing assertion above cannot be a chip
    // that failed to render.
    expect(chip.getAttribute('data-resolved')).toBe('true')
  })

  it('renders a measured class as a word too, so "measured" and "not recorded" differ', () => {
    render(
      <EvidenceChip
        id="exp_measured"
        resolve={() => ({
          label: 'an experiment',
          kind: 'experiment',
          mark: { provenance: 'platform', evidence_class: 'direct_stat' },
        })}
      />,
    )
    expect(screen.getByTestId('ev-exp_measured').textContent).toContain('measured')
  })
})

describe('the mark travels from the tool result to every id it produced', () => {
  it('gives an id produced by an untrusted tool card that card’s mark', () => {
    const resolve = makeResolver(null, [toolCard({ mark: FETCHED })])
    const hit = resolve('exp_fetched')
    expect(hit).not.toBeNull()
    expect(hit?.mark?.provenance).toBe('untrusted')
    expect(hit?.mark?.source).toBe('seekingalpha.example')
  })

  it('marks the handle as well as the ids — the full result is the fetched page', () => {
    const resolve = makeResolver(null, [
      toolCard({ mark: FETCHED, detail: { produced_ids: ['exp_fetched'], handle: 'h_99' } }),
    ])
    expect(resolve('h_99')?.mark?.provenance).toBe('untrusted')
  })

  it('leaves an id from a tool with no mark unmarked rather than trusted', () => {
    const resolve = makeResolver(null, [toolCard()])
    const hit = resolve('exp_fetched')
    expect(hit).not.toBeNull() // it resolves…
    expect(hit?.mark ?? null).toBeNull() // …and claims nothing about its origin
  })

  it('still strikes an id no tool produced, marked or not', () => {
    const resolve = makeResolver(null, [toolCard({ mark: FETCHED })])
    expect(resolve('exp_invented')).toBeNull()
  })
})

describe('a memory row can show its mark, and its cited count', () => {
  function row(over: Partial<MemoryRow> = {}): MemoryRow {
    return { id: 'm_1', subject: 'BTC', claim: 'the gap pays', ...over }
  }

  it('reads the mark off the row instead of a hardcoded null', () => {
    expect(rowMark(row({ mark: FETCHED }))?.provenance).toBe('untrusted')
  })

  it('carries the evidence class alone when that is all the row records', () => {
    expect(rowMark(row({ evidence_class: 'model_summary' }))?.evidence_class).toBe('model_summary')
  })

  it('returns null when the row records neither, rather than defaulting to trusted', () => {
    expect(rowMark(row())).toBeNull()
  })

  it('reads the cited count under the name the API actually serves', () => {
    // The row was read as `cited` and served as `cited_by_chain`, so the count
    // never rendered anywhere — a silent miss, not a visible one.
    expect(citedCount(row({ cited_by_chain: 9 }))).toBe(9)
    expect(citedCount(row({ cited: 3 }))).toBe(3)
  })

  it('keeps zero citations distinct from an unrecorded count', () => {
    expect(citedCount(row({ cited_by_chain: 0 }))).toBe(0)
    expect(citedCount(row())).toBeNull()
  })
})

describe('the mark survives the PDF export, which is the print stylesheet (§19.1a)', () => {
  // "Export PDF" is `window.print()`. There was no `@media print` block at all,
  // so the export was the app shell around the document — and the mark is drawn
  // with a dashed coloured border, which browsers drop from a print by default.
  // A mark that survives on screen and vanishes on paper fails the acceptance
  // exactly where it matters: the reviewer reading the PDF a month later is the
  // one who can no longer ask the run.
  const here = dirname(fileURLToPath(import.meta.url))
  const css = readFileSync(resolvePath(here, '../../../styles/desk.css'), 'utf8')

  it('has a print block at all', () => {
    expect(css).toContain('@media print')
  })

  it('forces the provenance mark to print rather than letting the browser drop it', () => {
    const print = css.slice(css.indexOf('@media print'))
    expect(print).toMatch(/\.prov-mark[^{]*\{[^}]*print-color-adjust:\s*exact/)
    expect(print).toContain('.ev-chip.untrusted')
    // The struck-through "not found" chip is the other honesty mark on the page.
    expect(print).toMatch(/\.ev-chip\.missing[^{]*\{[^}]*line-through/)
  })

  it('drops the app chrome so the exported page is the document', () => {
    const print = css.slice(css.indexOf('@media print'))
    for (const chrome of ['.desk-rail', '.desk-side', '.desk-dock']) {
      expect(print).toContain(chrome)
    }
  })
})
