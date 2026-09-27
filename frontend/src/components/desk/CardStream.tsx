// The card stream (AGENT-012 §3.2; task S-6.3).
//
// Not a transcript. Cards are grouped by step under a step header, and the raw
// events — exposure, compaction rungs, validation, usage — are folded behind a
// disclosure that is off by default and toggled with `d`.
//
// A 200-step run has to render without jank, and the honest way to get that is
// not to mount 200 steps. Only the most recent `WINDOW` groups are mounted; the
// rest sit behind one control that says exactly how many there are. A person
// scrolling a long run wants the end of it.
import { useEffect, useMemo, useState } from 'react'
import { Button } from '@/components/primitives/Button'
import { EmptyState } from '@/components/primitives/States'
import type { DeskCard } from '@/api/desk'
import { groupByStep } from '@/api/desk'
import { Card, type CardActions } from './Cards'
import { StepHeader, type IdResolver } from './Shared'

const WINDOW = 25

function storageKey(conversationId: string) {
  return `desk:details:${conversationId}`
}

/** Remembered per conversation, as S-6.3 requires. A private-mode browser that
 *  refuses storage must not take the stream down with it. */
function loadOpen(conversationId: string): Set<number> {
  try {
    const raw = localStorage.getItem(storageKey(conversationId))
    if (!raw) return new Set()
    return new Set(JSON.parse(raw) as number[])
  } catch {
    return new Set()
  }
}

export function CardStream({
  conversationId,
  cards,
  resolve,
  actions,
  detailsAll,
}: {
  conversationId: string
  cards: DeskCard[]
  resolve?: IdResolver
  actions?: CardActions
  /** The `d` key's global toggle; a step's own disclosure still overrides it. */
  detailsAll: boolean
}) {
  const groups = useMemo(() => groupByStep(cards), [cards])
  const [open, setOpen] = useState<Set<number>>(() => loadOpen(conversationId))
  const [showAll, setShowAll] = useState(false)

  // There is no effect resetting `open` and `showAll` when the conversation
  // changes, because the caller passes `key={conversationId}`: a different
  // conversation is a different stream with a different remembered disclosure
  // set, and remounting is how React says that. The effect that used to do it
  // re-read `localStorage` a render late, so the first render of a new
  // conversation showed the previous one's open steps.
  useEffect(() => {
    try {
      localStorage.setItem(storageKey(conversationId), JSON.stringify([...open]))
    } catch {
      // A browser that refuses storage loses the preference, not the stream.
    }
  }, [conversationId, open])

  if (groups.length === 0) {
    return (
      <EmptyState
        message="Nothing has happened in this conversation yet"
        detail="Ask a question, or describe what to research. Chat spends no trials; Research seals a pre-registration before it spends one."
      />
    )
  }

  const hidden = showAll ? 0 : Math.max(0, groups.length - WINDOW)
  const shown = hidden > 0 ? groups.slice(hidden) : groups

  return (
    <div>
      {hidden > 0 && (
        <Button size="xs" variant="ghost" onClick={() => setShowAll(true)}>
          Show {hidden} earlier {hidden === 1 ? 'step' : 'steps'}
        </Button>
      )}
      {shown.map((g) => {
        const isOpen = detailsAll || open.has(g.step)
        return (
          <section className="desk-step" key={g.step} aria-label={`step ${g.step}`}>
            <StepHeader
              step={g.step}
              label={g.label}
              tools={g.tools}
              seconds={g.seconds}
              open={isOpen}
              rawCount={g.raw.length}
              onToggle={() =>
                setOpen((prev) => {
                  const next = new Set(prev)
                  if (next.has(g.step)) next.delete(g.step)
                  else next.add(g.step)
                  return next
                })
              }
            />
            {g.cards.map((c) => (
              <Card key={c.seq} card={c} resolve={resolve} actions={actions} />
            ))}
            {isOpen && g.raw.map((c) => <Card key={c.seq} card={c} resolve={resolve} actions={actions} />)}
          </section>
        )
      })}
    </div>
  )
}
