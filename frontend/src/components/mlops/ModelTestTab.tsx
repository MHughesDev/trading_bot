import { useEffect, useMemo, useState } from 'react'
import {
  Play,
  Loader2,
  Save,
  Wand2,
  Trash2,
  TrendingUp,
  TrendingDown,
  Minus,
  Code2,
  SlidersHorizontal,
  RotateCcw,
} from 'lucide-react'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import { modelsApi, type Forecast } from '@/api/mlops'
import {
  useModel,
  useModelVersions,
  useMarketInstruments,
  useTestCases,
  useAddTestCase,
  useDeleteTestCase,
} from '@/hooks/useMlOps'

interface Props {
  modelId: string
}

interface VersionResult {
  version: number
  forecast?: Forecast
  latency_ms?: number
  error?: string
}

function DirectionPill({ direction }: { direction: string }) {
  const map = {
    up: { icon: TrendingUp, cls: 'text-pos bg-pos/10 border-pos/20' },
    down: { icon: TrendingDown, cls: 'text-neg bg-neg/10 border-neg/20' },
    flat: { icon: Minus, cls: 'text-fg-secondary bg-sunken border-border' },
  } as const
  const m = map[direction as keyof typeof map] ?? map.flat
  const Icon = m.icon
  return (
    <span
      className={cn(
        'inline-flex items-center gap-1 rounded-full border px-2.5 py-0.5 text-xs font-medium capitalize',
        m.cls,
      )}
    >
      <Icon className="h-3.5 w-3.5" />
      {direction}
    </span>
  )
}

function ConfidenceBar({ value }: { value: number }) {
  const pct = Math.round(Math.max(0, Math.min(1, value)) * 100)
  return (
    <div className="flex items-center gap-2">
      <div className="h-1.5 flex-1 overflow-hidden rounded-full bg-sunken">
        <div className="h-full rounded-full bg-accent" style={{ width: `${pct}%` }} />
      </div>
      <span className="font-mono text-xs text-text w-9 text-right">{pct}%</span>
    </div>
  )
}

export function ModelTestTab({ modelId }: Props) {
  const { data: model } = useModel(modelId)
  const { data: versions } = useModelVersions(modelId)
  const { data: marketInstruments } = useMarketInstruments()
  const { data: testCases } = useTestCases(modelId)
  const addCase = useAddTestCase(modelId)
  const deleteCase = useDeleteTestCase(modelId)

  const isLlm = model?.model_kind === 'external_llm_adapter'

  // Any trained version can be tested — evaluating (just trained), candidate
  // (passed eval), or active (promoted). Only drafts/failed are excluded.
  const testableVersions = useMemo(
    () =>
      (versions ?? [])
        .filter(
          (v) =>
            v.status === 'active' ||
            v.status === 'candidate' ||
            v.status === 'evaluating',
        )
        .sort((a, b) => b.version - a.version),
    [versions],
  )

  // Three "default this once the data arrives" effects used to live below. Each
  // one rendered the tab with an empty selection first and corrected it on the
  // next render, which on the instrument picker meant a visible flash of no
  // instrument chosen. They are the same rule stated as a fallback instead: what
  // the person picked, or the first thing available.
  const [versionChoice, setVersionChoice] = useState<number[]>([])
  const [instrumentChoice, setInstrumentChoice] = useState('')
  const [timeframeChoice, setTimeframeChoice] = useState('')
  const [features, setFeatures] = useState<Record<string, number>>({})
  const [featureOrder, setFeatureOrder] = useState<string[]>([])
  const [mode, setMode] = useState<'form' | 'json'>('form')
  const [jsonText, setJsonText] = useState('{}')
  const [jsonError, setJsonError] = useState<string | null>(null)
  const [autofillInfo, setAutofillInfo] = useState<string | null>(null)
  const [autofilling, setAutofilling] = useState(false)
  const [prompt, setPrompt] = useState('Summarize current BTC market sentiment in one line.')

  const [running, setRunning] = useState(false)
  const [results, setResults] = useState<VersionResult[]>([])
  const [llmResult, setLlmResult] = useState<{
    text?: string
    latency_ms?: number
    tokens?: number
    cost_usd?: string
    trace_id?: string
    error?: string
  } | null>(null)
  const [saveName, setSaveName] = useState('')

  // Instrument -> available timeframes.
  const instrumentMap = useMemo(() => {
    const m = new Map<string, string[]>()
    for (const mi of marketInstruments ?? []) {
      const list = m.get(mi.instrument_id) ?? []
      list.push(mi.timeframe)
      m.set(mi.instrument_id, list)
    }
    return m
  }, [marketInstruments])

  const selectedVersions =
    versionChoice.length > 0
      ? versionChoice
      : testableVersions.length > 0
        ? [testableVersions[0].version]
        : []

  // The instrument falls back to the first one with stored bars; the timeframe
  // falls back to the first one that instrument actually has. Stating the second
  // as a fallback also subsumes the correction the instrument picker used to do by
  // hand -- a timeframe the new instrument does not offer can no longer be the
  // selected value even for a render.
  const instrument = instrumentChoice || marketInstruments?.[0]?.instrument_id || ''
  const timeframesHere = instrumentMap.get(instrument) ?? []
  const timeframe =
    timeframeChoice && timeframesHere.includes(timeframeChoice) ? timeframeChoice : (timeframesHere[0] ?? '')

  // Prefill the expected feature schema (names + zeros) on load.
  useEffect(() => {
    if (isLlm || featureOrder.length > 0) return
    let cancelled = false
    modelsApi
      .featureVector(modelId)
      .then((r) => {
        if (cancelled) return
        setFeatureOrder(r.data.feature_order)
        setFeatures(r.data.features)
      })
      .catch(() => {})
    return () => {
      cancelled = true
    }
  }, [modelId, isLlm, featureOrder.length])

  function syncJsonFromForm() {
    setJsonText(JSON.stringify(features, null, 2))
  }

  function effectiveFeatures(): Record<string, number> | null {
    if (mode === 'json') {
      try {
        return JSON.parse(jsonText)
      } catch {
        setJsonError('Invalid JSON')
        return null
      }
    }
    return features
  }

  async function handleAutofill() {
    if (!instrument || !timeframe) return
    setAutofilling(true)
    setAutofillInfo(null)
    try {
      const r = await modelsApi.featureVector(modelId, {
        instrument_id: instrument,
        timeframe,
      })
      setFeatureOrder(r.data.feature_order)
      setFeatures(r.data.features)
      setJsonText(JSON.stringify(r.data.features, null, 2))
      if (r.data.source === 'computed') {
        const when = r.data.as_of_ms
          ? new Date(r.data.as_of_ms).toLocaleString()
          : 'latest bar'
        setAutofillInfo(`Computed from ${r.data.bars_used} bars · as of ${when}`)
      } else {
        setAutofillInfo('No stored bars for this instrument — filled schema with zeros.')
      }
    } catch (e) {
      setAutofillInfo(`Autofill failed: ${String(e)}`)
    } finally {
      setAutofilling(false)
    }
  }

  async function handleRun() {
    if (isLlm) return handleRunLlm()
    const feats = effectiveFeatures()
    if (!feats || selectedVersions.length === 0) return
    setRunning(true)
    setResults([])
    try {
      const settled = await Promise.all(
        selectedVersions.map(async (version): Promise<VersionResult> => {
          try {
            const res = await modelsApi.testVersion(modelId, version, [
              { instrument_id: instrument || 'unknown', features: feats },
            ])
            return {
              version,
              forecast: res.data.predictions?.[0],
              latency_ms: res.data.latency_ms,
            }
          } catch (e) {
            return { version, error: String(e) }
          }
        }),
      )
      setResults(settled)
    } finally {
      setRunning(false)
    }
  }

  async function handleRunLlm() {
    const version = selectedVersions[0] ?? testableVersions[0]?.version
    if (!version) return
    setRunning(true)
    setLlmResult(null)
    try {
      // The server reads `prompt` from the instance. This used to send only an
      // empty feature map, so the server answered its default "Hello" and the
      // prompt typed here never reached the model.
      const res = await modelsApi.testVersion(modelId, version, [{ prompt }])
      setLlmResult({
        text: res.data.text,
        latency_ms: res.data.latency_ms,
        tokens: res.data.tokens,
        cost_usd: res.data.cost_usd,
        trace_id: res.data.trace_id,
      })
    } catch (e) {
      setLlmResult({ error: String(e) })
    } finally {
      setRunning(false)
    }
  }

  async function handleSaveCase() {
    if (!saveName.trim()) return
    const feats = effectiveFeatures()
    if (!feats) return
    await addCase.mutateAsync({
      name: saveName.trim(),
      input: { instrument_id: instrument, features: feats },
    })
    setSaveName('')
  }

  function replayCase(input: { instrument_id?: string; features?: Record<string, number> }) {
    if (input.instrument_id) setInstrumentChoice(input.instrument_id)
    if (input.features) {
      setFeatures(input.features)
      setJsonText(JSON.stringify(input.features, null, 2))
      setFeatureOrder(Object.keys(input.features))
    }
  }

  function toggleVersion(version: number) {
    // Toggles against the *effective* selection, so the version shown as chosen by
    // the fallback is the one a click removes. Clearing the last one falls back to
    // the first testable version again, exactly as the old effect did.
    setVersionChoice(
      selectedVersions.includes(version)
        ? selectedVersions.filter((v) => v !== version)
        : [...selectedVersions, version],
    )
  }

  const canRun =
    selectedVersions.length > 0 && (isLlm || (mode === 'json' ? !jsonError : true))

  // ---- LLM adapter test surface -------------------------------------------
  if (isLlm) {
    return (
      <div className="grid grid-cols-1 lg:grid-cols-2 gap-5">
        <div className="space-y-4">
          <h3 className="text-sm font-medium text-text">Prompt</h3>
          <textarea
            value={prompt}
            onChange={(e) => setPrompt(e.target.value)}
            rows={6}
            className="w-full rounded-lg border border-border bg-surface px-2.5 py-2 text-xs text-text focus:outline-none focus:border-accent resize-none"
          />
          <Button className="w-full text-sm" onClick={handleRunLlm} disabled={running}>
            {running ? <Loader2 className="h-4 w-4 animate-spin" /> : <Play className="h-4 w-4" />}
            Run
          </Button>
        </div>
        <div className="space-y-4">
          <h3 className="text-sm font-medium text-text">Response</h3>
          {llmResult?.error && (
            <div className="rounded-xl border border-neg/20 bg-neg/10 p-4 text-sm text-neg">
              {llmResult.error}
            </div>
          )}
          {llmResult?.text && (
            <div className="rounded-xl border border-border bg-surface p-4">
              <p className="text-sm text-text whitespace-pre-wrap">{llmResult.text}</p>
              <p className="mt-3 text-xs text-fg-secondary font-mono" data-testid="llm-metrics">
                {llmResult.latency_ms !== undefined ? `${llmResult.latency_ms} ms` : 'latency —'}
                {' · '}
                {llmResult.tokens !== undefined ? `${llmResult.tokens} tokens` : 'tokens —'}
                {' · '}
                {llmResult.cost_usd !== undefined ? `$${llmResult.cost_usd}` : 'cost —'}
                {llmResult.trace_id ? ` · trace ${llmResult.trace_id}` : ''}
              </p>
            </div>
          )}
          {!llmResult && (
            <div className="rounded-xl border border-dashed border-border p-12 text-center text-sm text-fg-tertiary">
              Run a prompt to see the response.
            </div>
          )}
        </div>
      </div>
    )
  }

  // ---- Feature-based model test surface -----------------------------------
  return (
    <div className="grid grid-cols-1 lg:grid-cols-2 gap-5">
      {/* Input panel */}
      <div className="space-y-4">
        <h3 className="text-sm font-medium text-text">Test inputs</h3>

        {/* Versions (multi-select for compare) */}
        <div>
          <label className="block text-xs font-medium text-fg-secondary mb-1.5">
            Versions {selectedVersions.length > 1 && '(compare)'}
          </label>
          {testableVersions.length === 0 ? (
            <p className="text-xs text-fg-tertiary">
              No active or candidate versions yet. Train and promote a version first.
            </p>
          ) : (
            <div className="flex flex-wrap gap-1.5">
              {testableVersions.map((v) => (
                <button
                  key={v.version}
                  onClick={() => toggleVersion(v.version)}
                  className={cn(
                    'rounded-lg border px-2.5 py-1 text-xs transition-colors',
                    selectedVersions.includes(v.version)
                      ? 'border-accent bg-accent/10 text-text'
                      : 'border-border bg-surface text-fg-secondary hover:border-border-2',
                  )}
                >
                  v{v.version}
                  <span className="ml-1 text-fg-tertiary">{v.status}</span>
                </button>
              ))}
            </div>
          )}
        </div>

        {/* Instrument + timeframe + autofill */}
        <div className="rounded-lg border border-border bg-sunken p-3 space-y-3">
          <div className="grid grid-cols-2 gap-2">
            <div>
              <label className="block text-[11px] font-medium text-fg-secondary mb-1">
                Instrument
              </label>
              <select
                value={instrument}
                onChange={(e) => setInstrumentChoice(e.target.value)}
                className="w-full h-8 rounded-lg border border-border bg-surface px-2 text-xs text-text focus:outline-none focus:border-accent"
              >
                {instrumentMap.size === 0 && <option value="">— none stored —</option>}
                {[...instrumentMap.keys()].map((id) => (
                  <option key={id} value={id}>
                    {id}
                  </option>
                ))}
              </select>
            </div>
            <div>
              <label className="block text-[11px] font-medium text-fg-secondary mb-1">
                Timeframe
              </label>
              <select
                value={timeframe}
                onChange={(e) => setTimeframeChoice(e.target.value)}
                className="w-full h-8 rounded-lg border border-border bg-surface px-2 text-xs text-text focus:outline-none focus:border-accent"
              >
                {(instrumentMap.get(instrument) ?? []).map((tf) => (
                  <option key={tf} value={tf}>
                    {tf}
                  </option>
                ))}
              </select>
            </div>
          </div>
          <Button
            variant="outline"
            className="w-full text-xs h-8"
            onClick={handleAutofill}
            disabled={autofilling || !instrument || !timeframe}
          >
            {autofilling ? (
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
            ) : (
              <Wand2 className="h-3.5 w-3.5" />
            )}
            Autofill from latest bar
          </Button>
          {autofillInfo && <p className="text-[11px] text-fg-tertiary">{autofillInfo}</p>}
        </div>

        {/* Feature editor */}
        <div>
          <div className="flex items-center justify-between mb-1.5">
            <label className="text-xs font-medium text-fg-secondary">
              Features{' '}
              {model?.definition.feature_set_ref ? (
                <span className="text-fg-tertiary font-mono">
                  ({String(model.definition.feature_set_ref)})
                </span>
              ) : null}
            </label>
            <div className="flex items-center gap-1 rounded-lg border border-border bg-sunken p-0.5">
              <button
                onClick={() => setMode('form')}
                className={cn(
                  'flex items-center gap-1 rounded-md px-2 py-1 text-[11px]',
                  mode === 'form' ? 'bg-surface text-text shadow-sm' : 'text-fg-secondary',
                )}
              >
                <SlidersHorizontal className="h-3 w-3" />
                Fields
              </button>
              <button
                onClick={() => {
                  syncJsonFromForm()
                  setMode('json')
                }}
                className={cn(
                  'flex items-center gap-1 rounded-md px-2 py-1 text-[11px]',
                  mode === 'json' ? 'bg-surface text-text shadow-sm' : 'text-fg-secondary',
                )}
              >
                <Code2 className="h-3 w-3" />
                JSON
              </button>
            </div>
          </div>

          {mode === 'form' ? (
            <div className="space-y-1.5 max-h-[320px] overflow-y-auto pr-1">
              {featureOrder.length === 0 ? (
                <p className="text-[11px] text-fg-tertiary">Loading feature schema…</p>
              ) : (
                featureOrder.map((name) => (
                  <div key={name} className="flex items-center gap-2">
                    <label className="text-[11px] font-mono text-fg-secondary w-36 truncate">
                      {name}
                    </label>
                    <input
                      type="number"
                      value={Number.isFinite(features[name]) ? features[name] : 0}
                      onChange={(e) =>
                        setFeatures((p) => ({ ...p, [name]: parseFloat(e.target.value) || 0 }))
                      }
                      className="flex-1 h-7 rounded-md border border-border bg-surface px-2 text-[11px] font-mono text-text focus:outline-none focus:border-accent"
                    />
                  </div>
                ))
              )}
            </div>
          ) : (
            <textarea
              value={jsonText}
              onChange={(e) => {
                setJsonText(e.target.value)
                try {
                  JSON.parse(e.target.value)
                  setJsonError(null)
                } catch {
                  setJsonError('Invalid JSON')
                }
              }}
              rows={10}
              spellCheck={false}
              className={cn(
                'w-full rounded-lg border bg-surface px-2.5 py-2 text-xs font-mono text-text focus:outline-none resize-none',
                jsonError ? 'border-neg' : 'border-border focus:border-accent',
              )}
            />
          )}
          {jsonError && <p className="text-[11px] text-neg mt-1">{jsonError}</p>}
        </div>

        <Button className="w-full text-sm" onClick={handleRun} disabled={running || !canRun}>
          {running ? (
            <>
              <Loader2 className="h-4 w-4 animate-spin" />
              Running…
            </>
          ) : (
            <>
              <Play className="h-4 w-4" />
              Run Test{selectedVersions.length > 1 ? ` (${selectedVersions.length} versions)` : ''}
            </>
          )}
        </Button>

        {/* Save as test case */}
        <div className="flex gap-2">
          <input
            type="text"
            value={saveName}
            onChange={(e) => setSaveName(e.target.value)}
            placeholder="Name this case…"
            className="flex-1 h-8 rounded-lg border border-border bg-surface px-2.5 text-xs text-text placeholder:text-fg-tertiary focus:outline-none focus:border-accent"
          />
          <Button
            variant="outline"
            className="text-xs h-8"
            onClick={handleSaveCase}
            disabled={!saveName.trim() || addCase.isPending}
          >
            <Save className="h-3.5 w-3.5" />
            Save
          </Button>
        </div>
      </div>

      {/* Results panel */}
      <div className="space-y-4">
        <h3 className="text-sm font-medium text-text">Results</h3>

        {results.length === 0 && !running && (
          <div className="rounded-xl border border-dashed border-border p-12 flex items-center justify-center">
            <p className="text-sm text-fg-tertiary">Run a test to see predictions here.</p>
          </div>
        )}

        {running && (
          <div className="rounded-xl border border-border p-12 flex items-center justify-center">
            <Loader2 className="h-8 w-8 animate-spin text-accent" />
          </div>
        )}

        {results.map((r) => (
          <div key={r.version} className="rounded-xl border border-border bg-surface p-4 space-y-3">
            <div className="flex items-center justify-between">
              <span className="text-xs font-medium text-fg-secondary">Version v{r.version}</span>
              {r.latency_ms !== undefined && (
                <span className="font-mono text-xs text-fg-tertiary">{r.latency_ms} ms</span>
              )}
            </div>
            {r.error ? (
              <p className="text-sm text-neg">{r.error}</p>
            ) : r.forecast ? (
              <>
                <div className="flex items-center justify-between">
                  <DirectionPill direction={r.forecast.direction} />
                  <span className="font-mono text-sm text-text">
                    {r.forecast.magnitude}
                    <span className="text-fg-tertiary ml-1 text-xs">{r.forecast.horizon}</span>
                  </span>
                </div>
                <div>
                  <p className="text-[11px] text-fg-secondary mb-1">Confidence</p>
                  <ConfidenceBar value={r.forecast.confidence} />
                </div>
              </>
            ) : (
              <p className="text-sm text-fg-tertiary">No prediction returned.</p>
            )}
          </div>
        ))}

        {/* Saved cases */}
        {testCases && testCases.length > 0 && (
          <div className="rounded-xl border border-border bg-sunken p-4">
            <p className="text-xs font-medium text-fg-secondary mb-2">Saved cases</p>
            <div className="space-y-1.5">
              {testCases.map((c) => (
                <div key={c.case_id} className="flex items-center gap-2 text-xs">
                  <span className="text-text truncate flex-1">{c.name}</span>
                  <button
                    onClick={() => replayCase(c.input)}
                    className="flex items-center gap-1 rounded-md border border-border px-2 py-0.5 text-fg-secondary hover:text-text hover:border-border-2"
                  >
                    <RotateCcw className="h-3 w-3" />
                    Load
                  </button>
                  <button
                    onClick={() => deleteCase.mutate(c.case_id)}
                    className="rounded-md border border-border px-1.5 py-0.5 text-fg-tertiary hover:text-neg"
                  >
                    <Trash2 className="h-3 w-3" />
                  </button>
                </div>
              ))}
            </div>
          </div>
        )}
      </div>
    </div>
  )
}
