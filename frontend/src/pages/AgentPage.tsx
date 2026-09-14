// The agent: a chat, and a list of chats.
//
// What used to be here was a form — provider, model, goal, instrument, timeframe,
// max iterations, time budget, Start run. Every field except the goal was a decision
// the user had to make before any work started, which is exactly when nobody knows
// the answer. The agent picks the instrument now, and how many backtests it needs,
// and when it is finished.
//
// Runs are detached: closing this page does not stop anything, and coming back shows
// where the agent got to.

import { useEffect, useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { Bot } from 'lucide-react'
import { ConversationSidebar } from '@/components/agent/ConversationSidebar'
import { ChatView } from '@/components/agent/ChatView'
import { conversationsApi } from '@/api/agent'

const LAST_KEY = 'agent_last_conversation'

function remembered(): string | null {
  try {
    return localStorage.getItem(LAST_KEY)
  } catch {
    // A private window is not a reason to fail.
    return null
  }
}

export function AgentPage() {
  // Remembered across reloads so a running agent is what you land on, rather than an
  // empty pane you have to go and find it from.
  const [picked, setPicked] = useState<string | null>(remembered)

  const { data: conversations } = useQuery({
    queryKey: ['conversations'],
    queryFn: () => conversationsApi.list().then((r) => r.data.conversations),
    refetchInterval: 5000,
  })

  // Derived during render rather than corrected in an effect. The remembered id can
  // be stale — archived in another tab, or belonging to a different account — and
  // rendering an empty pane against a dead id for one frame before fixing it is a
  // visible flicker for no reason.
  const exists = conversations?.some((c) => c.conversation_id === picked) ?? false
  const selected = exists ? picked : (conversations?.[0]?.conversation_id ?? null)

  useEffect(() => {
    if (!selected) return
    try {
      localStorage.setItem(LAST_KEY, selected)
    } catch {
      /* see `remembered` */
    }
  }, [selected])

  return (
    <div className="flex h-full min-h-0 gap-3 p-3">
      <ConversationSidebar selected={selected} onSelect={setPicked} />

      <div className="panel min-w-0 flex-1 p-4">
        {selected ? (
          <ChatView key={selected} conversationId={selected} />
        ) : (
          <div className="flex h-full flex-col items-center justify-center gap-3 text-center">
            <Bot className="h-10 w-10 text-text-dim" />
            <div className="max-w-md">
              <p className="text-sm font-medium text-text">Research agent</p>
              <p className="mt-1 text-sm text-text-muted">
                Start a new chat and describe what you want investigated. The agent
                designs the strategies, runs the backtests and reports what it found —
                and keeps working if you close the tab.
              </p>
            </div>
          </div>
        )}
      </div>
    </div>
  )
}
