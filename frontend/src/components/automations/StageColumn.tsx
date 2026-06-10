// A single stage column in the pipeline stage board.
// Shows: stage name, membership count, enter/exit deltas.

import { cn } from '@/lib/utils'
import { ArrowUpRight, ArrowDownRight, Users } from 'lucide-react'

export interface StageData {
  stage_id: string
  label: string
  member_count: number
  entered_count: number
  exited_count: number
}

interface StageColumnProps {
  stage: StageData
  isFinal?: boolean
}

export function StageColumn({ stage, isFinal }: StageColumnProps) {
  return (
    <div
      className={cn(
        'flex flex-col gap-3 shrink-0 w-44 rounded-xl border px-4 py-4',
        isFinal
          ? 'border-green-500/30 bg-green-500/5'
          : 'border-border bg-surface-2',
      )}
    >
      {/* Stage label */}
      <div>
        <div className="text-xs font-semibold text-text-muted uppercase tracking-wider truncate">
          {isFinal ? '→ Execute' : stage.label}
        </div>
        {!isFinal && (
          <div className="text-xs text-text-dim mt-0.5 truncate font-mono">
            {stage.stage_id}
          </div>
        )}
      </div>

      {/* Member count */}
      <div className="flex items-center gap-1.5">
        <Users className="h-3.5 w-3.5 text-text-dim" />
        <span className="text-lg font-mono font-bold text-text">
          {stage.member_count}
        </span>
        <span className="text-xs text-text-dim">members</span>
      </div>

      {/* Enter/exit deltas */}
      <div className="flex items-center gap-3 text-xs font-mono">
        <span className="flex items-center gap-0.5 text-green-400">
          <ArrowUpRight className="h-3 w-3" />
          {stage.entered_count}
        </span>
        <span className="flex items-center gap-0.5 text-red-400">
          <ArrowDownRight className="h-3 w-3" />
          {stage.exited_count}
        </span>
      </div>
    </div>
  )
}
