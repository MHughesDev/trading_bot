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
