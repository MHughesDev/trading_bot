/**
 * I-6.7 — Ensemble Builder page.
 *
 * Lists existing ensembles and allows creating new ones by selecting
 * member models and a combiner strategy (LOP / CRPS-weighted / stacking).
 */

import { useState } from 'react'
import { useNavigate } from 'react-router-dom'
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { api as client } from '@/lib/api'
import { Plus, Loader2, GitMerge, ChevronRight, X, Check } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'

// ── Types ──────────────────────────────────────────────────────────────────

type CombinerKind = 'linear_opinion_pool' | 'crps_weighted' | 'stacking'

interface EnsembleRecord {
  ensemble_id: string
  slug: string
  display_name: string
  description?: string
  combiner: CombinerKind
  member_model_ids: string[]
  created_at: string
}

interface CreateEnsembleRequest {
  display_name: string
  description?: string
  combiner: CombinerKind
  member_model_ids: string[]
}

interface ModelOption {
  model_id: string
  slug: string
  display_name: string
  model_kind: string
}

// ── API hooks ──────────────────────────────────────────────────────────────

function useEnsembles() {
  return useQuery({
    queryKey: ['ensembles'],
    queryFn: () =>
      client.get<{ ensembles: EnsembleRecord[] }>('/api/ensembles').then((r) => r.data),
  })
}

function useForecasterModels() {
  return useQuery({
    queryKey: ['models-forecasters'],
    queryFn: () =>
      client
        .get<{ models: ModelOption[] }>('/api/models', { params: { kind: 'forecaster' } })
        .then((r) => r.data),
  })
}

function useCreateEnsemble() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (req: CreateEnsembleRequest) =>
      client.post<EnsembleRecord>('/api/ensembles', req).then((r) => r.data),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['ensembles'] }),
  })
}

// ── Combiner badge ─────────────────────────────────────────────────────────

const COMBINER_LABELS: Record<CombinerKind, string> = {
  linear_opinion_pool: 'LOP',
  crps_weighted: 'CRPS-weighted',
  stacking: 'Stacking',
}

const COMBINER_DESCRIPTIONS: Record<CombinerKind, string> = {
  linear_opinion_pool: 'Equal-weight average of quantile CDFs',
  crps_weighted: 'Weights proportional to inverse CRPS score',
  stacking: 'Meta-model learns optimal weights from evaluation data',
}

function CombinerBadge({ kind }: { kind: CombinerKind }) {
  const colors: Record<CombinerKind, string> = {
    linear_opinion_pool: 'bg-blue-500/15 text-blue-400 border-blue-500/30',
    crps_weighted: 'bg-purple-500/15 text-purple-400 border-purple-500/30',
    stacking: 'bg-emerald-500/15 text-emerald-400 border-emerald-500/30',
  }
  return (
    <span
      className={cn(
        'inline-flex items-center rounded-md border px-2 py-0.5 text-xs font-medium',
        colors[kind],
      )}
    >
      {COMBINER_LABELS[kind]}
    </span>
  )
}

// ── Create form ────────────────────────────────────────────────────────────

function CreateEnsembleForm({ onClose }: { onClose: () => void }) {
  const [name, setName] = useState('')
  const [description, setDescription] = useState('')
  const [combiner, setCombiner] = useState<CombinerKind>('linear_opinion_pool')
  const [selectedIds, setSelectedIds] = useState<string[]>([])

  const { data: modelsData, isLoading: modelsLoading } = useForecasterModels()
  const createMut = useCreateEnsemble()

  const models = modelsData?.models ?? []

  function toggleModel(id: string) {
    setSelectedIds((prev) =>
      prev.includes(id) ? prev.filter((x) => x !== id) : [...prev, id],
    )
  }

  function submit(e: React.FormEvent) {
    e.preventDefault()
    if (!name.trim() || selectedIds.length < 2) return
    createMut.mutate(
      {
        display_name: name.trim(),
        description: description.trim() || undefined,
        combiner,
        member_model_ids: selectedIds,
      },
      { onSuccess: onClose },
    )
  }

  return (
    <form
      onSubmit={submit}
      className="rounded-xl border border-accent/30 bg-surface p-5 space-y-4"
    >
      <div className="flex items-center justify-between">
        <h3 className="text-sm font-semibold text-text">New Ensemble</h3>
        <button type="button" onClick={onClose} className="text-text-muted hover:text-text">
          <X className="h-4 w-4" />
        </button>
      </div>

      {/* Name */}
      <div className="space-y-1">
        <label className="text-xs text-text-muted">Display name</label>
        <input
          value={name}
          onChange={(e) => setName(e.target.value)}
          placeholder="My Ensemble"
          className="w-full rounded-lg border border-border bg-surface-2 px-3 py-2 text-sm text-text focus:border-accent focus:outline-none"
          required
        />
      </div>

      {/* Description */}
      <div className="space-y-1">
        <label className="text-xs text-text-muted">Description (optional)</label>
        <input
          value={description}
          onChange={(e) => setDescription(e.target.value)}
          placeholder="Combines BTC forecasters for daily trading"
          className="w-full rounded-lg border border-border bg-surface-2 px-3 py-2 text-sm text-text focus:border-accent focus:outline-none"
        />
      </div>

      {/* Combiner */}
      <div className="space-y-1">
        <label className="text-xs text-text-muted">Combiner strategy</label>
        <div className="grid grid-cols-1 sm:grid-cols-3 gap-2">
          {(Object.keys(COMBINER_LABELS) as CombinerKind[]).map((k) => (
            <button
              key={k}
              type="button"
              onClick={() => setCombiner(k)}
              className={cn(
                'rounded-lg border p-3 text-left transition-colors',
                combiner === k
                  ? 'border-accent bg-accent/10'
                  : 'border-border bg-surface-2 hover:border-border-hover',
              )}
            >
              <div className="text-xs font-medium text-text mb-1">{COMBINER_LABELS[k]}</div>
              <div className="text-xs text-text-muted">{COMBINER_DESCRIPTIONS[k]}</div>
            </button>
          ))}
        </div>
      </div>

      {/* Member models */}
      <div className="space-y-1">
        <label className="text-xs text-text-muted">
          Member models{' '}
          <span className="text-text-dim">
            ({selectedIds.length} selected — minimum 2)
          </span>
        </label>
        {modelsLoading ? (
          <div className="flex items-center gap-2 text-xs text-text-muted py-2">
            <Loader2 className="h-3.5 w-3.5 animate-spin" />
            Loading models…
          </div>
        ) : models.length === 0 ? (
          <p className="text-xs text-text-muted py-2">
            No forecaster models found. Create and train one first.
          </p>
        ) : (
          <div className="grid grid-cols-1 sm:grid-cols-2 gap-2 max-h-52 overflow-y-auto pr-1">
            {models.map((m) => {
              const sel = selectedIds.includes(m.model_id)
              return (
                <button
                  key={m.model_id}
                  type="button"
                  onClick={() => toggleModel(m.model_id)}
                  className={cn(
                    'flex items-center gap-2 rounded-lg border p-2.5 text-left transition-colors',
                    sel
                      ? 'border-accent bg-accent/10'
                      : 'border-border bg-surface-2 hover:border-border-hover',
                  )}
                >
                  <div
                    className={cn(
                      'flex h-4 w-4 items-center justify-center rounded border',
                      sel ? 'border-accent bg-accent' : 'border-border bg-transparent',
                    )}
                  >
                    {sel && <Check className="h-2.5 w-2.5 text-white" />}
                  </div>
                  <div className="min-w-0">
                    <div className="text-xs font-medium text-text truncate">{m.display_name}</div>
                    <div className="text-xs text-text-muted font-mono truncate">{m.slug}</div>
                  </div>
                </button>
              )
            })}
          </div>
        )}
      </div>

      <div className="flex items-center justify-end gap-2 pt-2">
        <Button type="button" variant="outline" size="sm" onClick={onClose}>
          Cancel
        </Button>
        <Button
          type="submit"
          size="sm"
          disabled={!name.trim() || selectedIds.length < 2 || createMut.isPending}
        >
          {createMut.isPending && <Loader2 className="h-3.5 w-3.5 animate-spin" />}
          Create Ensemble
        </Button>
      </div>

      {createMut.isError && (
        <p className="text-xs text-red-400">
          {(createMut.error as Error)?.message ?? 'Failed to create ensemble'}
        </p>
      )}
    </form>
  )
}

// ── Main page ──────────────────────────────────────────────────────────────

export function EnsemblesPage() {
  const navigate = useNavigate()
  const { data, isLoading } = useEnsembles()
  const [showCreate, setShowCreate] = useState(false)

  const ensembles = data?.ensembles ?? []

  return (
    <div className="mx-auto w-full max-w-4xl px-6 py-6">
      <div className="flex items-center justify-between mb-6">
        <div className="flex items-center gap-3">
          <div className="flex h-9 w-9 items-center justify-center rounded-xl bg-surface-2 text-accent">
            <GitMerge className="h-4.5 w-4.5" />
          </div>
          <div>
            <h1 className="text-lg font-semibold text-text">Ensemble Builder</h1>
            <p className="text-xs text-text-muted">
              Combine multiple forecasters into a calibrated ensemble
            </p>
          </div>
        </div>
        <Button size="sm" onClick={() => setShowCreate(true)} disabled={showCreate}>
          <Plus className="h-3.5 w-3.5" />
          New Ensemble
        </Button>
      </div>

      {showCreate && (
        <div className="mb-6">
          <CreateEnsembleForm onClose={() => setShowCreate(false)} />
        </div>
      )}

      {isLoading && (
        <div className="flex items-center gap-2 text-sm text-text-muted py-12 justify-center">
          <Loader2 className="h-4 w-4 animate-spin" />
          Loading ensembles…
        </div>
      )}

      {!isLoading && ensembles.length === 0 && !showCreate && (
        <div className="text-center py-16 rounded-xl border border-dashed border-border text-text-muted">
          <GitMerge className="h-8 w-8 mx-auto mb-3 opacity-40" />
          <p className="text-sm font-medium">No ensembles yet</p>
          <p className="text-xs mt-1">
            Create your first ensemble to combine multiple forecaster outputs.
          </p>
        </div>
      )}

      <div className="space-y-3">
        {ensembles.map((ens) => (
          <div
            key={ens.ensemble_id}
            className="flex items-center gap-4 rounded-xl border border-border bg-surface p-4 hover:border-border-hover transition-colors cursor-pointer group"
            onClick={() => navigate(`/ensembles/${ens.ensemble_id}`)}
          >
            <div className="flex h-9 w-9 items-center justify-center rounded-lg bg-surface-2 text-accent shrink-0">
              <GitMerge className="h-4.5 w-4.5" />
            </div>

            <div className="flex-1 min-w-0">
              <div className="flex items-center gap-2 flex-wrap">
                <span className="text-sm font-medium text-text">{ens.display_name}</span>
                <CombinerBadge kind={ens.combiner} />
              </div>
              <div className="flex items-center gap-3 mt-1">
                <span className="text-xs text-text-muted font-mono">{ens.slug}</span>
                <span className="text-xs text-text-muted">
                  {ens.member_model_ids.length} member
                  {ens.member_model_ids.length !== 1 ? 's' : ''}
                </span>
              </div>
            </div>

            <ChevronRight className="h-4 w-4 text-text-muted opacity-0 group-hover:opacity-100 transition-opacity shrink-0" />
          </div>
        ))}
      </div>
    </div>
  )
}
