/**
 * `POST .../promote` answers HTTP 200 with `promoted: false` when the gate
 * refuses. The old modal treated any resolved request as a success and closed,
 * so a version that did not move looked promoted. It also sent `reason`, which
 * the server ignores, so an override was never recorded. This is the modal that
 * replaces it: the verdict is read from the body, the server's own checks are
 * shown on a refusal, and an override travels as `override_reason`.
 */
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { PromotionGateModal } from '../PromotionGateModal'
import type { ModelVersion, PromoteResult } from '@/api/mlops'

const version = (over: Partial<ModelVersion> = {}): ModelVersion => ({
  version: 4,
  model_id: 'm1',
  status: 'candidate',
  artifact_hash: 'abc123',
  created_at: '2026-10-01T00:00:00Z',
  ...over,
})

function setup(onPromote: (a: { environment: 'paper' | 'live'; overrideReason?: string }) => Promise<PromoteResult>, v = version()) {
  const onClose = vi.fn()
  render(<PromotionGateModal open version={v} hasProduction onClose={onClose} onPromote={onPromote} />)
  return { onClose }
}

describe('a refused gate is not a promotion', () => {
  it('stays open and shows the server checks when promoted is false', async () => {
    const onPromote = vi.fn().mockResolvedValue({
      promoted: false,
      reason: 'gate_failed',
      gate_checks: [
        { check: 'passed_eval_suite', passed: true, required: true },
        { check: 'no_metric_regression', passed: false, required: true },
      ],
    })
    const { onClose } = setup(onPromote)
    fireEvent.click(screen.getByRole('button', { name: 'Promote' }))
    await screen.findByText(/The server refused this promotion/)
    expect(onClose).not.toHaveBeenCalled()
    expect(screen.getByText('No metric regression vs production')).toBeInTheDocument()
    // The button now says what it will do, and cannot do it without a reason.
    const override = screen.getByRole('button', { name: 'Promote with override' })
    expect(override).toBeDisabled()
  })

  it('sends the override reason as override_reason and closes on success', async () => {
    const onPromote = vi
      .fn()
      .mockResolvedValueOnce({ promoted: false, gate_checks: [{ check: 'no_metric_regression', passed: false, required: true }] })
      .mockResolvedValueOnce({ promoted: true })
    const { onClose } = setup(onPromote)
    fireEvent.click(screen.getByRole('button', { name: 'Promote' }))
    await screen.findByText(/The server refused/)
    fireEvent.change(screen.getByPlaceholderText(/Why is this version/), { target: { value: 'hotfix for the open' } })
    fireEvent.click(screen.getByRole('button', { name: 'Promote with override' }))
    await waitFor(() => expect(onClose).toHaveBeenCalled())
    expect(onPromote).toHaveBeenLastCalledWith({ environment: 'paper', overrideReason: 'hotfix for the open' })
    expect(onPromote.mock.calls[0][0].overrideReason).toBeUndefined()
  })

  it('closes only when the body says promoted', async () => {
    const onPromote = vi.fn().mockResolvedValue({ promoted: true, version: 4 })
    const { onClose } = setup(onPromote)
    fireEvent.click(screen.getByRole('button', { name: 'Promote' }))
    await waitFor(() => expect(onClose).toHaveBeenCalledTimes(1))
  })

  it('says nothing changed when the request itself fails', async () => {
    const onPromote = vi.fn().mockRejectedValue(new Error('500'))
    const { onClose } = setup(onPromote)
    fireEvent.click(screen.getByRole('button', { name: 'Promote' }))
    await screen.findByText(/Nothing was changed/)
    expect(onClose).not.toHaveBeenCalled()
  })

  it('requires a reason up front when the client already sees a failed check', () => {
    setup(vi.fn(), version({ status: 'draft', artifact_hash: undefined }))
    expect(screen.getByRole('button', { name: 'Promote with override' })).toBeDisabled()
    expect(screen.getByText('A reason is required to override a failed check.')).toBeInTheDocument()
  })
})
