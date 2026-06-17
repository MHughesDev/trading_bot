// The detail view for one Experiment. The lifecycle state and trial counter are
// rendered in a sticky header that is ALWAYS on screen (J-5.5: you cannot read a
// result without seeing how many trials produced it), with the `unsafe` flag
// surfaced prominently when set. Below it: the study distribution viewers
// (INV-2), the null picker (D-7), the gate-funnel board (D-8), the INV-3
// significance card, the vault panel, and the reconciliation read-out.
import { useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { AlertTriangle, Loader2 } from 'lucide-react'
import {
  experimentsApi,
  type ExperimentView,
  type NullKind,
} from '@/api/experiments'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { toast } from '@/hooks/useToast'
import { SignificanceCard } from './SignificanceCard'
import { DistributionViewer } from './DistributionViewer'
import { GateFunnelBoard } from './GateFunnelBoard'
import { NullPicker } from './NullPicker'
import { VaultPanel } from './VaultPanel'

const STATE_VARIANT: Record<string, 'default' | 'active' | 'inactive' | 'warning'> = {
  candidate: 'default',
  validated: 'active',
  live: 'active',
  decaying: 'warning',
  retired: 'inactive',
}

export function ExperimentDetail({ exp }: { exp: ExperimentView }) {
  const id = exp.id
  const qc = useQueryClient()

  const invalidate = () => {
    qc.invalidateQueries({ queryKey: ['suite', 'experiments'] })
    qc.invalidateQueries({ queryKey: ['suite', 'experiment', id] })
  }

  const studies = useQuery({
    queryKey: ['suite', 'experiment', id, 'studies'],
    queryFn: () => experimentsApi.listStudies(id).then((r) => r.data.studies),
  })
  const nulls = useQuery({
    queryKey: ['suite', 'experiment', id, 'nulls'],
    queryFn: () => experimentsApi.nullPicker(id).then((r) => r.data),
  })
  const funnel = useQuery({
    queryKey: ['suite', 'experiment', id, 'funnel'],
    queryFn: () => experimentsApi.funnel(id).then((r) => r.data),
  })
  const vault = useQuery({
    queryKey: ['suite', 'experiment', id, 'vault'],
    queryFn: () => experimentsApi.vault(id).then((r) => r.data),
  })

  const refetchAll = () => {
    studies.refetch()
    nulls.refetch()
    funnel.refetch()
    vault.refetch()
    invalidate()
  }

  const chooseNull = useMutation({
    mutationFn: ({ kind, reason }: { kind: NullKind; reason?: string }) =>
      experimentsApi.chooseNull(id, kind, reason),
    onSuccess: () => {
      toast({ title: 'Null chosen' })
      nulls.refetch()
    },
    onError: (e: unknown) => toast({ title: errMsg(e, 'Could not choose null'), variant: 'error' }),
  })
  const advance = useMutation({
    mutationFn: () => experimentsApi.advanceFunnel(id),
    onSuccess: () => {
      toast({ title: 'Funnel advanced' })
      refetchAll()
    },
    onError: (e: unknown) => toast({ title: errMsg(e, 'Could not advance funnel'), variant: 'error' }),
  })
  const runVault = useMutation({
    mutationFn: () => experimentsApi.runVault(id),
    onSuccess: () => {
      toast({ title: 'Vault evaluated — experiment validated' })
      refetchAll()
    },
    onError: (e: unknown) => toast({ title: errMsg(e, 'Vault refused'), variant: 'error' }),
  })
  const promote = useMutation({
    mutationFn: () => experimentsApi.promote(id),
    onSuccess: () => {
      toast({ title: 'Promoted to live' })
      invalidate()
    },
    onError: (e: unknown) => toast({ title: errMsg(e, 'Could not promote'), variant: 'error' }),
  })

  return (
    <div className="flex flex-col gap-4">
      {/* Always-visible lifecycle + trial counter header. */}
      <div className="sticky top-0 z-10 flex flex-wrap items-center justify-between gap-3 rounded-lg border border-border bg-surface/95 p-4 backdrop-blur">
        <div className="min-w-0">
          <div className="flex items-center gap-2">
            <span className="truncate text-base font-semibold text-text">{exp.experiment_id}</span>
            <Badge variant={STATE_VARIANT[exp.state] ?? 'default'}>{exp.state}</Badge>
            {exp.unsafe && (
              <Badge variant="destructive" className="gap-1">
                <AlertTriangle className="h-3 w-3" />
                UNSAFE
              </Badge>
            )}
          </div>
          <div className="mt-0.5 truncate text-xs text-text-dim">
            {exp.strategy_family} · {exp.strategy_type} · primary test{' '}
            <span className="font-mono">{exp.primary_test}</span>
          </div>
        </div>
        <div className="flex items-center gap-4">
          <div className="text-right">
            <div className="text-[10px] uppercase tracking-wide text-text-dim">trial counter</div>
            <div className="text-xl font-bold tabular-nums text-text">
              {exp.trial_counter.toLocaleString()}
            </div>
          </div>
          {exp.state === 'validated' && (
            <Button size="sm" onClick={() => promote.mutate()} disabled={promote.isPending}>
              {promote.isPending && <Loader2 className="h-3 w-3 animate-spin" />}
              Promote to live
            </Button>
          )}
        </div>
      </div>

      <div className="grid grid-cols-1 gap-4 lg:grid-cols-2">
        <div className="flex flex-col gap-4">
          {nulls.data && (
            <NullPicker
              picker={nulls.data}
              saving={chooseNull.isPending}
              onChoose={(kind, reason) => chooseNull.mutate({ kind, reason })}
            />
          )}
          {funnel.data && (
            <GateFunnelBoard
              funnel={funnel.data}
              advancing={advance.isPending}
              canAdvance={!!nulls.data?.chosen}
              onAdvance={() => advance.mutate()}
            />
          )}
        </div>
        <div className="flex flex-col gap-4">
          <SignificanceCard significance={funnel.data?.significance ?? null} />
          {vault.data && (
            <VaultPanel vault={vault.data} running={runVault.isPending} onRun={() => runVault.mutate()} />
          )}
          {(exp.state === 'live' || exp.state === 'decaying') && (
            <ReconciliationPanel expId={id} onDone={invalidate} />
          )}
        </div>
      </div>

      {/* Sealed Study distributions (INV-2). */}
      <div>
        <h3 className="mb-2 text-sm font-semibold text-text">Studies</h3>
        {studies.isLoading ? (
          <div className="text-sm text-text-dim">Loading…</div>
        ) : studies.data && studies.data.length > 0 ? (
          <div className="grid grid-cols-1 gap-4 xl:grid-cols-2">
            {studies.data.map((s) => (
              <DistributionViewer key={s.study_id} study={s} />
            ))}
          </div>
        ) : (
          <div className="text-sm text-text-dim">
            No studies yet. Advancing the funnel runs the evidence studies the gates consume.
          </div>
        )}
      </div>
    </div>
  )
}

function ReconciliationPanel({ expId, onDone }: { expId: string; onDone: () => void }) {
  const [raw, setRaw] = useState('-0.5, -0.6, -0.5, -0.7')
  const reconcile = useMutation({
    mutationFn: () => {
      const realized = raw
        .split(',')
        .map((s) => Number(s.trim()))
        .filter((n) => !Number.isNaN(n))
      return experimentsApi.reconcile(expId, realized)
    },
    onSuccess: (r) => {
      toast({
        title: r.data.verdict.drifting ? 'Drift detected — decaying' : 'Within distribution',
      })
      onDone()
    },
    onError: (e: unknown) => toast({ title: errMsg(e, 'Reconcile failed'), variant: 'error' }),
  })
  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-sm">Reconciliation (live vs backtest)</CardTitle>
      </CardHeader>
      <CardContent className="flex flex-col gap-2">
        <p className="text-xs text-text-dim">
          Realized per-period returns. Drift below the planned worst-5% auto-transitions the
          experiment to decaying.
        </p>
        <textarea
          value={raw}
          onChange={(e) => setRaw(e.target.value)}
          rows={2}
          className="rounded-md border border-border-2 bg-surface px-2 py-1 text-sm text-text"
        />
        <div className="flex justify-end">
          <Button size="sm" disabled={reconcile.isPending} onClick={() => reconcile.mutate()}>
            {reconcile.isPending && <Loader2 className="h-3 w-3 animate-spin" />}
            Reconcile
          </Button>
        </div>
        {reconcile.data && (
          <div className="text-xs text-text-dim">
            {reconcile.data.data.verdict.points.length} periods · fraction below worst-5%:{' '}
            {(reconcile.data.data.verdict.fraction_below_worst5 * 100).toFixed(0)}% ·{' '}
            {reconcile.data.data.verdict.drifting ? (
              <span className="text-pnl-down">drifting</span>
            ) : (
              <span className="text-pnl-up">stable</span>
            )}
          </div>
        )}
      </CardContent>
    </Card>
  )
}

function errMsg(e: unknown, fallback: string): string {
  const resp = (e as { response?: { data?: { message?: string } } })?.response
  return resp?.data?.message ?? fallback
}
