/**
 * N-5.8: promotion is a proposal. The page offers "Propose promotion", shows the
 * gate and the verdict as evidence (a fail is visible), has NO direct promote
 * control, and offers approve / reject only on a proposal that is pending.
 */
import { fireEvent, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { never, ok, renderRoute, verdict, version } from './testkit'

const studio = vi.hoisted(() => ({ promotionEvidence: vi.fn(), proposals: vi.fn(), propose: vi.fn(), decide: vi.fn() }))
const mlops = vi.hoisted(() => ({ listVersions: vi.fn(), getAliases: vi.fn(), listDeployments: vi.fn(), promote: vi.fn() }))
vi.mock('@/api/modelsStudio', () => ({ studioApi: studio }))
vi.mock('@/api/mlops', () => ({ modelsApi: mlops }))

import { PromotionPanel } from '../PromotionPanel'

const proposal = (over: Record<string, unknown> = {}) => ({
  proposal_id: 'p1',
  model_id: 'm1',
  version: 2,
  environment: 'paper',
  rationale: 'Beats HARQ out of sample',
  status: 'pending',
  proposed_by: 'agent:research',
  proposed_at: '2026-10-04T09:00:00Z',
  decided_by: null,
  decided_at: null,
  decision_note: null,
  performed_at: null,
  effect: null,
  ...over,
})

const evidence = (over: Record<string, unknown> = {}) => ({
  model_id: 'm1',
  version: 2,
  environment: 'paper',
  gate_checks: [
    { check: 'has_evaluation', passed: true, required: true },
    { check: 'regression_clear', passed: false, required: true },
  ],
  gate_passed: false,
  verdict: verdict(),
  aliases: {},
  pending_proposals: [],
  ...over,
})

const show = () => renderRoute(<PromotionPanel modelId="m1" />)

beforeEach(() => {
  vi.clearAllMocks()
  mlops.listVersions.mockReturnValue(ok({ versions: [version(1), version(2)] }))
  mlops.getAliases.mockReturnValue(ok({ production: 1 }))
  mlops.listDeployments.mockReturnValue(ok({ deployments: [{ deployment_id: 'd1', version: 1, environment: 'paper', alias: 'production', status: 'active', traffic_pct: 100, deployed_at: '2026-10-01T00:00:00Z' }] }))
  studio.promotionEvidence.mockReturnValue(ok(evidence()))
  studio.proposals.mockReturnValue(ok({ proposals: [] }))
  studio.propose.mockReturnValue(ok({ proposal_id: 'p9', status: 'pending' }))
  studio.decide.mockReturnValue(ok({}))
})

describe('the promotion page', () => {
  it('shows skeletons while the evidence loads', () => {
    studio.promotionEvidence.mockReturnValue(never())
    show()
    expect(document.querySelector('.skel')).not.toBeNull()
  })

  it('shows an error that offers no proposal when the evidence cannot be read', async () => {
    studio.promotionEvidence.mockRejectedValue(new Error('x'))
    show()
    expect((await screen.findAllByRole('alert'))[0]).toHaveTextContent('promotion evidence could not be read')
    expect(screen.queryByRole('button', { name: 'Propose promotion' })).toBeNull()
  })

  it('says there are no versions or proposals or deployments, each in its own panel', async () => {
    mlops.listVersions.mockReturnValue(ok({ versions: [] }))
    mlops.getAliases.mockReturnValue(ok({}))
    mlops.listDeployments.mockReturnValue(ok({ deployments: [] }))
    show()
    expect(await screen.findByText('No proposals')).toBeInTheDocument()
    expect(screen.getByText('No deployments')).toBeInTheDocument()
    expect(screen.getAllByText(/No version/).length).toBeGreaterThan(0)
  })

  it('shows the gate checks, the failing verdict rule and the baselines as evidence', async () => {
    show()
    const checks = await screen.findAllByTestId('gate-check')
    expect(checks).toHaveLength(2)
    expect(checks.find((c) => c.textContent?.includes('regression_clear'))).toHaveAttribute('data-passed', 'false')
    expect(screen.getByTestId('gate-status')).toHaveTextContent('Gate not passed')
    const vol = screen.getAllByTestId('usage-rule').find((r) => r.dataset.rule === 'vol_forecast')!
    expect(vol).toHaveTextContent('Fail: blocks')
    expect(screen.getAllByTestId('baseline-row')).toHaveLength(1)
  })

  it('has no direct promote control, only Propose promotion', async () => {
    show()
    await screen.findByTestId('gate-status')
    expect(screen.queryByRole('button', { name: /^promote/i })).toBeNull()
    expect(screen.getByRole('button', { name: 'Propose promotion' })).toBeInTheDocument()
    expect(mlops.promote).not.toHaveBeenCalled()
  })

  it('needs a rationale, then posts the proposal for the version the evidence is about', async () => {
    show()
    await screen.findByTestId('gate-status')
    const btn = screen.getByRole('button', { name: 'Propose promotion' })
    expect(btn).toBeDisabled()
    fireEvent.change(screen.getByLabelText('Rationale'), { target: { value: 'Beats HARQ' } })
    fireEvent.change(screen.getByLabelText('Environment'), { target: { value: 'live' } })
    fireEvent.click(btn)
    await waitFor(() => expect(studio.propose).toHaveBeenCalledWith('m1', 2, { environment: 'live', rationale: 'Beats HARQ' }))
    expect(await screen.findByText('Proposal recorded as pending.')).toBeInTheDocument()
  })

  it('shows approve and reject on a pending proposal, and not on a decided one', async () => {
    studio.proposals.mockReturnValue(ok({ proposals: [proposal(), proposal({ proposal_id: 'p0', status: 'approved', decided_by: 'mason', decision_note: 'ok' })] }))
    show()
    const pending = await screen.findAllByTestId('pending-proposal')
    expect(pending).toHaveLength(1)
    expect(pending[0]).toHaveTextContent('proposed by agent:research')
    expect(screen.getAllByRole('button', { name: 'Approve' })).toHaveLength(1)
    expect(screen.getAllByRole('button', { name: 'Reject' })).toHaveLength(1)
    expect(screen.getByTestId('decided-proposal')).toHaveTextContent('approved')
  })

  it('shows no decide control at all when nothing is pending', async () => {
    studio.proposals.mockReturnValue(ok({ proposals: [proposal({ status: 'rejected', decided_by: 'mason', decision_note: 'no' })] }))
    show()
    await screen.findByTestId('decided-proposal')
    expect(screen.queryByRole('button', { name: 'Approve' })).toBeNull()
    expect(screen.queryByRole('button', { name: 'Reject' })).toBeNull()
  })

  it('needs a note to decide, and sends the decision', async () => {
    studio.proposals.mockReturnValue(ok({ proposals: [proposal()] }))
    show()
    const approve = await screen.findByRole('button', { name: 'Approve' })
    expect(approve).toBeDisabled()
    fireEvent.change(screen.getByLabelText('Decision note'), { target: { value: 'Reviewed the baselines' } })
    fireEvent.click(approve)
    await waitFor(() => expect(studio.decide).toHaveBeenCalledWith('p1', { decision: 'approved', note: 'Reviewed the baselines' }))
  })

  it('shows the refusal when an agent credential tries to decide (403)', async () => {
    studio.proposals.mockReturnValue(ok({ proposals: [proposal()] }))
    studio.decide.mockRejectedValue(Object.assign(new Error('forbidden'), { isAxiosError: true, response: { status: 403 } }))
    show()
    fireEvent.change(await screen.findByLabelText('Decision note'), { target: { value: 'x' } })
    fireEvent.click(screen.getByRole('button', { name: 'Reject' }))
    expect(await screen.findByText(/Only a human session can decide/)).toBeInTheDocument()
  })

  it('lists the version timeline with aliases and the deployments', async () => {
    show()
    const rows = await screen.findAllByTestId('timeline-row')
    expect(rows).toHaveLength(2)
    expect(rows[1]).toHaveTextContent('production')
    expect(await screen.findByRole('table', { name: 'Deployments' })).toHaveTextContent('100%')
  })
})
