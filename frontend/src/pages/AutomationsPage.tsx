// Automations section — Single-Instrument and Pipeline creation flows
// with a live stage board.

import { useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { api } from '@/lib/api'
import { SingleInstrumentFlow } from '@/components/automations/SingleInstrumentFlow'
import { PipelineFlow } from '@/components/automations/PipelineFlow'
import { StageBoard } from '@/components/automations/StageBoard'
import { cn } from '@/lib/utils'
import { Plus, Zap } from 'lucide-react'

type CreationMode = 'none' | 'single' | 'pipeline'

interface AutomationSummary {
  id: string
  kind: 'single_instrument' | 'pipeline'
  account_mode: string
  armed: boolean
  created_at: string
}

export function AutomationsPage() {
  const [creating, setCreating] = useState<CreationMode>('none')

  const { data, refetch } = useQuery({
    queryKey: ['automations'],
    queryFn: () =>
      api
        .get<{ automations: AutomationSummary[] }>('/api/automations')
        .then((r) => r.data.automations ?? []),
  })

  const automations = data ?? []

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

      {/* Existing automations */}
      {automations.length === 0 && creating === 'none' ? (
        <div className="flex h-48 items-center justify-center rounded-lg border border-dashed border-border text-text-dim text-sm">
          No automations yet — create one above.
        </div>
      ) : (
        <div className="space-y-4">
          {automations.map((a) => (
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
                  {a.armed ? 'Armed' : 'Disarmed'}
                </span>
                <span className="text-sm text-text-muted capitalize">
                  {a.kind.replace('_', ' ')}
                </span>
                <span className="text-xs text-text-dim uppercase">{a.account_mode}</span>
                <span className="ml-auto text-xs text-text-dim">
                  {new Date(a.created_at).toLocaleDateString()}
                </span>
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
    </div>
  )
}
