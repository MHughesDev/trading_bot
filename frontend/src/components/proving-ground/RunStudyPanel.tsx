// The manual Study runner — the "open exploration" surface (Phase 1).
//
// Every Study kind the engine accepts is reachable here. Honesty is preserved
// not by hiding the controls (the old funnel-only design) but by the ledger:
// the backend auto-increments the experiment's trial counter on every run and
// deflates every significance number by it. So you may explore freely, but each
// look permanently raises your significance bar — there are no free looks.
//
// The kind↔vary pairing mirrors `StudyConfig::validate` (study/config.rs) exactly
// so an ill-typed request is never sent.
import { useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { AlertTriangle, FlaskConical, Loader2 } from 'lucide-react'
import {
  experimentsApi,
  type StudyKind,
  type VarySpec,
  type SelectionRule,
} from '@/api/experiments'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
import { toast } from '@/hooks/useToast'

const STUDY_KINDS: { value: StudyKind; label: string; blurb: string }[] = [
  { value: 'parameter_sweep', label: 'Parameter sweep', blurb: 'Perf across an explicit param grid.' },
  { value: 'neighborhood', label: 'Neighborhood', blurb: 'Plateau vs spike around one param.' },
  { value: 'walk_forward', label: 'Walk-forward', blurb: 'Rolling in-sample / out-of-sample windows.' },
  { value: 'cpcv', label: 'CPCV', blurb: 'Combinatorial purged cross-validation.' },
  { value: 'nested_cv', label: 'Nested CV', blurb: 'Nested purged cross-validation.' },
  { value: 'permutation_null', label: 'Permutation null', blurb: 'Skill vs luck against a chosen null.' },
  { value: 'synthetic_paths', label: 'Synthetic paths', blurb: 'Resample synthetic price paths.' },
  { value: 'cost_sweep', label: 'Cost sweep', blurb: 'Optimistic→pessimistic cost ladder.' },
  { value: 'trade_monte_carlo', label: 'Trade Monte Carlo', blurb: 'Block-bootstrap the trade list.' },
  { value: 'regime_conditional', label: 'Regime conditional', blurb: 'Perf within labeled regimes.' },
]

const METRICS = [
  'sharpe',
  'sortino',
  'calmar',
  'cagr',
  'total_return',
  'detrended_sharpe',
  'max_drawdown',
  'profit_factor',
]

const SELECTION_RULES: { value: SelectionRule; label: string }[] = [
  { value: 'none', label: 'Carry nothing forward' },
  { value: 'median_stable_centroid', label: 'Median-stable centroid' },
  { value: 'worst_case_robust', label: 'Worst-case robust' },
]

export function RunStudyPanel({ expId, onRan }: { expId: string; onRan?: () => void }) {
  const qc = useQueryClient()
  const [kind, setKind] = useState<StudyKind>('parameter_sweep')
  const [metric, setMetric] = useState('sharpe')
  const [question, setQuestion] = useState('')
  const [selectionRule, setSelectionRule] = useState<SelectionRule>('none')

  // Vary-specific fields (only the ones for the selected kind are read).
  const [param, setParam] = useState('fast')
  const [center, setCenter] = useState('12')
  const [step, setStep] = useState('1')
  const [k, setK] = useState('3')
  const [nGroups, setNGroups] = useState('6')
  const [kTest, setKTest] = useState('2')
  const [seeds, setSeeds] = useState('200')
  const [mcN, setMcN] = useState('1000')
  const [mcBlock, setMcBlock] = useState('10')
  const [costRefs, setCostRefs] = useState('cost:optimistic, cost:base, cost:pessimistic')
  const [json, setJson] = useState('[]')

  // For permutation_null we must pass the chosen null's id (INV-3).
  const nulls = useQuery({
    queryKey: ['suite', 'experiment', expId, 'nulls'],
    queryFn: () => experimentsApi.nullPicker(expId).then((r) => r.data),
  })
  const chosenNullId = nulls.data?.chosen?.null_id ?? null
  const needsNull = kind === 'permutation_null'

  const run = useMutation({
    mutationFn: () => {
      const vary = buildVary()
      return experimentsApi.runStudy(expId, {
        study_id: `study-${crypto.randomUUID()}`,
        kind,
        vary,
        metric,
        question: question.trim(),
        selection_rule: selectionRule,
        ...(needsNull ? { null_ref: chosenNullId ?? undefined } : {}),
      })
    },
    onSuccess: () => {
      toast({ title: 'Study run — trial counter incremented' })
      setQuestion('')
      qc.invalidateQueries({ queryKey: ['suite', 'experiment', expId] })
      qc.invalidateQueries({ queryKey: ['suite', 'experiments'] })
      onRan?.()
    },
    onError: (e: unknown) => toast({ title: errMsg(e, 'Study refused'), variant: 'error' }),
  })

  function buildVary(): VarySpec {
    switch (kind) {
      case 'parameter_sweep':
        return { vary: 'params', grid: parseJson(json) as Record<string, unknown>[] }
      case 'neighborhood':
        return { vary: 'neighborhood', param, center: +center, step: +step, k: +k }
      case 'walk_forward':
        return { vary: 'data_windows', windows: parseJson(json) as [string, string][] }
      case 'cpcv':
      case 'nested_cv':
        return { vary: 'cpcv_groups', n_groups: +nGroups, k_test: +kTest }
      case 'permutation_null':
      case 'synthetic_paths':
        return { vary: 'seeds', n: +seeds }
      case 'cost_sweep':
        return {
          vary: 'cost_ladder',
          cost_model_refs: costRefs.split(',').map((s) => s.trim()).filter(Boolean),
        }
      case 'trade_monte_carlo':
        return { vary: 'trade_resamples', n: +mcN, block: +mcBlock }
      case 'regime_conditional':
        return { vary: 'regimes', windows: parseJson(json) as [string, string, string][] }
    }
  }

  // Client-side mirror of StudyConfig::validate so we disable rather than 500.
  const invalid = validate()
  function validate(): string | null {
    if (!question.trim()) return 'A pre-declared question is required (logged before the run).'
    if (needsNull && !chosenNullId) return 'Choose a null for this experiment before a permutation test.'
    if ((kind === 'cpcv' || kind === 'nested_cv') && !(+kTest > 0 && +kTest < +nGroups))
      return 'CPCV requires 0 < k_test < n_groups.'
    if ((kind === 'parameter_sweep' || kind === 'walk_forward' || kind === 'regime_conditional') &&
      parseJsonSafe(json) === undefined)
      return 'The list field must be valid JSON.'
    return null
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2 text-sm">
          <FlaskConical className="h-4 w-4 text-blue-400" />
          Run a study
        </CardTitle>
      </CardHeader>
      <CardContent className="flex flex-col gap-3">
        {/* The ledger contract, stated up front. */}
        <div className="flex items-start gap-2 rounded-md border border-amber-500/30 bg-amber-500/10 p-2 text-xs text-amber-200">
          <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
          <span>
            Every run permanently increments this experiment's trial counter and raises the
            significance bar it must clear. Explore freely — but there are no free looks.
          </span>
        </div>

        <Field label="Study kind">
          <Select value={kind} onValueChange={(v) => setKind(v as StudyKind)}>
            <SelectTrigger><SelectValue /></SelectTrigger>
            <SelectContent>
              {STUDY_KINDS.map((s) => (
                <SelectItem key={s.value} value={s.value}>{s.label}</SelectItem>
              ))}
            </SelectContent>
          </Select>
          <p className="mt-1 text-[11px] text-text-dim">
            {STUDY_KINDS.find((s) => s.value === kind)?.blurb}
          </p>
        </Field>

        {/* Vary-specific fields. */}
        {kind === 'neighborhood' && (
          <div className="grid grid-cols-4 gap-2">
            <Field label="Param"><Input value={param} onChange={(e) => setParam(e.target.value)} /></Field>
            <Field label="Center"><Input value={center} onChange={(e) => setCenter(e.target.value)} type="number" /></Field>
            <Field label="Step"><Input value={step} onChange={(e) => setStep(e.target.value)} type="number" /></Field>
            <Field label="±k steps"><Input value={k} onChange={(e) => setK(e.target.value)} type="number" /></Field>
          </div>
        )}
        {(kind === 'cpcv' || kind === 'nested_cv') && (
          <div className="grid grid-cols-2 gap-2">
            <Field label="n_groups"><Input value={nGroups} onChange={(e) => setNGroups(e.target.value)} type="number" /></Field>
            <Field label="k_test"><Input value={kTest} onChange={(e) => setKTest(e.target.value)} type="number" /></Field>
          </div>
        )}
        {(kind === 'permutation_null' || kind === 'synthetic_paths') && (
          <Field label="Seeds (n)"><Input value={seeds} onChange={(e) => setSeeds(e.target.value)} type="number" /></Field>
        )}
        {kind === 'trade_monte_carlo' && (
          <div className="grid grid-cols-2 gap-2">
            <Field label="Resamples (n)"><Input value={mcN} onChange={(e) => setMcN(e.target.value)} type="number" /></Field>
            <Field label="Block size"><Input value={mcBlock} onChange={(e) => setMcBlock(e.target.value)} type="number" /></Field>
          </div>
        )}
        {kind === 'cost_sweep' && (
          <Field label="Cost-model refs (optimistic → pessimistic, comma-separated)">
            <Input value={costRefs} onChange={(e) => setCostRefs(e.target.value)} />
          </Field>
        )}
        {(kind === 'parameter_sweep' || kind === 'walk_forward' || kind === 'regime_conditional') && (
          <Field
            label={
              kind === 'parameter_sweep'
                ? 'Param grid (JSON array of override maps)'
                : kind === 'walk_forward'
                  ? 'Windows (JSON: [[start, end], …])'
                  : 'Regimes (JSON: [[start, end, label], …])'
            }
          >
            <textarea
              value={json}
              onChange={(e) => setJson(e.target.value)}
              rows={3}
              spellCheck={false}
              className="w-full rounded-md border border-border-2 bg-surface px-2 py-1 font-mono text-xs text-text"
            />
          </Field>
        )}

        {needsNull && (
          <p className="text-[11px] text-text-dim">
            Null:{' '}
            {chosenNullId ? (
              <span className="font-mono text-text">{nulls.data?.chosen?.kind}</span>
            ) : (
              <span className="text-pnl-down">none chosen</span>
            )}
          </p>
        )}

        <div className="grid grid-cols-2 gap-2">
          <Field label="Metric">
            <Select value={metric} onValueChange={setMetric}>
              <SelectTrigger><SelectValue /></SelectTrigger>
              <SelectContent>
                {METRICS.map((m) => (
                  <SelectItem key={m} value={m}>{m}</SelectItem>
                ))}
              </SelectContent>
            </Select>
          </Field>
          <Field label="Carry-forward rule">
            <Select value={selectionRule} onValueChange={(v) => setSelectionRule(v as SelectionRule)}>
              <SelectTrigger><SelectValue /></SelectTrigger>
              <SelectContent>
                {SELECTION_RULES.map((r) => (
                  <SelectItem key={r.value} value={r.value}>{r.label}</SelectItem>
                ))}
              </SelectContent>
            </Select>
          </Field>
        </div>

        <Field label="Question (pre-declared, logged before the run)">
          <Input
            value={question}
            onChange={(e) => setQuestion(e.target.value)}
            placeholder="e.g. Does the edge survive realistic costs?"
          />
        </Field>

        <div className="flex items-center justify-between">
          <span className="text-[11px] text-pnl-down">{invalid ?? ''}</span>
          <Button size="sm" disabled={!!invalid || run.isPending} onClick={() => run.mutate()}>
            {run.isPending && <Loader2 className="h-3 w-3 animate-spin" />}
            Run study
          </Button>
        </div>
      </CardContent>
    </Card>
  )
}

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="flex flex-col gap-1">
      <Label className="text-[11px] text-text-dim">{label}</Label>
      {children}
    </div>
  )
}

function parseJson(s: string): unknown {
  return JSON.parse(s)
}
function parseJsonSafe(s: string): unknown {
  try {
    return JSON.parse(s)
  } catch {
    return undefined
  }
}
function errMsg(e: unknown, fallback: string): string {
  const resp = (e as { response?: { data?: { message?: string } } })?.response
  return resp?.data?.message ?? fallback
}
