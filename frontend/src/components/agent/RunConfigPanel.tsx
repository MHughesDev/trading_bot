// Agent run configuration: provider → model → goal → budgets → start.
// Models are listed live from the provider once a credential is configured
// (Ollama needs none). The model field is a free-type combobox — anything the
// provider serves can be typed even if not in the list.

import { useMemo, useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Loader2, Play, Settings2 } from 'lucide-react'
import { cn } from '@/lib/utils'
import {
  agentApi,
  llmApi,
  LLM_PROVIDERS,
  type LlmProvider,
  type StartRunRequest,
} from '@/api/agent'
import { coverageApi } from '@/api/backtests'
import { LlmCredentialForm } from './LlmCredentialForm'

const TIMEFRAMES = ['1m', '5m', '15m', '1h', '4h', '1d']

export function RunConfigPanel({ onStarted }: { onStarted: (runId: string) => void }) {
  const qc = useQueryClient()
  const [provider, setProvider] = useState<LlmProvider>('anthropic')
  const [model, setModel] = useState('')
  const [goal, setGoal] = useState('')
  const [instrument, setInstrument] = useState('')
  const [timeframe, setTimeframe] = useState('')
  const [maxIterations, setMaxIterations] = useState(15)
  const [budgetHours, setBudgetHours] = useState(4)
  const [showCreds, setShowCreds] = useState(false)
  const [error, setError] = useState('')

  const { data: creds } = useQuery({
    queryKey: ['llm-credentials'],
    queryFn: () => llmApi.status().then((r) => r.data),
  })
  const providerStatus = creds?.providers.find((p) => p.provider === provider)
  const providerReady = provider === 'ollama' || (providerStatus?.configured ?? false)

  const { data: models, isFetching: modelsLoading } = useQuery({
    queryKey: ['llm-models', provider],
    queryFn: () => llmApi.listModels(provider).then((r) => r.data.models),
    enabled: providerReady,
    staleTime: 60_000,
    retry: false,
  })

  const { data: coverage } = useQuery({
    queryKey: ['backtest-coverage'],
    queryFn: () => coverageApi.list().then((r) => r.data.coverage),
    staleTime: 60_000,
  })
  const instruments = useMemo(
    () => [...new Set((coverage ?? []).map((c) => c.instrument_id))].sort(),
    [coverage],
  )

  const startMutation = useMutation({
    mutationFn: async () => {
      setError('')
      const req: StartRunRequest = {
        goal: goal.trim(),
        provider,
        model: model.trim(),
        max_iterations: maxIterations,
        wallclock_budget_secs: budgetHours * 3600,
        constraints: {},
      }
      if (instrument) req.constraints!.instrument_id = instrument
      if (timeframe) req.constraints!.timeframe = timeframe
      const res = await agentApi.start(req)
      return res.data.run_id
    },
    onSuccess: (runId) => {
      qc.invalidateQueries({ queryKey: ['agent-runs'] })
      onStarted(runId)
    },
    onError: (e: unknown) => {
      const detail =
        (e as { response?: { data?: { message?: string } } })?.response?.data?.message
      setError(detail ?? 'Failed to start the run.')
    },
  })

  const canStart = providerReady && model.trim() && goal.trim() && !startMutation.isPending

  return (
    <div className="rounded-xl border border-border bg-surface-2 p-4 space-y-3">
      <div className="flex items-center justify-between">
        <h2 className="text-sm font-semibold text-text">New agent run</h2>
        <button
          onClick={() => setShowCreds((s) => !s)}
          className="flex items-center gap-1 text-xs text-text-muted hover:text-text transition-colors"
        >
          <Settings2 className="h-3.5 w-3.5" />
          Providers
        </button>
      </div>

      {showCreds && <LlmCredentialForm />}

      <div>
        <label className="block text-xs text-text-dim mb-1">Provider</label>
        <select
          value={provider}
          onChange={(e) => {
            setProvider(e.target.value as LlmProvider)
            setModel('')
          }}
          className="w-full rounded-lg px-3 py-1.5 text-sm bg-surface border border-border text-text focus:outline-none focus:ring-1 focus:ring-accent"
        >
          {LLM_PROVIDERS.map((p) => (
            <option key={p.id} value={p.id}>
              {p.label}
            </option>
          ))}
        </select>
        {!providerReady && (
          <p className="text-xs text-amber-400 mt-1">
            No {provider} credential saved — open Providers above to connect one.
          </p>
        )}
      </div>

      <div>
        <label className="block text-xs text-text-dim mb-1">
          Model {modelsLoading && <Loader2 className="inline h-3 w-3 animate-spin" />}
        </label>
        <input
          type="text"
          list="agent-model-options"
          value={model}
          onChange={(e) => setModel(e.target.value)}
          placeholder={providerReady ? 'Pick or type a model id' : 'Configure the provider first'}
          disabled={!providerReady}
          className="w-full rounded-lg px-3 py-1.5 text-sm bg-surface border border-border text-text placeholder:text-text-dim focus:outline-none focus:ring-1 focus:ring-accent disabled:opacity-50"
        />
        <datalist id="agent-model-options">
          {(models ?? []).map((m) => (
            <option key={m.id} value={m.id}>
              {m.display_name ?? m.id}
            </option>
          ))}
        </datalist>
      </div>

      <div>
        <label className="block text-xs text-text-dim mb-1">Goal</label>
        <textarea
          value={goal}
          onChange={(e) => setGoal(e.target.value)}
          rows={4}
          placeholder="e.g. Design an EMA-crossover strategy for BTC-USD on 1h bars, backtest the last 90 days, and iterate until it beats buy-and-hold with a drawdown under 15%."
          className="w-full rounded-lg px-3 py-1.5 text-sm bg-surface border border-border text-text placeholder:text-text-dim focus:outline-none focus:ring-1 focus:ring-accent resize-y"
        />
      </div>

      <div className="grid grid-cols-2 gap-2">
        <div>
          <label className="block text-xs text-text-dim mb-1">Instrument (optional)</label>
          <input
            type="text"
            list="agent-instrument-options"
            value={instrument}
            onChange={(e) => setInstrument(e.target.value)}
            placeholder="Agent picks"
            className="w-full rounded-lg px-3 py-1.5 text-sm bg-surface border border-border text-text placeholder:text-text-dim focus:outline-none focus:ring-1 focus:ring-accent"
          />
          <datalist id="agent-instrument-options">
            {instruments.map((i) => (
              <option key={i} value={i} />
            ))}
          </datalist>
        </div>
        <div>
          <label className="block text-xs text-text-dim mb-1">Timeframe (optional)</label>
          <select
            value={timeframe}
            onChange={(e) => setTimeframe(e.target.value)}
            className="w-full rounded-lg px-3 py-1.5 text-sm bg-surface border border-border text-text focus:outline-none focus:ring-1 focus:ring-accent"
          >
            <option value="">Agent picks</option>
            {TIMEFRAMES.map((tf) => (
              <option key={tf} value={tf}>
                {tf}
              </option>
            ))}
          </select>
        </div>
      </div>

      <div className="grid grid-cols-2 gap-2">
        <div>
          <label className="block text-xs text-text-dim mb-1">Max iterations</label>
          <input
            type="number"
            min={1}
            max={200}
            value={maxIterations}
            onChange={(e) => setMaxIterations(Number(e.target.value) || 15)}
            className="w-full rounded-lg px-3 py-1.5 text-sm bg-surface border border-border text-text focus:outline-none focus:ring-1 focus:ring-accent"
          />
        </div>
        <div>
          <label className="block text-xs text-text-dim mb-1">Time budget (hours)</label>
          <input
            type="number"
            min={1}
            max={24}
            value={budgetHours}
            onChange={(e) => setBudgetHours(Number(e.target.value) || 4)}
            className="w-full rounded-lg px-3 py-1.5 text-sm bg-surface border border-border text-text focus:outline-none focus:ring-1 focus:ring-accent"
          />
        </div>
      </div>

      {error && (
        <div className="rounded-lg bg-red-500/10 border border-red-500/20 px-3 py-2 text-xs text-red-400">
          {error}
        </div>
      )}

      <button
        disabled={!canStart}
        onClick={() => startMutation.mutate()}
        className={cn(
          'flex w-full items-center justify-center gap-1.5 rounded-lg px-3 py-2 text-sm font-medium transition-colors',
          'bg-accent text-white hover:bg-accent/80 disabled:opacity-40',
        )}
      >
        {startMutation.isPending ? (
          <Loader2 className="h-4 w-4 animate-spin" />
        ) : (
          <Play className="h-4 w-4" />
        )}
        {startMutation.isPending ? 'Starting…' : 'Start run'}
      </button>
    </div>
  )
}
