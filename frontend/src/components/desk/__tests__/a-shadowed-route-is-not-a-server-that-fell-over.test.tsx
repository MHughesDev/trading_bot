/**
 * The memory browser's seven tabs, and the one of them that cannot be read at
 * all (AGENT-012 §7; Set S task S-6.18).
 *
 * # What the running platform says, which no plan file records
 *
 * Probed against the live API on :7080, unauthenticated, where a mounted route
 * answers 401 and an absent one answers 404:
 *
 *     401  /api/memory/findings      401  /api/memory/questions
 *     401  /api/memory/negative      405  /api/memory/preferences   ← this one
 *     401  /api/memory/profiles      401  /api/memory/defects
 *                                    401  /api/memory/signals
 *
 * **405**, not 401 and not 404. `crates/api/src/routes/mod.rs` registers
 *
 *     .route("/api/memory/preferences", post(memory::record_preference))   // line 234
 *     .route("/api/memory/{bucket}",    get(memory::list))                 // line 240
 *
 * and axum's matcher prefers a static segment over a parameter, so a **GET** to
 * that path is taken by the literal route, which has no GET, and refused before
 * it can reach the list handler. One of the seven tabs is unreachable and it is
 * a route-table collision, not a bug in this component and not something a
 * frontend change can work around.
 *
 * # Two failures it was arriving as, both reassuring
 *
 * `deskApi.memory` went through `optional()`, which folds 404/405/501 to `null`
 * and **rethrows everything else**. So:
 *
 *   * the 405 rendered *"/api/memory/{bucket} is served, so this is the endpoint
 *     failing rather than the endpoint missing — check the API logs"*, sending
 *     the reader to a log that contains nothing, because nothing went wrong;
 *   * and a **500 or a dropped connection rethrew**, leaving `data.data` as
 *     `undefined`, which matched neither the `null` branch nor anything else —
 *     and fell through to *"Nothing is recorded under findings"*. A server that
 *     fell over rendered as an empty research record. That is the reassuring
 *     direction, on the page a person uses to decide whether a question has
 *     already been answered.
 *
 * Plus the counts beside them: `Conflicts (0)` was printed over a bucket nobody
 * could read — a measured zero standing in for an absent measurement, and the
 * particular zero that claims your memory does not disagree with itself.
 *
 * WHICH OF THESE WOULD HAVE FAILED BEFORE — checked by reverting `api/desk.ts`
 * and `MemoryBrowser.tsx` in the working copy and running this file, not
 * assumed. Every test in it fails: `BucketRefusal` did not exist, and the two
 * browser tests fail on the old text ("Nothing is recorded under findings" for
 * the thrown read, "check the API logs" for the 405).
 */
import { render, screen, waitFor } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import type { ReactElement } from 'react'
import { describe, expect, it, vi } from 'vitest'

const memory = vi.hoisted(() => vi.fn())
vi.mock('@/api/desk', () => ({ deskApi: { memory } }))

import { BucketRefusal, MemoryBrowser } from '../MemoryBrowser'

function withClient(ui: ReactElement) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: 0 } } })
  return render(<QueryClientProvider client={qc}>{ui}</QueryClientProvider>)
}

describe('a 405 is a route-table collision, not a server that fell over', () => {
  it('names the collision and the two routes that cause it', () => {
    render(<BucketRefusal bucket="preferences" status={405} />)
    expect(screen.getByText(/405 Method Not Allowed/)).toBeInTheDocument()
    expect(screen.getByText(/POST \/api\/memory\/preferences/)).toBeInTheDocument()
  })

  it('does not send the reader to a log that has nothing in it', () => {
    // The old sentence ended "check the API logs". It is the opposite of
    // useful here: nothing went wrong, so nothing was logged, and the reader
    // spends their time in the one place that cannot tell them.
    render(<BucketRefusal bucket="preferences" status={405} />)
    expect(screen.queryByText(/check the API logs/i)).toBeNull()
    expect(screen.getByText(/nothing in the API log to find/)).toBeInTheDocument()
    expect(screen.getByText(/needs the route table changed/)).toBeInTheDocument()
  })

  it('says the entries are not lost, only unreadable from here', () => {
    // The distinction somebody acts on. "Cannot be read" invites a second look;
    // "your preferences are gone" invites re-recording them.
    render(<BucketRefusal bucket="preferences" status={405} />)
    expect(screen.getByText(/not lost, only unreadable/)).toBeInTheDocument()
  })
})

describe('the other refusals each get their own sentence', () => {
  it('reads a 404 as a route nothing mounts', () => {
    render(<BucketRefusal bucket="findings" status={404} />)
    expect(screen.getByText(/not mounted on this platform/)).toBeInTheDocument()
  })

  it('reads a 401 as a credential rather than as a missing store', () => {
    render(<BucketRefusal bucket="findings" status={401} />)
    expect(screen.getByText(/refused this read for want of a credential/)).toBeInTheDocument()
  })

  it('sends a 500 to the API log, which is the one case where there is something in it', () => {
    render(<BucketRefusal bucket="findings" status={500} />)
    expect(screen.getByText(/answered 500/)).toBeInTheDocument()
    expect(screen.getByText(/the API log has it/)).toBeInTheDocument()
  })

  it('distinguishes a request that carried no status at all', () => {
    render(<BucketRefusal bucket="findings" status={null} />)
    expect(screen.getByText(/did not complete and carried no status/)).toBeInTheDocument()
  })
})

describe('a bucket that could not be read is not a bucket with nothing in it', () => {
  it('does not claim an empty research record when the read failed', async () => {
    memory.mockResolvedValue({ ok: false, status: 500 })
    withClient(<MemoryBrowser />)
    await waitFor(() => expect(screen.getByText(/Memory is unavailable/)).toBeInTheDocument())
    expect(screen.queryByText(/Nothing is recorded under findings/)).toBeNull()
  })

  it('still claims an empty bucket when the store answered with one', async () => {
    memory.mockResolvedValue({ ok: true, data: { rows: [] } })
    withClient(<MemoryBrowser />)
    await waitFor(() =>
      expect(screen.getByText(/Nothing is recorded under findings/)).toBeInTheDocument(),
    )
  })

  it('does not count conflicts it could not look for', async () => {
    // `Conflicts (0)` over a bucket nobody read is the claim that your memory
    // does not disagree with itself, asserted from an empty array that is the
    // shape of a failure.
    memory.mockResolvedValue({ ok: false, status: 405 })
    withClient(<MemoryBrowser />)
    await waitFor(() => expect(screen.getByText('Conflicts (unknown)')).toBeInTheDocument())
    expect(screen.queryByText('Conflicts (0)')).toBeNull()
  })

  it('counts them when it read them', async () => {
    memory.mockResolvedValue({ ok: true, data: { rows: [] } })
    withClient(<MemoryBrowser />)
    await waitFor(() => expect(screen.getByText('Conflicts (0)')).toBeInTheDocument())
  })

  it('does not stack a row count under the refusal that contradicts it', async () => {
    // Found by rendering the old component against a 405 rather than by
    // reading it: under "Memory is unavailable" it also printed **"0 rows are
    // in this bucket and none matches the filter"** — a second empty state,
    // asserting a count, disagreeing with the sentence above it about whether
    // anything is known at all.
    memory.mockResolvedValue({ ok: false, status: 405 })
    const { container } = withClient(<MemoryBrowser />)
    await waitFor(() => expect(screen.getByText(/405 Method Not Allowed/)).toBeInTheDocument())
    expect(screen.queryByText(/No row matches this search/)).toBeNull()
    expect(container.textContent).not.toContain('rows are in this bucket')
  })
})
