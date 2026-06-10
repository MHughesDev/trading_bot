// Stage board for a pipeline automation — shows per-stage membership counts
// and enter/exit deltas.  Data refreshes every 5 seconds.

import { useQuery } from '@tanstack/react-query'
import { api } from '@/lib/api'
import { StageColumn, type StageData } from './StageColumn'
import { ArrowRight } from 'lucide-react'

interface StageBoardProps {
  automationId: string
}

export function StageBoard({ automationId }: StageBoardProps) {
  const { data } = useQuery({
    queryKey: ['automation-stages', automationId],
    queryFn: () =>
      api
        .get<{ stages: StageData[] }>(`/api/automations/${automationId}/stages`)
        .then((r) => r.data),
    refetchInterval: 5000,
  })

  const stages = data?.stages ?? []

  if (stages.length === 0) {
    return (
      <div className="flex h-32 items-center justify-center text-text-dim text-sm">
        No stage data available yet
      </div>
    )
  }

  return (
    <div className="flex items-start gap-2 overflow-x-auto py-2 px-1">
      {stages.map((stage, i) => (
        <div key={stage.stage_id} className="flex items-center gap-2">
          <StageColumn stage={stage} isFinal={i === stages.length - 1} />
          {i < stages.length - 1 && (
            <ArrowRight className="h-4 w-4 text-text-dim shrink-0" />
          )}
        </div>
      ))}
    </div>
  )
}
