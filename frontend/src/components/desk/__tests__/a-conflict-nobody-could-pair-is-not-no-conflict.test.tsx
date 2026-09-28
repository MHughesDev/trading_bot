/**
 * S-6.18's two acceptance clauses, and the defect the first one was hiding
 * (AGENT-012 §7; task S-6.18).
 *
 * **Acceptance:** *"a conflicting pair renders side by side and neither is hidden
 * by ranking; every row opens its provenance (run, tool, data window)."* Neither
 * clause had a test. `grep -rn "conflict"` over the whole test suite returned one
 * line, in a different file, about a count.
 *
 * Writing them found the defect in the function whose job was the clause.
 * `conflictPairs` resolved the id a row names against the rows it was given and
 * did `if (!o) continue` — so a row declaring `conflicts_with: ['f_99']` where
 * `f_99` is in a different bucket, or was simply not in the response, **dropped
 * the conflict entirely**. The consequences compound:
 *
 *  - the pair does not render, so the row keeping the disagreement is invisible
 *    in the view that exists to show disagreements;
 *  - `Conflicts (0)` renders as a measured zero over data that declares one;
 *  - and the empty state under it reads "Nothing in this bucket disagrees with
 *    anything else" — a claim the page has contradicting evidence for.
 *
 * That is this file's own header argument turned inside out: the reference could
 * not be resolved, and the surface reported the absence of the thing referred to.
 * A dangling counterpart is now returned and named.
 */
import { fireEvent, render, screen } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import type { ReactElement } from 'react'
import { describe, expect, it, vi } from 'vitest'

const memory = vi.hoisted(() => vi.fn())
vi.mock('@/api/desk', () => ({ deskApi: { memory } }))

import { MemoryBrowser, type MemoryRow } from '../MemoryBrowser'
import { conflictPairs } from '../MemoryBrowser.helpers'

function row(over: Partial<MemoryRow> & { id: string }): MemoryRow {
  return {
    claim: `the claim of ${over.id}`,
    subject: 'BTC-USD',
    ...over,
  } as MemoryRow
}

function withClient(ui: ReactElement) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: 0 } } })
  return render(<QueryClientProvider client={qc}>{ui}</QueryClientProvider>)
}

describe('conflictPairs keeps the disagreement even when it cannot pair it', () => {
  it('pairs both rows when the bucket holds both', () => {
    const a = row({ id: 'f_1', conflicts_with: ['f_2'] })
    const b = row({ id: 'f_2' })
    const pairs = conflictPairs([a, b])
    expect(pairs).toHaveLength(1)
    expect(pairs[0].row.id).toBe('f_1')
    expect(pairs[0].otherId).toBe('f_2')
    expect(pairs[0].other?.id).toBe('f_2')
  })

  it('[was dropped] keeps a conflict whose counterpart is not in this bucket', () => {
    const only = row({ id: 'f_1', conflicts_with: ['f_99'] })
    const pairs = conflictPairs([only])
    expect(pairs).toHaveLength(1)
    expect(pairs[0].otherId).toBe('f_99')
    expect(pairs[0].other).toBeNull()
  })

  it('counts a mutually declared conflict once, not twice', () => {
    const a = row({ id: 'f_1', conflicts_with: ['f_2'] })
    const b = row({ id: 'f_2', conflicts_with: ['f_1'] })
    expect(conflictPairs([a, b])).toHaveLength(1)
  })

  it('does not turn a malformed self-reference into a disagreement', () => {
    expect(conflictPairs([row({ id: 'f_1', conflicts_with: ['f_1'] })])).toEqual([])
  })

  it('keeps every conflict a row declares, so one is not ranked out by another', () => {
    const a = row({ id: 'f_1', conflicts_with: ['f_2', 'f_3'] })
    const pairs = conflictPairs([a, row({ id: 'f_2' }), row({ id: 'f_3' })])
    expect(pairs.map((p) => p.otherId).sort()).toEqual(['f_2', 'f_3'])
  })
})

describe('a conflicting pair renders side by side', () => {
  function openConflicts() {
    // The one button that switches the view, found by its own label.
    fireEvent.click(screen.getByRole('button', { name: /^Conflicts/ }))
  }

  it('draws both rows in one section, neither below the other', async () => {
    memory.mockResolvedValue({
      ok: true,
      data: {
        rows: [
          row({ id: 'f_1', claim: 'the gap pays after costs', conflicts_with: ['f_2'] }),
          row({ id: 'f_2', claim: 'the gap does not pay after costs' }),
        ],
      },
    })
    withClient(<MemoryBrowser />)
    expect(await screen.findByText('the gap pays after costs')).toBeInTheDocument()
    openConflicts()
    const section = await screen.findByLabelText('conflict between f_1 and f_2')
    // Both claims inside the *same* section element: that is what "side by side"
    // means in a DOM, and it is what a ranked list cannot do.
    expect(section.textContent).toContain('the gap pays after costs')
    expect(section.textContent).toContain('the gap does not pay after costs')
    expect(section.querySelectorAll('.desk-card').length).toBeGreaterThanOrEqual(2)
  })

  it('[was invisible] shows the row and names the half the bucket does not hold', async () => {
    memory.mockResolvedValue({
      ok: true,
      data: { rows: [row({ id: 'f_1', claim: 'the gap pays after costs', conflicts_with: ['f_99'] })] },
    })
    withClient(<MemoryBrowser />)
    expect(await screen.findByText('the gap pays after costs')).toBeInTheDocument()
    openConflicts()
    const section = await screen.findByLabelText('conflict between f_1 and f_99')
    expect(section.textContent).toContain('the gap pays after costs')
    expect(screen.getByText('f_99 is not in this bucket')).toBeInTheDocument()
    // And the count is the count of declared conflicts, not of pairable ones.
    expect(screen.getByRole('button', { name: /^Conflicts/ }).textContent).toContain('(1)')
    // The sentence that used to render instead.
    expect(screen.queryByText(/Nothing in this bucket disagrees/)).toBeNull()
  })

  it('still says there is no conflict when the bucket really declares none', async () => {
    memory.mockResolvedValue({ ok: true, data: { rows: [row({ id: 'f_1' })] } })
    withClient(<MemoryBrowser />)
    expect(await screen.findByText('the claim of f_1')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: /^Conflicts/ }).textContent).toContain('(0)')
    openConflicts()
    expect(await screen.findByText('No conflicts')).toBeInTheDocument()
  })

  it('does not claim a count over a bucket it could not read', async () => {
    // The pre-existing rule, pinned here too: an unreadable bucket has no number.
    memory.mockResolvedValue({ ok: false, status: 405 })
    withClient(<MemoryBrowser />)
    expect(await screen.findByRole('button', { name: /Conflicts \(unknown\)/ })).toBeInTheDocument()
  })
})

describe('every row opens its provenance', () => {
  it('names the run, the tool and the data window on each row', async () => {
    memory.mockResolvedValue({
      ok: true,
      data: {
        rows: [row({ id: 'f_1', provenance: { run: 'r_42', tool: 'run_sweep', window: '2024-01..2024-06' } })],
      },
    })
    withClient(<MemoryBrowser />)
    const line = await screen.findByText(/provenance: run r_42/)
    expect(line.textContent).toContain('run_sweep')
    expect(line.textContent).toContain('2024-01..2024-06')
  })

  it('says which part was never recorded rather than leaving it blank', async () => {
    // A blank where the tool should be reads as a row with no tool. "unknown" is
    // the only honest word for a field the store did not carry.
    memory.mockResolvedValue({ ok: true, data: { rows: [row({ id: 'f_1' })] } })
    withClient(<MemoryBrowser />)
    const line = await screen.findByText(/provenance: run unknown/)
    expect(line.textContent).toContain('tool unknown')
    expect(line.textContent).toContain('unknown')
  })

  it('carries the provenance onto both halves of a conflict, which is where it is read', async () => {
    memory.mockResolvedValue({
      ok: true,
      data: {
        rows: [
          row({ id: 'f_1', conflicts_with: ['f_2'], provenance: { run: 'r_1', tool: 'run_sweep', window: 'w1' } }),
          row({ id: 'f_2', provenance: { run: 'r_2', tool: 'run_backtest', window: 'w2' } }),
        ],
      },
    })
    withClient(<MemoryBrowser />)
    expect(await screen.findByText('the claim of f_1')).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: /^Conflicts/ }))
    const section = await screen.findByLabelText('conflict between f_1 and f_2')
    expect(section.textContent).toContain('run r_1')
    expect(section.textContent).toContain('run r_2')
  })
})
