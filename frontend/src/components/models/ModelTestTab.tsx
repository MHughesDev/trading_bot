import { useState } from 'react'
import { Play, Loader2, Save } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import { modelsApi } from '@/api/models'
import { useModelVersions } from '@/hooks/useModels'

interface Props {
  modelId: string
}

interface TestResult {
  direction?: string
  confidence?: number
  latency_ms?: number
  cost_usd?: number
  raw?: unknown
}

const DEFAULT_FEATURES = JSON.stringify(
  {
    close: 45000.0,
    volume: 1234567.89,
    rsi_14: 58.3,
    ema_20: 44800.0,
    atr_14: 420.5,
  },
  null,
  2,
)

export function ModelTestTab({ modelId }: Props) {
  const { data: versions } = useModelVersions(modelId)
  const [instrumentId, setInstrumentId] = useState('BTC/USDT.BINANCE')
  const [featuresText, setFeaturesText] = useState(DEFAULT_FEATURES)
  const [featuresError, setFeaturesError] = useState<string | null>(null)
  const [selectedVersion, setSelectedVersion] = useState<string>('')
  const [isRunning, setIsRunning] = useState(false)
  const [result, setResult] = useState<TestResult | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [saved, setSaved] = useState(false)

  const activeVersions = versions?.filter(
    (v) => v.status === 'active' || v.status === 'candidate',
  ) ?? []

  const versionToTest =
    selectedVersion
      ? parseInt(selectedVersion, 10)
      : activeVersions[0]?.version

  function handleFeaturesChange(text: string) {
    setFeaturesText(text)
    try {
      JSON.parse(text)
      setFeaturesError(null)
    } catch {
      setFeaturesError('Invalid JSON')
    }
  }

  async function handleRun() {
    if (!versionToTest) return
    let features: Record<string, number>
    try {
      features = JSON.parse(featuresText)
    } catch {
      setFeaturesError('Invalid JSON')
      return
    }

    setIsRunning(true)
    setError(null)
    setResult(null)
    setSaved(false)
    try {
      const start = performance.now()
      const res = await modelsApi.testVersion(modelId, versionToTest, {
        instrument_id: instrumentId,
        features,
      })
      const latency = Math.round(performance.now() - start)
      const raw = res.data as Record<string, unknown>
      setResult({
        direction: raw?.direction as string | undefined,
        confidence: raw?.confidence as number | undefined,
        latency_ms: (raw?.latency_ms as number | undefined) ?? latency,
        cost_usd: raw?.cost_usd as number | undefined,
        raw,
      })
    } catch (err) {
      setError(String(err))
    } finally {
      setIsRunning(false)
    }
  }

  async function handleSaveCase() {
    if (!result || !versionToTest) return
    try {
      await modelsApi.testVersion(modelId, versionToTest, {
        instrument_id: instrumentId,
        features: JSON.parse(featuresText),
      })
      setSaved(true)
    } catch {
      // ignore for save case
    }
  }

  return (
    <div className="grid grid-cols-1 lg:grid-cols-2 gap-5">
      {/* Input panel */}
      <div className="space-y-4">
        <h3 className="text-sm font-medium text-text">Test inputs</h3>

        {/* Version selector */}
        {versions && versions.length > 0 && (
          <div>
            <label className="block text-xs font-medium text-text-muted mb-1.5">
              Version
            </label>
            <select
              value={selectedVersion}
              onChange={(e) => setSelectedVersion(e.target.value)}
              className="w-full h-8 rounded-lg border border-border bg-surface px-2.5 text-xs text-text focus:outline-none focus:border-accent"
            >
              <option value="">Latest active</option>
              {versions
                .sort((a, b) => b.version - a.version)
                .map((v) => (
                  <option
                    key={v.version}
                    value={v.version}
                    disabled={v.status !== 'active' && v.status !== 'candidate'}
                  >
                    v{v.version} ({v.status})
                  </option>
                ))}
            </select>
          </div>
        )}

        {/* Instrument */}
        <div>
          <label className="block text-xs font-medium text-text-muted mb-1.5">
            Instrument ID
          </label>
          <input
            type="text"
            value={instrumentId}
            onChange={(e) => setInstrumentId(e.target.value)}
            placeholder="BTC/USDT.BINANCE"
            className="w-full h-8 rounded-lg border border-border bg-surface px-2.5 text-xs text-text font-mono placeholder:text-text-dim focus:outline-none focus:border-accent"
          />
        </div>

        {/* Features */}
        <div>
          <label className="block text-xs font-medium text-text-muted mb-1.5">
            Features (JSON)
          </label>
          <textarea
            value={featuresText}
            onChange={(e) => handleFeaturesChange(e.target.value)}
            rows={12}
            className={cn(
              'w-full rounded-lg border bg-surface px-2.5 py-2 text-xs font-mono text-text focus:outline-none resize-none',
              featuresError ? 'border-pnl-down' : 'border-border focus:border-accent',
            )}
          />
          {featuresError && (
            <p className="text-xs text-pnl-down mt-1">{featuresError}</p>
          )}
        </div>

        <Button
          className="w-full text-sm"
          onClick={handleRun}
          disabled={isRunning || !versionToTest || !!featuresError}
        >
          {isRunning ? (
            <>
              <Loader2 className="h-4 w-4 animate-spin" />
              Running…
            </>
          ) : (
            <>
              <Play className="h-4 w-4" />
              Run Test
            </>
          )}
        </Button>
      </div>

      {/* Results panel */}
      <div className="space-y-4">
        <h3 className="text-sm font-medium text-text">Results</h3>

        {error && (
          <div className="rounded-xl border border-pnl-down/20 bg-pnl-down/10 p-4">
            <p className="text-sm text-pnl-down">{error}</p>
          </div>
        )}

        {!result && !error && !isRunning && (
          <div className="rounded-xl border border-dashed border-border p-12 flex items-center justify-center">
            <p className="text-sm text-text-dim">Run a test to see results here.</p>
          </div>
        )}

        {isRunning && (
          <div className="rounded-xl border border-border p-12 flex items-center justify-center">
            <Loader2 className="h-8 w-8 animate-spin text-accent" />
          </div>
        )}

        {result && !isRunning && (
          <>
            {/* Key metrics */}
            <div className="grid grid-cols-2 gap-3">
              {result.direction !== undefined && (
                <div className="rounded-xl border border-border bg-surface p-4">
                  <p className="text-xs text-text-muted mb-1">Direction</p>
                  <p
                    className={cn(
                      'text-lg font-semibold capitalize',
                      result.direction === 'bullish'
                        ? 'text-pnl-up'
                        : result.direction === 'bearish'
                          ? 'text-pnl-down'
                          : 'text-text',
                    )}
                  >
                    {result.direction}
                  </p>
                </div>
              )}

              {result.confidence !== undefined && (
                <div className="rounded-xl border border-border bg-surface p-4">
                  <p className="text-xs text-text-muted mb-1">Confidence</p>
                  <p className="text-lg font-semibold font-mono text-text">
                    {(result.confidence * 100).toFixed(1)}%
                  </p>
                </div>
              )}

              {result.latency_ms !== undefined && (
                <div className="rounded-xl border border-border bg-surface p-4">
                  <p className="text-xs text-text-muted mb-1">Latency</p>
                  <p className="text-lg font-semibold font-mono text-text">
                    {result.latency_ms}
                    <span className="text-xs text-text-muted ml-1">ms</span>
                  </p>
                </div>
              )}

              {result.cost_usd !== undefined && (
                <div className="rounded-xl border border-border bg-surface p-4">
                  <p className="text-xs text-text-muted mb-1">Cost</p>
                  <p className="text-lg font-semibold font-mono text-text">
                    ${result.cost_usd.toFixed(6)}
                  </p>
                </div>
              )}
            </div>

            {/* Raw response */}
            <div className="rounded-xl border border-border bg-surface-2 p-4">
              <p className="text-xs text-text-muted mb-2">Raw response</p>
              <pre className="text-xs font-mono text-text overflow-x-auto">
                {JSON.stringify(result.raw, null, 2)}
              </pre>
            </div>

            <Button
              variant="outline"
              className="w-full text-sm"
              onClick={handleSaveCase}
              disabled={saved}
            >
              <Save className="h-4 w-4" />
              {saved ? 'Saved as test case' : 'Save as test case'}
            </Button>
          </>
        )}
      </div>
    </div>
  )
}
