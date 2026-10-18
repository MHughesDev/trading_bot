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
import { fireEvent, within } from '@testing-library/react'

const trajectory = vi.fn()
// The scrubber falls back to the driver's `decision` rows on the messages
// route when the trajectory route is unmounted; by default this run has none.
const messages = vi.fn((..._args: unknown[]) => Promise.resolve({ data: { messages: [] as unknown[], last_seq: 0 } }))
vi.mock('@/api/desk', async (orig) => {
  const real = await orig<typeof import('@/api/desk')>()
  return {
    ...real,
    deskApi: {
      ...real.deskApi,
      trajectory: (id: string) => trajectory(id),
      messages: (id: string, after?: number) => messages(id, after),
    },
  }
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

describe('the scrubber replays the driver decision log when the trajectory is unmounted', () => {
  const decision = (seq: number, step: number, phase: string, prompt: string, layers?: Record<string, number>) => ({
    seq,
    kind: 'decision',
    created_at: '2026-10-03T10:00:00Z',
    content: {
      step,
      phase,
      tool: null,
      system: 'You are the research agent.\n  Keep indentation.',
      prompt,
      schema: phase === 'native' ? null : {},
      raw_reply: '{"tool":"run_backtest"}',
      usage: { input_tokens: 1234, output_tokens: 56 },
      ...(layers ? { layers } : {}),
    },
  })

  it('shows the stored prompt byte-for-byte and says when it is the request', async () => {
    trajectory.mockResolvedValueOnce(null)
    const prompt = 'Step 1\n\t  exactly   these   bytes\n'
    messages.mockResolvedValueOnce({
      data: {
        messages: [
          { seq: 1, kind: 'stage', created_at: '2026-10-03T10:00:00Z', content: { stage: 'design' } },
          decision(2, 1, 'select_tool', prompt, { charter: 100, schemas: 34, body: 1000, output: 512 }),
          decision(3, 2, 'native', 'fallback only'),
        ],
        last_seq: 3,
      },
    })
    messages.mockResolvedValueOnce({ data: { messages: [], last_seq: 3 } })
    const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })
    render(
      <QueryClientProvider client={qc}>
        <RunDetail runId="run_1" onBack={() => undefined} />
      </QueryClientProvider>,
    )
    const pre = await screen.findByTestId('replay-prompt')
    expect(pre.textContent).toBe(prompt)
    expect(screen.getByTestId('replay-system').textContent).toBe('You are the research agent.\n  Keep indentation.')
    expect(screen.getByTestId('replay-faithful')).toBeInTheDocument()
    // S-6.16: the driver's per-layer estimate is shown beside the provider's count.
    const split = screen.getByTestId('replay-layer-split').textContent ?? ''
    expect(split).toMatch(/charter 100 · schemas 34 · body\s+1000 \(= 1134 prompt tokens, 92% of the provider count\)/)
    expect(split).toMatch(/output cap 512/)
    // The native call is labelled, not passed off as what the model saw, and its
    // row (no layers) says they were not recorded rather than inventing a split.
    fireEvent.click(screen.getByRole('button', { name: '2' }))
    expect(screen.queryByTestId('replay-layer-split')).toBeNull()
    expect(screen.getByTestId('replay-layers-unrecorded')).toBeInTheDocument()
    expect(screen.getByTestId('replay-unfaithful')).toBeInTheDocument()
    // Stages: the one reached is enabled, the rest are drawn and disabled.
    const stages = screen.getByTestId('replay-stages')
    expect(within(stages).getByRole('button', { name: 'design' })).not.toBeDisabled()
    expect(within(stages).getByRole('button', { name: 'propose' })).toBeDisabled()
    fireEvent.click(within(stages).getByRole('button', { name: 'design' }))
    expect(screen.getByTestId('replay-faithful')).toBeInTheDocument()
  })
})
