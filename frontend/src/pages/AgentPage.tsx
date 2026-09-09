// Agent page — internal LLM agent that designs strategies, runs backtests,
// waits on them server-side, and iterates. Left: config + run list.
// Right: live run timeline.

import { useState } from 'react'
import { Bot } from 'lucide-react'
import { RunConfigPanel } from '@/components/agent/RunConfigPanel'
import { RunList } from '@/components/agent/RunList'
import { RunTimeline } from '@/components/agent/RunTimeline'

export function AgentPage() {
  const [selectedRun, setSelectedRun] = useState<string | null>(null)

  return (
    <div className="flex h-full min-h-0 gap-4 p-4">
      {/* Left: config + runs */}
      <div className="flex w-96 shrink-0 flex-col gap-3 overflow-y-auto pr-1">
        <RunConfigPanel onStarted={setSelectedRun} />
        <div>
          <h2 className="text-xs font-semibold uppercase tracking-wide text-text-dim mb-2 px-1">
            Runs
          </h2>
          <RunList selected={selectedRun} onSelect={setSelectedRun} />
        </div>
      </div>

      {/* Right: timeline */}
      <div className="flex-1 min-w-0 rounded-xl border border-border bg-surface p-4">
        {selectedRun ? (
          <RunTimeline runId={selectedRun} />
        ) : (
          <div className="flex h-full flex-col items-center justify-center gap-3 text-center">
            <Bot className="h-10 w-10 text-text-dim" />
            <div>
              <p className="text-sm font-medium text-text">Agent workspace</p>
              <p className="mt-1 max-w-md text-sm text-text-muted">
                Give the agent a goal — it designs a trading strategy, backtests it
                against real historical data (waiting out long runs on its own), reads
                the results, and iterates. Select a run to watch it work.
              </p>
            </div>
          </div>
        )}
      </div>
    </div>
  )
}
