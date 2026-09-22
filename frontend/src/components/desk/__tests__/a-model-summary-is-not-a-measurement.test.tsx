/**
 * The memory row's provenance mark, checked where a person meets it
 * (AGENT-012 §6.3a, §7, §19.1a; Set S tasks S-6.18 and S-6.23).
 *
 * `rowMark` was written ready for `evidence_class` and covered by a unit test,
 * and `/api/memory/{bucket}` only began serving the field on 2026-09-21. A field
 * arriving and a field *reaching the reader* are two different claims, and the
 * gap between them is one `<ProvenanceBadge>` nobody had rendered since the
 * field existed. §6.3a's acceptance is visual — "`model_summary` and
 * `direct_stat` are visually distinct" — so it is asserted on the rendered row
 * rather than on the function that feeds it.
 *
 * The browser's empty states are here for the reason this file's subject is:
 * a bucket that answered with no rows rendered as **blank space**, which is the
 * same blank as a request in flight and as one that failed. `MemoryBrowser`'s
 * own header argues exactly this distinction about the server's refusals — "an
 * empty list would read as 'you have recorded no playbooks'" — and then the
 * component made the reader guess between three of them.
 *
 * `deskApi.memory` now answers a `StatusRead` — `{ ok: true, data }` or
 * `{ ok: false, status }` — rather than `T | null`, because one of the seven
 * buckets answers **405** and the folded `null` sent the reader to an API log
 * with nothing in it. The mocks below carry the new shape; the claims are
 * unchanged.
 */
import { render, screen, waitFor } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { MemoryBrowser, MemoryRowCard, type MemoryRow } from '../MemoryBrowser'

const memory = vi.hoisted(() => vi.fn())
vi.mock('@/api/desk', () => ({ deskApi: { memory } }))

function row(over: Partial<MemoryRow> = {}): MemoryRow {
  return { id: 'm_1', subject: 'BTC-USD', claim: 'the overnight gap pays', ...over }
}

function withClient(ui: React.ReactElement) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(<QueryClientProvider client={qc}>{ui}</QueryClientProvider>)
}

describe('a model summary does not look like a measurement on a memory row', () => {
  it('names a direct_stat as measured', () => {
    render(<MemoryRowCard row={row({ evidence_class: 'direct_stat' })} />)
    expect(screen.getByText('measured')).toBeInTheDocument()
    expect(screen.queryByText('model summary')).toBeNull()
  })

  it('names a model_summary as one, in words rather than only a hue', () => {
    render(<MemoryRowCard row={row({ evidence_class: 'model_summary' })} />)
    expect(screen.getByText('model summary')).toBeInTheDocument()
    expect(screen.queryByText('measured')).toBeNull()
  })

  it('says nothing rather than inventing a class the row does not carry', () => {
    render(<MemoryRowCard row={row()} />)
    expect(screen.queryByText('measured')).toBeNull()
    expect(screen.queryByText('model summary')).toBeNull()
    // …and the row itself rendered, so the two absences above are not an
    // assertion passing against a component that drew nothing.
    expect(screen.getByText('the overnight gap pays')).toBeInTheDocument()
  })

  it('still flags an untrusted row, and names the host', () => {
    render(
      <MemoryRowCard
        row={row({
          mark: {
            provenance: 'untrusted',
            source: 'seekingalpha.example',
            fetched_at: '2026-09-20T10:00:00Z',
            evidence_class: 'model_summary',
          },
        })}
      />,
    )
    expect(screen.getByTestId('provenance-untrusted').textContent).toContain('seekingalpha.example')
    expect(screen.getByText('model summary')).toBeInTheDocument()
  })
})

describe('the evidence class survives the trip from the endpoint to the badge', () => {
  beforeEach(() => memory.mockReset())

  it('marks the row the store served, without the component being told twice', async () => {
    // The wire shape `crates/api/src/routes/memory.rs` `row()` builds, including
    // the field it did not serve until 2026-09-21.
    memory.mockResolvedValue({
      ok: true,
      data: {
        rows: [
          {
            id: 'm_1',
            subject: 'BTC-USD',
            claim: 'the overnight gap pays',
            confidence: 'suggestive',
            evidence_class: 'model_summary',
            n: null,
            provenance: { run: 'run_9', tool: 'fetch_page' },
          },
        ],
      },
    })
    withClient(<MemoryBrowser />)
    await waitFor(() => expect(screen.getByText('model summary')).toBeInTheDocument())
    expect(screen.getByText('the overnight gap pays')).toBeInTheDocument()
  })
})

describe('an empty bucket is not the same blank as a failed read', () => {
  beforeEach(() => memory.mockReset())

  it('says the store answered with nothing', async () => {
    memory.mockResolvedValue({ ok: true, data: { rows: [] } })
    withClient(<MemoryBrowser />)
    await waitFor(() =>
      expect(screen.getByText(/Nothing is recorded under findings/)).toBeInTheDocument(),
    )
    expect(screen.queryByText(/Memory is unavailable/)).toBeNull()
  })

  it('says the endpoint did not answer, which is a different sentence', async () => {
    memory.mockResolvedValue({ ok: false, status: 500 })
    withClient(<MemoryBrowser />)
    await waitFor(() => expect(screen.getByText(/Memory is unavailable/)).toBeInTheDocument())
    expect(screen.queryByText(/Nothing is recorded under/)).toBeNull()
  })

  it('counts the rows a filter hid rather than showing a blank list', async () => {
    memory.mockResolvedValue({
      ok: true,
      data: { rows: [{ id: 'm_1', subject: 'BTC-USD', claim: 'the gap pays' }] },
    })
    const { getByLabelText } = withClient(<MemoryBrowser />)
    await waitFor(() => expect(screen.getByText('the gap pays')).toBeInTheDocument())
    const search = getByLabelText('Search memory') as HTMLInputElement
    search.focus()
    const { fireEvent } = await import('@testing-library/react')
    fireEvent.change(search, { target: { value: 'zzz' } })
    await waitFor(() => expect(screen.getByText(/No row matches this search/)).toBeInTheDocument())
    expect(screen.queryByText(/Nothing is recorded under/)).toBeNull()
  })
})
