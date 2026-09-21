// The proposals queue (AGENT-013 §6.8, §7.5; Set S tasks S-10.9, S-10.15).
//
// An agent creates an inert row and says so out loud; a person decides here, and
// the registry write happens at that moment, as that person.
//
// **Not the approvals inbox.** `ApprovalsInbox` answers a *running* agent's
// `AskHuman` pause: a run is stopped, waiting, and a timeout has a default. A
// proposal is decided later and nothing is waiting on it. Collapsing the two
// would put blocked runs and unhurried decisions in one list, where the timer on
// one half would read as urgency on the other.
//
// Three things every card shows, for the same reason the inbox shows three:
//
//   * the **dossier** the decision rests on — a proposal with none would ask
//     somebody to decide from the agent's own summary of the agent's own work;
//   * the **ring** it reaches, because "writes a strategy" and "trades on paper"
//     are not the same decision;
//   * whether approving it would **do anything**. Two of the three kinds have no
//     approval-side writer yet, and that is a fact the person deciding is owed
//     before they decide rather than after.
import { useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'

import { Badge } from '@/components/primitives/Badge'
import { Button } from '@/components/primitives/Button'
import { EmptyState, ErrorState, PanelLoading } from '@/components/primitives/States'
import { deskApi, type Proposal, type ProposalDecision } from '@/api/desk'

const KIND_LABELS: Record<string, string> = {
  promote_strategy: 'Promote strategy',
  change_limits: 'Change risk limits',
  deploy_paper: 'Deploy to paper',
}

/**
 * What a ring means, in the words of the thing it protects.
 *
 * A number alone makes the two decisions look like degrees of the same thing.
 */
export function ringLabel(ring: number): string {
  if (ring >= 2) return 'ring 2 · reaches paper trading'
  if (ring === 1) return "ring 1 · writes the platform's own artifacts"
  return 'ring 0 · research only'
}

/**
 * The line shown under an approve button.
 *
 * Returns `null` when approving would perform the write, because a card that
 * explained itself in the ordinary case would bury the case that matters.
 */
export function inertNotice(p: Proposal): string | null {
  if (p.actionable) return null
  if (!p.subject_id) {
    return 'This proposal names no target row — it predates the column — so approving it records the decision and changes nothing.'
  }
  return 'The platform has no approval-side writer for this kind yet, so approving it records the decision and changes nothing.'
}

function Card({ p, onDecided }: { p: Proposal; onDecided: (d: ProposalDecision) => void }) {
  const [note, setNote] = useState('')
  const decide = useMutation({
    mutationFn: (decision: 'approved' | 'rejected') => deskApi.decideProposal(p.proposal_id, decision, note),
    onSuccess: (d) => {
      if (d) onDecided(d)
    },
  })
  const inert = inertNotice(p)

  return (
    <div className="desk-card">
      <div className="desk-card-hd">
        <Badge tone={p.ring >= 2 ? 'neg' : 'outline'}>{KIND_LABELS[p.kind] ?? p.kind}</Badge>
        <Badge tone="outline">{ringLabel(p.ring)}</Badge>
        <Badge tone="neutral">{p.envelope}</Badge>
      </div>
      <p>{p.subject}</p>
      {p.rationale && <p className="sub">{p.rationale}</p>}
      <p className="sub">
        {/* The decision rests on this, not on the card. */}
        dossier: {p.dossier_ref}
      </p>
      {inert && <p className="sub">{inert}</p>}
      <label className="sub" htmlFor={`note-${p.proposal_id}`}>
        Why (recorded on the decision)
      </label>
      <input
        id={`note-${p.proposal_id}`}
        value={note}
        onChange={(e) => setNote(e.target.value)}
        placeholder="on what basis"
      />
      <div className="desk-actions">
        <Button
          size="xs"
          variant="primary"
          disabled={decide.isPending}
          onClick={() => decide.mutate('approved')}
        >
          Approve
        </Button>
        {/* A rejection is a decision and is recorded as one. An approval rate
            over a queue where rejections vanish is not a rate. */}
        <Button size="xs" variant="ghost" disabled={decide.isPending} onClick={() => decide.mutate('rejected')}>
          Reject
        </Button>
      </div>
      {decide.isError && <p className="sub">The decision was not recorded. Nothing changed.</p>}
    </div>
  )
}

export function ProposalsQueue() {
  const qc = useQueryClient()
  const [last, setLast] = useState<ProposalDecision | null>(null)

  const data = useQuery({
    queryKey: ['desk', 'proposals'],
    queryFn: () => deskApi.proposals(),
  })

  const pending = data.data?.pending ?? []

  return (
    <div className="desk-scroll">
      {/* Four states, not two. `optional()` only folds 404/405/501 into `null`
          and rethrows everything else, so a 500 or a dropped connection left
          `data.data` `undefined` — and `undefined` matched neither the `null`
          branch nor the `data.data && pending.length === 0` one, so the queue
          rendered **nothing at all**: no rows, no error, no empty state. A
          blank panel on a decisions queue is the reassuring reading of a failure,
          and this queue is where rings 1 and 2 are licensed. */}
      {data.isPending && <PanelLoading lines={2} />}
      {!data.isPending && data.data === undefined && (
        <ErrorState
          message="The proposals queue could not be read"
          detail="This is not the same as nothing waiting to be decided. A proposal into ring 1 or ring 2 stays undecided."
          onRetry={() => void data.refetch()}
        />
      )}
      {data.data === null && (
        <EmptyState
          message="Proposals are unavailable"
          detail="/api/lifecycle/proposals is served, so this is the endpoint failing rather than the endpoint missing — check the API logs."
        />
      )}

      {last && (
        <div className="desk-card">
          <div className="desk-card-hd">
            <Badge tone={last.decision === 'approved' ? 'neutral' : 'outline'}>{last.decision}</Badge>
            {/* Decided and performed are two facts. An approval the platform
                could not act on says so here rather than reading as done. */}
            <Badge tone={last.performed ? 'neutral' : 'neg'}>
              {last.performed ? 'performed' : 'nothing performed'}
            </Badge>
          </div>
          <p className="sub">
            {last.performed
              ? 'The write happened, as you.'
              : `Recorded. ${String((last.effect as { why?: string }).why ?? 'The platform performed nothing.')}`}
          </p>
        </div>
      )}

      {data.data && pending.length === 0 ? (
        <EmptyState message="Nothing to decide" detail="No run has asked to reach ring 1 or ring 2." />
      ) : (
        pending.map((p) => (
          <Card
            key={p.proposal_id}
            p={p}
            onDecided={(d) => {
              setLast(d)
              void qc.invalidateQueries({ queryKey: ['desk', 'proposals'] })
            }}
          />
        ))
      )}
    </div>
  )
}
