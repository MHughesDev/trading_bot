import { useEffect, useMemo, useState } from 'react'
import { Code2, SlidersHorizontal, RotateCcw } from 'lucide-react'
import { cn } from '@/lib/utils'
import { schemaFor } from './HyperparamEditor.schema'
import { sklearnParams } from './HyperparamEditor.schema'

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

  // Reset to framework defaults during the render that changes the framework. As
  // an effect this rendered the previous framework's knob values against the new
  // framework's schema once -- a learning rate from one adapter shown as if it
  // belonged to another.
  const [prevFramework, setPrevFramework] = useState(framework)
  if (framework !== prevFramework) {
    setPrevFramework(framework)
    setValues(defaultsFor(framework))
  }

  // Parsed once, here, instead of inside the push-up effect. `rawError` was state
  // the effect set but never cleared -- switching back to the knobs did not reset
  // it -- which only went unnoticed because it is rendered in the JSON pane alone.
  // As a derived value it cannot be stale.
  const parsedRaw = useMemo<{ ok: true; value: unknown } | { ok: false }>(() => {
    try {
      return { ok: true, value: JSON.parse(rawText || '{}') }
    } catch {
      return { ok: false }
    }
  }, [rawText])
  const rawError = mode === 'raw' && !parsedRaw.ok ? 'Invalid JSON' : null

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

  // Push changes up. This stays an effect: it is a message to the parent, which is
  // an external system as far as this component is concerned, and a render is no
  // place to send one.
  useEffect(() => {
    if (mode === 'raw') {
      if (parsedRaw.ok) onChange(parsedRaw.value as Record<string, unknown>, true)
      else onChange({}, false)
    } else {
      onChange(formHyperparams, true)
    }
    // `onChange` is deliberately absent: callers pass a fresh closure every render,
    // and including it would make this effect a render loop.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [mode, parsedRaw, formHyperparams])

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
        <label className="text-xs font-medium text-fg-secondary">Hyperparameters</label>
        <div className="flex items-center gap-1 rounded-lg border border-border bg-sunken p-0.5">
          <button
            type="button"
            onClick={() => setMode('form')}
            className={cn(
              'flex items-center gap-1 rounded-md px-2 py-1 text-[11px] transition-colors',
              mode === 'form' ? 'bg-surface text-text shadow-sm' : 'text-fg-secondary',
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
              mode === 'raw' ? 'bg-surface text-text shadow-sm' : 'text-fg-secondary',
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
            <p className="text-[11px] text-fg-tertiary">
              No tunable hyperparameters for this framework.
            </p>
          ) : (
            schema.map((p) => (
              <div key={p.key}>
                <div className="flex items-center justify-between mb-1">
                  <label className="text-[11px] font-medium text-fg-secondary" title={p.help}>
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
                  <p className="mt-0.5 text-[10px] text-fg-tertiary leading-snug">{p.help}</p>
                )}
              </div>
            ))
          )}
          <button
            type="button"
            onClick={() => setValues(defaultsFor(framework))}
            disabled={disabled}
            className="flex items-center gap-1 text-[11px] text-fg-secondary hover:text-text disabled:opacity-50"
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
              rawError ? 'border-neg' : 'border-border focus:border-accent',
            )}
          />
          {rawError ? (
            <p className="text-[11px] text-neg mt-1">{rawError}</p>
          ) : (
            <p className="text-[10px] text-fg-tertiary mt-1">
              Raw object merged over the model definition. Any key the framework
              adapter reads is honored.
            </p>
          )}
        </div>
      )}
    </div>
  )
}
