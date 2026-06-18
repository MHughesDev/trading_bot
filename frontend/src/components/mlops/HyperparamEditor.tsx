import { useEffect, useMemo, useState } from 'react'
import { Code2, SlidersHorizontal, RotateCcw } from 'lucide-react'
import { cn } from '@/lib/utils'

// ---------------------------------------------------------------------------
// Per-framework parameter schemas.
//
// Each descriptor maps to a key the Python trainer adapter actually reads, so
// every knob here changes real training behavior (see apps/model-trainer).
// ---------------------------------------------------------------------------

type ParamType = 'int' | 'float' | 'select'

export interface ParamDesc {
  key: string
  label: string
  type: ParamType
  default: number | string
  min?: number
  max?: number
  step?: number
  options?: string[]
  help?: string
}

const XGBOOST_PARAMS: ParamDesc[] = [
  { key: 'n_estimators', label: 'Boosting rounds', type: 'int', default: 200, min: 10, max: 2000, step: 10, help: 'Number of trees / boosting iterations.' },
  { key: 'max_depth', label: 'Max depth', type: 'int', default: 6, min: 1, max: 20, step: 1, help: 'Maximum tree depth. Higher = more capacity, more overfit risk.' },
  { key: 'learning_rate', label: 'Learning rate (eta)', type: 'float', default: 0.05, min: 0.001, max: 0.5, step: 0.001 },
  { key: 'subsample', label: 'Row subsample', type: 'float', default: 1.0, min: 0.1, max: 1.0, step: 0.05, help: 'Fraction of rows sampled per tree.' },
  { key: 'colsample_bytree', label: 'Column subsample', type: 'float', default: 1.0, min: 0.1, max: 1.0, step: 0.05 },
  { key: 'min_child_weight', label: 'Min child weight', type: 'float', default: 1.0, min: 0, max: 20, step: 0.5 },
  { key: 'gamma', label: 'Min split loss (gamma)', type: 'float', default: 0.0, min: 0, max: 10, step: 0.1 },
  { key: 'reg_alpha', label: 'L1 reg (alpha)', type: 'float', default: 0.0, min: 0, max: 10, step: 0.1 },
  { key: 'reg_lambda', label: 'L2 reg (lambda)', type: 'float', default: 1.0, min: 0, max: 10, step: 0.1 },
  { key: 'early_stopping_rounds', label: 'Early stopping rounds', type: 'int', default: 30, min: 0, max: 200, step: 5, help: 'Stop if val metric stalls for N rounds. 0 disables.' },
]

const LIGHTGBM_PARAMS: ParamDesc[] = [
  { key: 'n_estimators', label: 'Boosting rounds', type: 'int', default: 200, min: 10, max: 2000, step: 10 },
  { key: 'max_depth', label: 'Max depth', type: 'int', default: -1, min: -1, max: 20, step: 1, help: '-1 means no limit.' },
  { key: 'learning_rate', label: 'Learning rate', type: 'float', default: 0.05, min: 0.001, max: 0.5, step: 0.001 },
  { key: 'num_leaves', label: 'Num leaves', type: 'int', default: 31, min: 2, max: 256, step: 1, help: 'Main complexity knob for LightGBM.' },
  { key: 'feature_fraction', label: 'Feature fraction', type: 'float', default: 1.0, min: 0.1, max: 1.0, step: 0.05 },
  { key: 'bagging_fraction', label: 'Bagging fraction', type: 'float', default: 1.0, min: 0.1, max: 1.0, step: 0.05 },
  { key: 'min_child_samples', label: 'Min child samples', type: 'int', default: 20, min: 1, max: 200, step: 1 },
  { key: 'reg_alpha', label: 'L1 reg', type: 'float', default: 0.0, min: 0, max: 10, step: 0.1 },
  { key: 'reg_lambda', label: 'L2 reg', type: 'float', default: 0.0, min: 0, max: 10, step: 0.1 },
  { key: 'early_stopping_rounds', label: 'Early stopping rounds', type: 'int', default: 30, min: 0, max: 200, step: 5, help: 'Stop if val metric stalls for N rounds. 0 disables.' },
]

const TORCH_PARAMS: ParamDesc[] = [
  { key: 'arch', label: 'Architecture', type: 'select', default: 'mlp', options: ['mlp', 'lstm'], help: 'MLP feed-forward or single-step LSTM.' },
  { key: 'epochs', label: 'Epochs', type: 'int', default: 50, min: 1, max: 500, step: 1 },
  { key: 'learning_rate', label: 'Learning rate', type: 'float', default: 0.001, min: 0.0001, max: 0.1, step: 0.0001 },
  { key: 'batch_size', label: 'Batch size', type: 'int', default: 64, min: 8, max: 1024, step: 8 },
  { key: 'hidden_dim', label: 'Hidden units', type: 'int', default: 32, min: 8, max: 256, step: 8 },
  { key: 'weight_decay', label: 'Weight decay', type: 'float', default: 0.0, min: 0, max: 0.1, step: 0.001, help: 'L2 regularization on weights.' },
]

// sklearn passes kwargs straight to the chosen estimator — a kwarg the
// estimator rejects makes the whole construction fall back to defaults, so
// only surface params valid for the selected estimator.
const SKLEARN_ESTIMATOR: ParamDesc = {
  key: 'estimator',
  label: 'Estimator',
  type: 'select',
  default: 'RandomForestClassifier',
  options: ['RandomForestClassifier', 'GradientBoostingClassifier', 'LogisticRegression'],
}

function sklearnParams(estimator: string): ParamDesc[] {
  if (estimator === 'LogisticRegression') {
    return [
      SKLEARN_ESTIMATOR,
      { key: 'C', label: 'Inverse reg (C)', type: 'float', default: 1.0, min: 0.01, max: 100, step: 0.01 },
      { key: 'max_iter', label: 'Max iterations', type: 'int', default: 100, min: 50, max: 2000, step: 50 },
    ]
  }
  // Random forest / gradient boosting both accept n_estimators + max_depth.
  return [
    SKLEARN_ESTIMATOR,
    { key: 'n_estimators', label: 'Estimators', type: 'int', default: 100, min: 10, max: 1000, step: 10 },
    { key: 'max_depth', label: 'Max depth', type: 'int', default: 6, min: 1, max: 30, step: 1 },
  ]
}

export function schemaFor(framework: string, values: Record<string, unknown>): ParamDesc[] {
  switch (framework) {
    case 'xgboost':
      return XGBOOST_PARAMS
    case 'lightgbm':
      return LIGHTGBM_PARAMS
    case 'torch':
      return TORCH_PARAMS
    case 'sklearn':
      return sklearnParams(String(values.estimator ?? SKLEARN_ESTIMATOR.default))
    default:
      return []
  }
}

function defaultsFor(framework: string): Record<string, number | string> {
  const out: Record<string, number | string> = {}
  for (const p of schemaFor(framework, {})) out[p.key] = p.default
  return out
}

interface Props {
  framework: string
  disabled?: boolean
  /** Emits the effective hyperparameters and whether they are valid. */
  onChange: (hyperparams: Record<string, unknown>, valid: boolean) => void
}

export function HyperparamEditor({ framework, disabled, onChange }: Props) {
  const [mode, setMode] = useState<'form' | 'raw'>('form')
  const [values, setValues] = useState<Record<string, number | string>>(() =>
    defaultsFor(framework),
  )
  const [rawText, setRawText] = useState('{}')
  const [rawError, setRawError] = useState<string | null>(null)

  // Reset to framework defaults whenever the framework changes.
  useEffect(() => {
    setValues(defaultsFor(framework))
  }, [framework])

  const schema = useMemo(() => schemaFor(framework, values), [framework, values])

  // Only emit keys that differ from the framework default, so the backend
  // shallow-merge stays minimal and definition defaults shine through.
  const formHyperparams = useMemo(() => {
    const defaults = defaultsFor(framework)
    const out: Record<string, unknown> = {}
    for (const p of schema) {
      const v = values[p.key]
      if (v !== undefined && v !== defaults[p.key]) out[p.key] = v
    }
    // `estimator` (and arch) are structural — always include when non-default
    // is set; the loop above already handles that.
    return out
  }, [framework, schema, values])

  // Push changes up.
  useEffect(() => {
    if (mode === 'raw') {
      try {
        const parsed = JSON.parse(rawText || '{}')
        setRawError(null)
        onChange(parsed, true)
      } catch {
        setRawError('Invalid JSON')
        onChange({}, false)
      }
    } else {
      onChange(formHyperparams, true)
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [mode, rawText, formHyperparams])

  function setParam(p: ParamDesc, raw: string) {
    setValues((prev) => {
      const next = { ...prev }
      if (p.type === 'select') {
        next[p.key] = raw
        // Switching sklearn estimator changes the field set — drop stale keys.
        if (p.key === 'estimator') {
          const keep = new Set(sklearnParams(raw).map((d) => d.key))
          for (const k of Object.keys(next)) if (!keep.has(k)) delete next[k]
          for (const d of sklearnParams(raw)) if (next[d.key] === undefined) next[d.key] = d.default
        }
      } else {
        const num = p.type === 'int' ? parseInt(raw, 10) : parseFloat(raw)
        next[p.key] = Number.isNaN(num) ? (p.default as number) : num
      }
      return next
    })
  }

  function syncRawFromForm() {
    setRawText(JSON.stringify(formHyperparams, null, 2))
  }

  return (
    <div className="space-y-3">
      <div className="flex items-center justify-between">
        <label className="text-xs font-medium text-text-muted">Hyperparameters</label>
        <div className="flex items-center gap-1 rounded-lg border border-border bg-surface-2 p-0.5">
          <button
            type="button"
            onClick={() => setMode('form')}
            className={cn(
              'flex items-center gap-1 rounded-md px-2 py-1 text-[11px] transition-colors',
              mode === 'form' ? 'bg-surface text-text shadow-sm' : 'text-text-muted',
            )}
          >
            <SlidersHorizontal className="h-3 w-3" />
            Knobs
          </button>
          <button
            type="button"
            onClick={() => {
              syncRawFromForm()
              setMode('raw')
            }}
            className={cn(
              'flex items-center gap-1 rounded-md px-2 py-1 text-[11px] transition-colors',
              mode === 'raw' ? 'bg-surface text-text shadow-sm' : 'text-text-muted',
            )}
          >
            <Code2 className="h-3 w-3" />
            JSON
          </button>
        </div>
      </div>

      {mode === 'form' ? (
        <div className="space-y-3">
          {schema.length === 0 ? (
            <p className="text-[11px] text-text-dim">
              No tunable hyperparameters for this framework.
            </p>
          ) : (
            schema.map((p) => (
              <div key={p.key}>
                <div className="flex items-center justify-between mb-1">
                  <label className="text-[11px] font-medium text-text-muted" title={p.help}>
                    {p.label}
                  </label>
                  <span className="font-mono text-[11px] text-text">
                    {String(values[p.key] ?? p.default)}
                  </span>
                </div>
                {p.type === 'select' ? (
                  <select
                    value={String(values[p.key] ?? p.default)}
                    onChange={(e) => setParam(p, e.target.value)}
                    disabled={disabled}
                    className="w-full h-8 rounded-lg border border-border bg-surface px-2 text-xs text-text focus:outline-none focus:border-accent disabled:opacity-50"
                  >
                    {p.options?.map((o) => (
                      <option key={o} value={o}>
                        {o}
                      </option>
                    ))}
                  </select>
                ) : (
                  <div className="flex items-center gap-2">
                    <input
                      type="range"
                      min={p.min}
                      max={p.max}
                      step={p.step}
                      value={Number(values[p.key] ?? p.default)}
                      onChange={(e) => setParam(p, e.target.value)}
                      disabled={disabled}
                      className="flex-1 accent-accent disabled:opacity-50"
                    />
                    <input
                      type="number"
                      min={p.min}
                      max={p.max}
                      step={p.step}
                      value={Number(values[p.key] ?? p.default)}
                      onChange={(e) => setParam(p, e.target.value)}
                      disabled={disabled}
                      className="w-16 h-7 rounded-md border border-border bg-surface px-1.5 text-[11px] font-mono text-text focus:outline-none focus:border-accent disabled:opacity-50"
                    />
                  </div>
                )}
                {p.help && (
                  <p className="mt-0.5 text-[10px] text-text-dim leading-snug">{p.help}</p>
                )}
              </div>
            ))
          )}
          <button
            type="button"
            onClick={() => setValues(defaultsFor(framework))}
            disabled={disabled}
            className="flex items-center gap-1 text-[11px] text-text-muted hover:text-text disabled:opacity-50"
          >
            <RotateCcw className="h-3 w-3" />
            Reset to defaults
          </button>
        </div>
      ) : (
        <div>
          <textarea
            value={rawText}
            onChange={(e) => setRawText(e.target.value)}
            rows={10}
            disabled={disabled}
            spellCheck={false}
            className={cn(
              'w-full rounded-lg border bg-surface px-2.5 py-2 text-xs font-mono text-text focus:outline-none resize-none disabled:opacity-50',
              rawError ? 'border-pnl-down' : 'border-border focus:border-accent',
            )}
          />
          {rawError ? (
            <p className="text-[11px] text-pnl-down mt-1">{rawError}</p>
          ) : (
            <p className="text-[10px] text-text-dim mt-1">
              Raw object merged over the model definition. Any key the framework
              adapter reads is honored.
            </p>
          )}
        </div>
      )}
    </div>
  )
}
