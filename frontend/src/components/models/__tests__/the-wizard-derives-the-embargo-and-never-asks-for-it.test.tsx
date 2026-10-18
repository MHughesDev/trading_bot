/**
 * N-5.3: the wizard declares a DatasetDeclaration, SHOWS the derived purge and
 * embargo (INV-12/INV-15) and has no input for either; it refuses to submit a
 * declaration the server did not accept or cannot lower to the trainer.
 */
import { fireEvent, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { never, ok, renderRoute } from './testkit'

const studio = vi.hoisted(() => ({ families: vi.fn(), validateDeclaration: vi.fn(), train: vi.fn() }))
const mlops = vi.hoisted(() => ({ create: vi.fn() }))
vi.mock('@/api/modelsStudio', () => ({ studioApi: studio }))
vi.mock('@/api/mlops', () => ({ modelsApi: mlops }))

import { ModelCreateWizardPage } from '../../../pages/ModelCreateWizardPage'

const families = [
  { family: 'gbm', variant: 'lgbm', framework: 'lightgbm', role: 'candidate', targets: [], outputs: [], needs_sequence: false, uses_gpu: false, requires: [], selectable_as_candidate: true, entrypoint: 'x', description: 'Gradient boosting', contamination: null },
  { family: 'har', variant: 'rv', framework: null, role: 'baseline', targets: [], outputs: [], needs_sequence: false, uses_gpu: false, requires: [], selectable_as_candidate: false, entrypoint: 'y', description: 'Heterogeneous AR baseline', contamination: null },
]

const accepted = (over: Record<string, unknown> = {}) => ({
  ok: true,
  declaration_hash: 'abcdef0123456789abcdef',
  derived: { purge_bars: 24, embargo_bars: 288, horizon_bars: 24, lookback_bars: 50, sample_estimate: 9000 },
  clip: null,
  lowering: { ok: true, unsupported: [], trainer_fields: ['data'] },
  ...over,
})

function type(label: string | RegExp, value: string) {
  fireEvent.change(screen.getByLabelText(label), { target: { value } })
}
const next = () => fireEvent.click(screen.getByRole('button', { name: 'Next' }))

/** Every control's accessible name on the current step. */
function controlNames(): string[] {
  const out: string[] = []
  for (const el of document.querySelectorAll('input, select, textarea')) {
    const id = el.getAttribute('id')
    const lbl = id ? document.querySelector(`label[for="${id}"]`)?.textContent : null
    out.push(`${el.getAttribute('aria-label') ?? ''} ${lbl ?? ''} ${el.getAttribute('name') ?? ''} ${el.getAttribute('placeholder') ?? ''}`)
  }
  return out
}

/** Walks every step, returning the control names seen on each. */
async function walkToReview(opts: { weights?: string } = {}) {
  const seen: string[][] = []
  fireEvent.click(await screen.findByRole('radio', { name: 'gbm/lgbm' }))
  seen.push(controlNames())
  next()
  type('Instruments', 'BTC-USD ETH-USD')
  type('Window start', '2026-01-01')
  type('Window end', '2026-06-01')
  seen.push(controlNames())
  next()
  type('Features', 'ema(close,21)\nrsi(close,14)')
  seen.push(controlNames())
  next()
  type('Horizon (bars)', '24')
  const w = opts.weights ?? 'uniqueness'
  if (w !== 'unset') fireEvent.change(screen.getByLabelText('Sample weights'), { target: { value: w } })
  seen.push(controlNames())
  next()
  type('Train days', '90')
  type('Test days', '14')
  type('Refit days', '14')
  seen.push(controlNames())
  next()
  return seen
}

beforeEach(() => {
  vi.clearAllMocks()
  studio.families.mockReturnValue(ok({ families }))
  studio.validateDeclaration.mockReturnValue(ok(accepted()))
})

describe('the create wizard: families', () => {
  it('shows a skeleton while families load', () => {
    studio.families.mockReturnValue(never())
    renderRoute(<ModelCreateWizardPage />)
    expect(document.querySelector('.skel')).not.toBeNull()
  })

  it('says so when the registry has no families', async () => {
    studio.families.mockReturnValue(ok({ families: [] }))
    renderRoute(<ModelCreateWizardPage />)
    expect(await screen.findByText('No model families registered')).toBeInTheDocument()
  })

  it('shows an error when the families cannot be read', async () => {
    studio.families.mockRejectedValue(new Error('down'))
    renderRoute(<ModelCreateWizardPage />)
    expect(await screen.findByRole('alert')).toHaveTextContent('families could not be read')
  })

  it('offers only candidates, lists baselines as always trained beside, and blocks Next until one is chosen', async () => {
    renderRoute(<ModelCreateWizardPage />)
    await screen.findByRole('radio', { name: 'gbm/lgbm' })
    expect(screen.queryByRole('radio', { name: 'har/rv' })).toBeNull()
    expect(screen.getByTestId('baseline-families')).toHaveTextContent('har/rv')
    expect(screen.getByRole('button', { name: 'Next' })).toBeDisabled()
    fireEvent.click(screen.getByRole('radio', { name: 'gbm/lgbm' }))
    expect(screen.getByRole('button', { name: 'Next' })).toBeEnabled()
  })
})

describe('the create wizard: the derived embargo', () => {
  it('shows the server-derived purge and embargo and has no input for either on any step', async () => {
    renderRoute(<ModelCreateWizardPage />)
    const seen = await walkToReview()
    expect(await screen.findByTestId('derived-embargo')).toHaveTextContent('288 bars')
    expect(screen.getByTestId('derived-purge')).toHaveTextContent('24 bars')
    expect(screen.getByTestId('derived')).toHaveTextContent('Derived by the server, not editable')
    // No step, including the review, has a control named for purge or embargo.
    for (const names of [...seen, controlNames()]) {
      expect(names.join(' ')).not.toMatch(/purge|embargo/i)
    }
  })

  it('sends a declaration with no purge or embargo key, weights present, and the CV scheme', async () => {
    renderRoute(<ModelCreateWizardPage />)
    await walkToReview()
    await waitFor(() => expect(studio.validateDeclaration).toHaveBeenCalled())
    const body = studio.validateDeclaration.mock.calls[0][0]
    const json = JSON.stringify(body)
    expect(json).not.toMatch(/purge|embargo/i)
    expect(body.declaration.weights).toEqual({ kind: 'uniqueness' })
    expect(body.declaration.universe).toEqual(['BTC-USD', 'ETH-USD'])
    expect(body.declaration.features).toEqual(['ema(close,21)', 'rsi(close,14)'])
    expect(body.declaration.cv).toEqual({ kind: 'walk_forward', train_days: 90, test_days: 14, refit_days: 14, mode: 'expanding' })
  })

  it('refuses to validate, and names the step, when weights were not chosen', async () => {
    renderRoute(<ModelCreateWizardPage />)
    await walkToReview({ weights: 'unset' })
    expect(await screen.findByTestId('form-problems')).toHaveTextContent('there is no default')
    expect(studio.validateDeclaration).not.toHaveBeenCalled()
    expect(screen.getByRole('button', { name: /create and train/i })).toBeDisabled()
  })

  it('shows the server refusal against its field', async () => {
    studio.validateDeclaration.mockReturnValue(ok({ ok: false, errors: [{ field: 'window', message: 'start must be before end' }] }))
    renderRoute(<ModelCreateWizardPage />)
    await walkToReview()
    expect(await screen.findByTestId('server-refusal')).toHaveTextContent('start must be before end')
    expect(screen.getByRole('button', { name: /create and train/i })).toBeDisabled()
  })

  it('says when the window end was clipped to the cutoff', async () => {
    studio.validateDeclaration.mockReturnValue(ok(accepted({ clip: { requested_end: '2026-06-01T23:59:59Z', clipped_end: '2026-05-01T00:00:00Z' } })))
    renderRoute(<ModelCreateWizardPage />)
    await walkToReview()
    expect(await screen.findByTestId('clip-notice')).toHaveTextContent('clipped to 2026-05-01')
  })

  it('disables submission, and lists why, when the declaration cannot lower to the trainer', async () => {
    studio.validateDeclaration.mockReturnValue(
      ok(accepted({ lowering: { ok: false, unsupported: [{ field: 'sampling', why: 'cusum is not lowered yet' }], trainer_fields: [] } })),
    )
    renderRoute(<ModelCreateWizardPage />)
    await walkToReview()
    expect(await screen.findByTestId('lowering-refused')).toHaveTextContent('cusum is not lowered yet')
    type('Model name', 'BTC model')
    expect(screen.getByRole('button', { name: /create and train/i })).toBeDisabled()
  })
})

describe('the create wizard: submit', () => {
  it('creates the model, then trains it with the declaration and the family', async () => {
    mlops.create.mockReturnValue(ok({ model_id: 'new1', slug: 'new1' }))
    studio.train.mockReturnValue(ok({ run_id: 'r1' }))
    renderRoute(<ModelCreateWizardPage />)
    await walkToReview()
    await screen.findByTestId('derived-embargo')
    type('Model name', 'BTC model')
    fireEvent.click(screen.getByRole('button', { name: /create and train/i }))
    await waitFor(() => expect(studio.train).toHaveBeenCalled())
    expect(mlops.create.mock.calls[0][0].display_name).toBe('BTC model')
    const [id, body] = studio.train.mock.calls[0]
    expect(id).toBe('new1')
    expect(body.family).toEqual({ family: 'gbm', variant: 'lgbm' })
    expect(body.declaration.weights).toEqual({ kind: 'uniqueness' })
    expect(JSON.stringify(body)).not.toMatch(/purge|embargo/i)
  })

  it('reports a failed start and does not train', async () => {
    mlops.create.mockRejectedValue(new Error('no'))
    renderRoute(<ModelCreateWizardPage />)
    await walkToReview()
    await screen.findByTestId('derived-embargo')
    type('Model name', 'BTC model')
    fireEvent.click(screen.getByRole('button', { name: /create and train/i }))
    expect(await screen.findByText(/Nothing was started/)).toBeInTheDocument()
    expect(studio.train).not.toHaveBeenCalled()
  })
})
