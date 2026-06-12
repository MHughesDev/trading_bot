// Automations section — Single-Instrument and Pipeline creation flows
// with a live stage board.
//
// Automations are server-side state: armed automations keep running on the
// platform in BOTH paper and live mode at the same time, regardless of which
// mode this tab's badge is showing.  The list below therefore always shows
// both groups; the mode badge only changes which dashboard data you view.

import { useState } from 'react'
import { useMutation, useQuery } from '@tanstack/react-query'
import { api } from '@/lib/api'
import { useModeStore, type TradingMode } from '@/store/mode'
import { SingleInstrumentFlow } from '@/components/automations/SingleInstrumentFlow'
import { PipelineFlow } from '@/components/automations/PipelineFlow'
import { StageBoard } from '@/components/automations/StageBoard'
import { cn } from '@/lib/utils'
import { Info, Plus, Trash2, Zap } from 'lucide-react'

type CreationMode = 'none' | 'single' | 'pipeline'

interface AutomationSummary {
  id: string
  kind: 'single_instrument' | 'pipeline'
  account_mode: string
  armed: boolean
  active: boolean
  created_at: string
}

export function AutomationsPage() {
  const [creating, setCreating] = useState<CreationMode>('none')
  const { mode } = useModeStore()

  const { data, refetch } = useQuery({
    queryKey: ['automations'],
    queryFn: () =>
      api
        .get<{ automations: AutomationSummary[] }>('/api/automations')
        .then((r) => r.data.automations ?? []),
    refetchInterval: 15000,
  })

  const setArmed = useMutation({
    mutationFn: ({ id, armed }: { id: string; armed: boolean }) =>
      api.post(`/api/automations/${id}/${armed ? 'arm' : 'disarm'}`),
    onSuccess: () => void refetch(),
  })

  const remove = useMutation({
    mutationFn: (id: string) => api.delete(`/api/automations/${id}`),
    onSuccess: () => void refetch(),
  })

  const automations = data ?? []
  const groups: Array<{ label: TradingMode; items: AutomationSummary[] }> = [
    { label: 'PAPER', items: automations.filter((a) => a.account_mode === 'paper') },
    { label: 'LIVE', items: automations.filter((a) => a.account_mode === 'live') },
  ]
  // Show the group matching this tab's mode first.
  if (mode === 'LIVE') groups.reverse()

  return (
    <div className="flex flex-col gap-6 p-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-xl font-semibold text-text">Automations</h1>
          <p className="text-sm text-text-muted mt-1">
            Pipeline and single-instrument automations.
          </p>
        </div>
        <div className="flex gap-2">
          <button
            onClick={() => setCreating('single')}
            className={cn(
              'flex items-center gap-1.5 rounded-lg px-3 py-1.5 text-sm border transition-colors',
              creating === 'single'
                ? 'border-accent bg-accent/10 text-accent'
                : 'border-border text-text-muted hover:text-text hover:bg-border',
            )}
          >
            <Plus className="h-3.5 w-3.5" />
            Single instrument
          </button>
          <button
            onClick={() => setCreating('pipeline')}
            className={cn(
              'flex items-center gap-1.5 rounded-lg px-3 py-1.5 text-sm border transition-colors',
              creating === 'pipeline'
                ? 'border-accent bg-accent/10 text-accent'
                : 'border-border text-text-muted hover:text-text hover:bg-border',
            )}
          >
            <Plus className="h-3.5 w-3.5" />
            Pipeline
          </button>
        </div>
      </div>

      {/* Both modes run concurrently server-side */}
      <div className="flex items-start gap-2 rounded-lg border border-border bg-surface px-4 py-3 text-xs text-text-muted">
        <Info className="h-3.5 w-3.5 shrink-0 mt-0.5 text-blue-400" />
        <span>
          Armed automations run on the server in <b>paper and live at the same
          time</b> — switching this tab's {mode} badge changes what the
          dashboard displays, not what runs.
        </span>
      </div>

      {/* Creation flow */}
      {creating === 'single' && (
        <div className="rounded-xl border border-border bg-surface">
          <div className="flex items-center gap-2 px-4 py-3 border-b border-border">
            <Zap className="h-4 w-4 text-accent" />
            <span className="text-sm font-semibold text-text">New single-instrument automation</span>
          </div>
          <SingleInstrumentFlow
            onArmed={() => {
              setCreating('none')
              void refetch()
            }}
          />
        </div>
      )}
      {creating === 'pipeline' && (
        <div className="rounded-xl border border-border bg-surface">
          <div className="flex items-center gap-2 px-4 py-3 border-b border-border">
            <Zap className="h-4 w-4 text-accent" />
            <span className="text-sm font-semibold text-text">New pipeline automation</span>
          </div>
          <PipelineFlow
            onArmed={() => {
              setCreating('none')
              void refetch()
            }}
          />
        </div>
      )}

      {/* Existing automations, grouped by account mode */}
      {automations.length === 0 && creating === 'none' ? (
        <div className="flex h-48 items-center justify-center rounded-lg border border-dashed border-border text-text-dim text-sm">
          No automations yet — create one above.
        </div>
      ) : (
        groups.map(({ label, items }) => (
          <section key={label}>
            <h2 className="flex items-center gap-2 text-xs font-semibold uppercase tracking-widest text-text-dim mb-3">
              <span
                className={cn(
                  'h-1.5 w-1.5 rounded-full',
                  label === 'PAPER' ? 'bg-amber-400' : 'bg-green-400',
                )}
              />
              {label} automations
              <span className="text-text-dim font-normal normal-case tracking-normal">
                — {items.filter((a) => a.armed).length} running
              </span>
            </h2>
            {items.length === 0 ? (
              <div className="rounded-lg border border-dashed border-border px-4 py-3 text-xs text-text-dim">
                None yet.
              </div>
            ) : (
              <div className="space-y-4">
                {items.map((a) => (
                  <div key={a.id} className="rounded-xl border border-border bg-surface overflow-hidden">
                    <div className="flex items-center gap-3 px-4 py-3 border-b border-border">
                      <span
                        className={cn(
                          'rounded-full px-2 py-0.5 text-xs font-semibold',
                          a.armed
                            ? 'bg-green-500/10 text-green-400'
                            : 'bg-text-dim/10 text-text-dim',
                        )}
                      >
                        {a.armed ? 'Running' : 'Disarmed'}
                      </span>
                      <span className="text-sm text-text-muted capitalize">
                        {a.kind.replace('_', ' ')}
                      </span>
                      <span
                        className={cn(
                          'text-xs font-semibold uppercase',
                          a.account_mode === 'paper' ? 'text-amber-400' : 'text-green-400',
                        )}
                      >
                        {a.account_mode}
                      </span>
                      <span className="ml-auto text-xs text-text-dim">
                        {new Date(a.created_at).toLocaleDateString()}
                      </span>
                      <button
                        onClick={() => setArmed.mutate({ id: a.id, armed: !a.armed })}
                        disabled={setArmed.isPending}
                        className={cn(
                          'rounded-lg px-2.5 py-1 text-xs border transition-colors disabled:opacity-40',
                          a.armed
                            ? 'border-border text-text-muted hover:text-red-400 hover:bg-border'
                            : 'border-accent/40 text-accent hover:bg-accent/10',
                        )}
                      >
                        {a.armed ? 'Disarm' : 'Arm'}
                      </button>
                      <button
                        onClick={() => remove.mutate(a.id)}
                        disabled={remove.isPending}
                        className="rounded-lg p-1.5 text-text-dim hover:text-red-400 hover:bg-border border border-border transition-colors disabled:opacity-40"
                        aria-label="Delete automation"
                      >
                        <Trash2 className="h-3.5 w-3.5" />
                      </button>
                    </div>

                    {a.kind === 'pipeline' && (
                      <div className="px-4 py-3">
                        <StageBoard automationId={a.id} />
                      </div>
                    )}
                  </div>
                ))}
              </div>
            )}
          </section>
        ))
      )}
    </div>
  )
}
