/**
 * The replay scrubber reported the platform's missing route as a fact about the
 * run (AGENT-012 §4, §19.4; Set S task S-6.16).
 *
 * `RunDetail` read `deskApi.trajectory`, which is an `optional()` call, and
 * `optional()` answers `null` for a 404 and the decoded body for anything else.
 * Both fell through `traj.data?.steps ?? []` into the same empty state, whose
 * headline was:
 *
 *   > No trajectory is stored for this run
 *
 * — a claim about the run, made by a panel with no source for one.
 *
 * Verified against the running platform on 2026-09-22 rather than taken from
 * the plan: `GET /api/agent/runs/{id}/trajectory` answers **404** on :7080,
 * where every mounted route under that prefix answers 401
 * (`/api/agent/runs/{id}` does). So on this deployment the unmounted branch is
 * the only branch that renders, and the sentence it replaced was wrong on every
 * run the desk has ever opened. `…/state`, `…/lifecycle`, `…/pause`, `…/resume`
 * and `…/stop` answer 404 too; `…/{id}` and `/api/agent/runs` answer 401.
 *
 * The two sentences have to stay two, because they send the reader to opposite
 * places: one to AGENT-010 §5, and one to the run.
 */
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'

const trajectory = vi.fn()
vi.mock('@/api/desk', async (orig) => {
  const real = await orig<typeof import('@/api/desk')>()
  return { ...real, deskApi: { ...real.deskApi, trajectory: (id: string) => trajectory(id) } }
})

const { RunDetail } = await import('../RunDetail')

async function open() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  render(
    <QueryClientProvider client={qc}>
      <RunDetail runId="run_1" onBack={() => undefined} />
    </QueryClientProvider>,
  )
  await screen.findByRole('button', { name: /Back to the conversation/ })
}

describe('an empty scrubber says whose emptiness it is', () => {
  it('blames the unmounted route, not the run, when nothing answered', async () => {
    // `optional()` turns the platform's 404 into this `null`.
    trajectory.mockResolvedValueOnce(null)
    await open()
    expect(screen.getByText(/Nothing here can report what this run did/)).toBeInTheDocument()
    expect(screen.getByText(/answers 404 where every mounted route answers 401/)).toBeInTheDocument()
    expect(screen.queryByText(/No trajectory is stored for this run/)).toBeNull()
  })

  it('reports a run with no steps as a fact about the run, once the route answers', async () => {
    trajectory.mockResolvedValueOnce({ steps: [] })
    await open()
    expect(screen.getByText(/This run has no stored steps/)).toBeInTheDocument()
    expect(screen.getByText(/a fact about the run rather than about the platform/)).toBeInTheDocument()
    expect(screen.queryByText(/does not mount/)).toBeNull()
  })

  it('gives the two absences two different headlines', async () => {
    trajectory.mockResolvedValueOnce(null)
    await open()
    const unmounted = screen.getByText(/Nothing here can report/).textContent
    screen.getByRole('button', { name: /Back to the conversation/ })
    expect(unmounted).not.toContain('no stored steps')
  })

  it('does not claim to have written a file it never wrote', async () => {
    // "Save as case" was `window.alert('Trajectory written to evals/ …')` in
    // front of a handler that called nothing, and `GET /api/evals` answers 404
    // on the live platform. Held here so it cannot come back.
    trajectory.mockResolvedValueOnce(null)
    await open()
    expect(screen.queryByText(/Save as case/)).toBeNull()
    expect(screen.queryByText(/written to/)).toBeNull()
  })
})
