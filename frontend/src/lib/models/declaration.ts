// Form state -> `DatasetDeclaration`, for the create wizard (N-5.3).
//
// Two rules this file holds:
//   * there is no purge and no embargo anywhere in it -- the form has no field
//     for either and `buildDeclaration` cannot emit one. The server derives both
//     (INV-12/INV-15) and the wizard shows what it derived;
//   * `weights` has no default. An empty choice is a problem, not `none`
//     (ADR-P2-22).
// Semantic validation (window order, lag monotonicity, cutoff...) is the
// server's: this file only turns text into the typed shape and reports what it
// could not parse.
import type {
  BarrierPath,
  Cv,
  DatasetDeclaration,
  Sampling,
  Scaler,
  Target,
  Timeframe,
  WalkMode,
  Weights,
} from '@/api/modelsStudio'

export const TIMEFRAMES: Timeframe[] = ['1m', '5m', '15m', '30m', '1h', '4h', '1d']

export type TargetKind = Target['kind']
export type SamplingKind = Sampling['kind']
export type CvKind = Cv['kind']
export type WeightsChoice = '' | Weights['kind']

export interface DeclarationForm {
  instruments: string
  timeframe: Timeframe
  start: string
  end: string
  features: string
  lags: string
  winsorize: string
  scaler: Scaler
  samplingKind: SamplingKind
  hSigma: string
  samplingStrategy: string
  targetKind: TargetKind
  horizon: string
  logReturn: boolean
  ptSigma: string
  slSigma: string
  maxBars: string
  barrierPath: BarrierPath
  metaStrategy: string
  weights: WeightsChoice
  halfLife: string
  seqWindow: string
  seqStride: string
  cvKind: CvKind
  trainDays: string
  testDays: string
  refitDays: string
  walkMode: WalkMode
  k: string
  n: string
}

export const EMPTY_FORM: DeclarationForm = {
  instruments: '',
  timeframe: '1h',
  start: '',
  end: '',
  features: '',
  lags: '',
  winsorize: '',
  scaler: 'none',
  samplingKind: 'every_bar',
  hSigma: '',
  samplingStrategy: '',
  targetKind: 'forward_return',
  horizon: '',
  logReturn: true,
  ptSigma: '',
  slSigma: '',
  maxBars: '',
  barrierPath: 'high_low',
  metaStrategy: '',
  weights: '',
  halfLife: '',
  seqWindow: '',
  seqStride: '1',
  cvKind: 'walk_forward',
  trainDays: '',
  testDays: '',
  refitDays: '',
  walkMode: 'expanding',
  k: '5',
  n: '6',
}

export interface BuildProblem {
  field: string
  message: string
}

export interface Built {
  declaration: DatasetDeclaration | null
  problems: BuildProblem[]
}

function words(s: string): string[] {
  return s
    .split(/[\s,;]+/)
    .map((x) => x.trim())
    .filter((x) => x.length > 0)
}

/** Feature expressions may contain commas (`ema(close,21)`), so only newlines and `;` split them. */
function featureList(s: string): string[] {
  return s
    .split(/[\n;]+/)
    .map((x) => x.trim())
    .filter((x) => x.length > 0)
}

function day(s: string, end: boolean): string | null {
  if (!/^\d{4}-\d{2}-\d{2}$/.test(s)) return null
  return end ? `${s}T23:59:59Z` : `${s}T00:00:00Z`
}

export function buildDeclaration(f: DeclarationForm, opts: { needsSequence?: boolean } = {}): Built {
  const problems: BuildProblem[] = []
  const bad = (field: string, message: string) => problems.push({ field, message })

  const num = (field: string, raw: string, what: string, integer = false): number => {
    const t = raw.trim()
    const n = t === '' ? Number.NaN : Number(t)
    if (!Number.isFinite(n) || (integer && !Number.isInteger(n))) {
      bad(field, `${what} must be ${integer ? 'a whole number' : 'a number'}`)
      return 0
    }
    return n
  }

  const instruments = words(f.instruments)
  if (instruments.length === 0) bad('universe', 'name at least one instrument')

  const start = day(f.start, false)
  const end = day(f.end, true)
  if (!start) bad('window.start', 'pick a start date')
  if (!end) bad('window.end', 'pick an end date')

  const features = featureList(f.features)
  if (features.length === 0) bad('features', 'declare at least one feature')

  const lags = words(f.lags).map((x) => Number(x))
  if (lags.some((l) => !Number.isInteger(l))) bad('transforms.lags', 'lags are whole numbers of bars')

  let winsorize: number | null = null
  if (f.winsorize.trim() !== '') winsorize = num('transforms.winsorize', f.winsorize, 'the winsorise tail')

  let sampling: Sampling
  switch (f.samplingKind) {
    case 'cusum':
      sampling = { kind: 'cusum', h_sigma: num('sampling.h_sigma', f.hSigma, 'the CUSUM threshold') }
      break
    case 'strategy_events':
      sampling = { kind: 'strategy_events', strategy_version: f.samplingStrategy.trim() }
      break
    default:
      sampling = { kind: 'every_bar' }
  }

  let target: Target
  switch (f.targetKind) {
    case 'forward_return':
      target = { kind: 'forward_return', horizon_bars: num('target', f.horizon, 'the horizon', true), log: f.logReturn }
      break
    case 'realized_vol':
      target = { kind: 'realized_vol', horizon_bars: num('target', f.horizon, 'the horizon', true) }
      break
    case 'rank':
      target = { kind: 'rank', horizon_bars: num('target', f.horizon, 'the horizon', true) }
      break
    case 'direction':
      target = { kind: 'direction', horizon_bars: num('target', f.horizon, 'the horizon', true) }
      break
    case 'triple_barrier':
      target = {
        kind: 'triple_barrier',
        pt_sigma: num('target', f.ptSigma, 'the profit-take multiple'),
        sl_sigma: num('target', f.slSigma, 'the stop multiple'),
        max_bars: num('target', f.maxBars, 'max bars', true),
        path: f.barrierPath,
      }
      break
    default:
      target = {
        kind: 'meta_label',
        strategy_version: f.metaStrategy.trim(),
        pt_sigma: num('target', f.ptSigma, 'the profit-take multiple'),
        sl_sigma: num('target', f.slSigma, 'the stop multiple'),
        max_bars: num('target', f.maxBars, 'max bars', true),
        path: f.barrierPath,
      }
  }

  let weights: Weights = { kind: 'none' }
  if (f.weights === '') {
    bad('weights', 'choose a weighting: there is no default')
  } else if (f.weights === 'time_decay') {
    weights = { kind: 'time_decay', half_life_days: num('weights', f.halfLife, 'the half-life') }
  } else {
    weights = { kind: f.weights }
  }

  let sequence: DatasetDeclaration['sequence'] = null
  if (opts.needsSequence) {
    sequence = {
      window: num('sequence.window', f.seqWindow, 'the sequence window', true),
      stride: num('sequence.stride', f.seqStride, 'the stride', true),
    }
  }

  let cv: Cv
  switch (f.cvKind) {
    case 'purged_kfold':
      cv = { kind: 'purged_kfold', k: num('cv.k', f.k, 'k', true) }
      break
    case 'cpcv':
      cv = { kind: 'cpcv', n: num('cv.n', f.n, 'n', true), k: num('cv.k', f.k, 'k', true) }
      break
    default:
      cv = {
        kind: 'walk_forward',
        train_days: num('cv.train_days', f.trainDays, 'train days', true),
        test_days: num('cv.test_days', f.testDays, 'test days', true),
        refit_days: num('cv.refit_days', f.refitDays, 'refit days', true),
        mode: f.walkMode,
      }
  }

  if (problems.length > 0 || !start || !end) return { declaration: null, problems }

  const declaration: DatasetDeclaration = {
    universe: instruments,
    timeframe: f.timeframe,
    window: { start, end },
    features,
    transforms: { winsorize, scaler: f.scaler, lags },
    sampling,
    target,
    weights,
    sequence,
    cv,
  }
  return { declaration, problems }
}

/** Wizard steps in order; `fields` are the server-error prefixes each step owns. */
export const STEPS = [
  { id: 'family', label: 'Family', fields: ['family'] },
  { id: 'universe', label: 'Universe and window', fields: ['universe', 'timeframe', 'window'] },
  { id: 'features', label: 'Features', fields: ['features', 'transforms', 'sequence'] },
  { id: 'target', label: 'Target and weights', fields: ['sampling', 'target', 'weights'] },
  { id: 'cv', label: 'Validation', fields: ['cv'] },
  { id: 'review', label: 'Review', fields: [] },
] as const

export type StepId = (typeof STEPS)[number]['id']

export function stepOfField(field: string): StepId {
  const head = field.split('.')[0]
  for (const s of STEPS) {
    if ((s.fields as readonly string[]).includes(head)) return s.id
  }
  return 'review'
}
