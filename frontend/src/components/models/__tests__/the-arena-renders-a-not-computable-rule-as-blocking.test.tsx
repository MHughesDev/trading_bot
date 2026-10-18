/**
 * N-5.7: candidate against baselines on identical folds, the usage-rule
 * verdicts never hidden, and a not_computable rule shown as BLOCKING, not as a
 * pass (ADR-N-05).
 */
import { fireEvent, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { never, ok, renderRoute, verdict, version } from './testkit'

const studio = vi.hoisted(() => ({ verdict: vi.fn() }))
const mlops = vi.hoisted(() => ({ listVersions: vi.fn(), getAliases: vi.fn(), listEvals: vi.fn(), evaluate: vi.fn() }))
vi.mock('@/api/modelsStudio', () => ({ studioApi: studio }))
vi.mock('@/api/mlops', () => ({ modelsApi: mlops }))

import { EvaluationsArena } from '../EvaluationsArena'

const show = () => renderRoute(<EvaluationsArena modelId="m1" />)

beforeEach(() => {
  vi.clearAllMocks()
  mlops.listVersions.mockReturnValue(ok({ versions: [version(2, { metrics: { qlike: 0.9 } })] }))
  mlops.getAliases.mockReturnValue(ok({}))
  mlops.listEvals.mockReturnValue(ok({ evaluations: [{ run_id: 'e1abcdef', status: 'succeeded', created_at: '2026-10-03T00:00:00Z' }] }))
  mlops.evaluate.mockReturnValue(ok({}))
  studio.verdict.mockReturnValue(ok(verdict()))
})

describe('the evaluations arena', () => {
  it('shows skeletons while the verdict loads', () => {
    studio.verdict.mockReturnValue(never())
    show()
    expect(document.querySelector('.skel')).not.toBeNull()
  })

  it('says there is nothing to evaluate when there is no version', async () => {
    mlops.listVersions.mockReturnValue(ok({ versions: [] }))
    show()
    expect(await screen.findByText('Nothing to evaluate')).toBeInTheDocument()
  })

  it('shows an error that does not clear the version when the verdict cannot be read', async () => {
    studio.verdict.mockRejectedValue(new Error('x'))
    show()
    expect((await screen.findAllByRole('alert'))[0]).toHaveTextContent('verdict could not be read')
    expect(screen.queryByTestId('verdict-headline')).toBeNull()
  })

  it('renders a not_computable rule as blocking, in the same weight as a fail, never as a pass', async () => {
    show()
    const rules = await screen.findAllByTestId('usage-rule')
    const tail = rules.find((r) => r.dataset.rule === 'tail_risk')!
    expect(tail).toHaveAttribute('data-status', 'not_computable')
    expect(tail).toHaveAttribute('data-blocking', 'true')
    expect(tail).toHaveTextContent('Not computable: blocks')
    expect(tail).toHaveTextContent('no ES backtest')
    expect(tail).not.toHaveTextContent(/^.*Pass/)
    const vol = rules.find((r) => r.dataset.rule === 'vol_forecast')!
    expect(vol).toHaveTextContent('Fail: blocks')
    expect(vol).toHaveTextContent('HARQ is better')
    expect(screen.getByTestId('verdict-headline')).toHaveTextContent('Blocked by 2 rules')
  })

  it('does not call a verdict allowed while a rule blocks, even if the server said allowed', async () => {
    studio.verdict.mockReturnValue(ok(verdict({ allowed: true })))
    show()
    expect(await screen.findByTestId('verdict-headline')).toHaveTextContent('Blocked by 2 rules')
  })

  it('shows all-pass as allowed', async () => {
    studio.verdict.mockReturnValue(
      ok(
        verdict({
          allowed: true,
          rules: [{ rule: 'return_forecast', applies: true, status: 'pass', reason: 'R2 positive', measured: {} }],
        }),
      ),
    )
    show()
    expect(await screen.findByTestId('verdict-headline')).toHaveTextContent('Allowed to size a position')
  })

  it('does not treat a rule that does not apply as blocking', async () => {
    studio.verdict.mockReturnValue(
      ok(verdict({ allowed: true, rules: [{ rule: 'tail_risk', applies: false, status: 'not_applicable', reason: 'no VaR consumer', measured: {} }] })),
    )
    show()
    const rule = await screen.findByTestId('usage-rule')
    expect(rule).toHaveAttribute('data-blocking', 'false')
    expect(rule).toHaveTextContent('Not applicable')
  })

  it('lays the candidate beside the baselines and marks a score it did not report', async () => {
    show()
    const cand = await screen.findByTestId('candidate-row')
    expect(cand).toHaveTextContent('Candidate v2')
    expect(cand).toHaveTextContent('0.9000') // qlike is reported
    expect(cand).toHaveTextContent('not reported') // crps is not
    expect(screen.getByText(/har/, { selector: 'td' })).toBeInTheDocument()
  })

  it('warns when the baselines were not scored on identical folds', async () => {
    studio.verdict.mockReturnValue(
      ok(
        verdict({
          baselines: [
            { name: 'har', family: 'har', variant: 'rv', folds: [0, 1], scores: { qlike: 1 }, contamination: null },
            { name: 'naive', family: 'naive', variant: 'zero', folds: [0, 1, 2], scores: { qlike: 2 }, contamination: null },
          ],
        }),
      ),
    )
    show()
    expect(await screen.findByTestId('folds-differ')).toBeInTheDocument()
  })

  it('says a version with no baselines is not a comparison', async () => {
    studio.verdict.mockReturnValue(ok(verdict({ baselines: [] })))
    show()
    expect(await screen.findByTestId('no-baselines')).toBeInTheDocument()
    expect(screen.queryByTestId('candidate-row')).toBeNull()
  })

  it('runs an evaluation for the selected version', async () => {
    show()
    await screen.findByTestId('candidate-row')
    fireEvent.click(screen.getByRole('button', { name: 'Run evaluation' }))
    await waitFor(() => expect(mlops.evaluate).toHaveBeenCalledWith('m1', 2))
  })
})
