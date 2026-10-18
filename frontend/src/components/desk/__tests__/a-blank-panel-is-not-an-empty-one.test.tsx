/**
 * Two panels where a failed read drew the reassuring picture (AGENT-012 §2, §10;
 * AGENT-013 §7.5; Set S tasks S-6.20, S-10.9).
 *
 * **The proposals queue rendered nothing at all.** `optional()` folds only
 * 404/405/501 into `null` and rethrows everything else, so a 500, a 401 or a
 * dropped connection left `data.data` as `undefined` — which matched neither the
 * `data.data === null` branch nor the `data.data && pending.length === 0` one.
 * No rows, no error, no empty state: a blank panel, on the queue where a run
 * asks to reach ring 1 or ring 2. Blank is what "nothing is waiting to be
 * decided" looks like, and it is the reading somebody acts on by walking away.
 *
 * **Every provider read as "not configured" when nothing answered.**
 * `configured={s?.configured ?? false}` on the Models page turned a credential
 * list that never came back into a confident claim that the platform holds no
 * key — with a **Connect** button beside it inviting somebody to paste one it
 * may already have. The rule does not only apply to the reassuring direction:
 * the fact is unknown, and the badge said it was known.
 */
import { render, screen, waitFor } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import type { ReactElement } from 'react'
import { describe, expect, it, vi } from 'vitest'

const proposals = vi.hoisted(() => vi.fn())
vi.mock('@/api/desk', () => ({ deskApi: { proposals, decideProposal: vi.fn() } }))
vi.mock('@/api/llm', async (orig) => ({
  ...(await orig<object>()),
  llmKeysApi: { models: vi.fn(() => Promise.resolve([])) },
}))

import { ProposalsQueue } from '../ProposalsQueue'
// The provider card moved to Settings -> AI providers (PROVIDERS track); the rule
// it is held to here did not change.
import { ProviderCard } from '@/components/providers/ProviderCard'
import { PROVIDERS, type ProviderStatus } from '@/api/llm'

function withClient(ui: ReactElement) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: 0 } } })
  return render(<QueryClientProvider client={qc}>{ui}</QueryClientProvider>)
}

describe('the proposals queue says something when it could not be read', () => {
  it('does not render a blank panel on a request that threw', async () => {
    proposals.mockRejectedValue(new Error('500'))
    const { container } = withClient(<ProposalsQueue />)
    await waitFor(() => {
      expect(screen.getByText(/could not be read/)).toBeInTheDocument()
    })
    expect(screen.getByText(/not the same as nothing waiting to be decided/)).toBeInTheDocument()
    // The blank the panel used to be.
    expect(container.textContent).not.toBe('')
    expect(screen.queryByText(/Nothing to decide/)).toBeNull()
  })

  it('still says the endpoint is unavailable on a 404-shaped null', async () => {
    proposals.mockResolvedValue(null)
    withClient(<ProposalsQueue />)
    await waitFor(() => expect(screen.getByText(/Proposals are unavailable/)).toBeInTheDocument())
    expect(screen.queryByText(/Nothing to decide/)).toBeNull()
  })

  it('claims an empty queue only when the endpoint answered with one', async () => {
    proposals.mockResolvedValue({ pending: [] })
    withClient(<ProposalsQueue />)
    await waitFor(() => expect(screen.getByText(/Nothing to decide/)).toBeInTheDocument())
    expect(screen.queryByText(/could not be read/)).toBeNull()
  })

  it('renders the rows when there are rows, so the states above are not the only branch', async () => {
    proposals.mockResolvedValue({
      pending: [
        {
          proposal_id: 'p_1',
          kind: 'deploy_paper',
          ring: 2,
          envelope: 'Promotion',
          subject: 'gap strategy v3',
          subject_id: 'st_3',
          rationale: 'dossier d_9 clears the funnel',
          dossier_ref: 'd_9',
          actionable: true,
        },
      ],
    })
    withClient(<ProposalsQueue />)
    await waitFor(() => expect(screen.getByText('gap strategy v3')).toBeInTheDocument())
    expect(screen.queryByText(/Nothing to decide/)).toBeNull()
  })
})

describe('a provider whose credential state was never read says so', () => {
  const anthropic = PROVIDERS[0]
  const row = (over: Partial<ProviderStatus>): ProviderStatus => ({
    provider: 'anthropic',
    configured: false,
    key_last4: null,
    base_url: null,
    updated_at: null,
    verified_at: null,
    last_checked_at: null,
    last_error: null,
    default_model: null,
    is_default: false,
    ...over,
  })

  it('does not claim the platform holds no key', () => {
    withClient(<ProviderCard meta={anthropic} status={null} canSave />)
    expect(screen.getByText('state unknown')).toBeInTheDocument()
    expect(screen.queryByText('Not connected')).toBeNull()
    // "Save & test" asserts there is nothing stored. Neither is known here,
    // so the form is not drawn.
    expect(screen.queryByRole('button', { name: 'Save & test' })).toBeNull()
  })

  it('still says connected when the list answered that it is', () => {
    withClient(
      <ProviderCard
        meta={anthropic}
        status={row({ configured: true, key_last4: '4f2a', verified_at: new Date().toISOString() })}
        canSave
      />,
    )
    expect(screen.getByText(/key ending ••••4f2a/)).toBeInTheDocument()
  })

  it('still says not connected when the list answered that it is not', () => {
    withClient(<ProviderCard meta={anthropic} status={row({})} canSave />)
    expect(screen.getByText('Not connected')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Save & test' })).toBeInTheDocument()
  })
})
