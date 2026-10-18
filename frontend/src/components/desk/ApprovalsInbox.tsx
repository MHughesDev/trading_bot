// The approvals inbox (COMP-006 §4, UI-05; AGENT-012 §1, §3.2; task S-6.15).
//
// Moved here from `components/workspace/` and rebuilt on the primitives. S-6.15
// says "reuse `ApprovalsInbox`", and its *logic* is reused verbatim — the eight
// kinds, the option-id normalisation, the timeout default, the evidence handles.
// What is not reused is its markup, which was written in the back-compat
// Tailwind aliases this phase exists to delete. Keeping the file where it was
// would have left the desk's own inbox as an alias consumer, which is precisely
// the "rename, not a rebuild" failure S-6.1 warns about. Recorded as a
// deliberate deviation rather than done quietly.
//
// Three things every card shows, because an approval without them is a button:
// the **evidence** the decision rests on, the **default** that happens on
// timeout, and the **time left**. A reviewer who cannot see what happens if they
// do nothing is not making a decision, they are clearing a notification.
import { useState } from 'react'
import { invalidateApprovals } from '@/lib/desk/approvals'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { Badge } from '@/components/primitives/Badge'
import { Button } from '@/components/primitives/Button'
import { EmptyState } from '@/components/primitives/States'
import { researchApi, type Approval } from '@/api/research'
import { EnvelopeBanner, EvidenceChip, type IdResolver } from './Shared'
import { optionIds, timeLeft, evidenceOf } from './ApprovalsInbox.helpers'
import { isIrreversibleEnvelope } from '@/components/desk/Cards.helpers'

const KIND_LABELS: Record<string, string> = {
  ask_user: 'Question',
  plan: 'Plan approval',
  budget: 'Budget increase',
  qc_waiver: 'Data quality waiver',
  model_promotion: 'Model promotion',
  skill_promotion: 'Skill promotion',
  paper_deployment: 'Paper deployment',
  tool_call: 'Tool call needs approval',
}

export function ApprovalCard({
  approval,
  onAnswered,
  resolve,
}: {
  approval: Approval
  onAnswered?: () => void
  resolve?: IdResolver
}) {
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [armed, setArmed] = useState<string | null>(null)
  // Why this was answered the way it was.
  //
  // `researchApi.answerApproval` has taken a third `note` argument since it was
  // written and this card called it with two, so the inbox — the surface whose
  // entire job is deciding these — had no field for the reason and could not
  // have sent one if it had. On the other side `spend_holdout_look` refuses an
  // approval without twenty characters of reason, into an append-only log; so
  // for those kinds this was not a missing nicety, it was a card that could
  // only ever be refused.
  //
  // Offered on the two-step path only, matching `Cards.tsx`: the reversible
  // answers are the ones you take back rather than explain.
  const [note, setNote] = useState('')
  const options = optionIds(approval.options)
  const left = timeLeft(approval.timeout_at)
  const evidence = evidenceOf(approval.payload)
  const envelope = (approval.payload.envelope as string) ?? null
  const twoStep = isIrreversibleEnvelope(envelope)
  const question =
    (approval.payload.question as string) ??
    (approval.payload.summary as string) ??
    (approval.payload.text as string) ??
    (approval.payload.reason as string) ??
    ''
  const tool = approval.payload.tool as string | undefined
  const fix = approval.payload.fix as string | undefined

  const answer = async (option: string) => {
    if (twoStep && armed !== option) {
      setArmed(option)
      return
    }
    setBusy(true)
    setError(null)
    try {
      await researchApi.answerApproval(approval.approval_id, option, note.trim() || undefined)
      onAnswered?.()
    } catch (e) {
      const detail = (e as { response?: { data?: { message?: string } } }).response?.data
      setError(detail?.message ?? 'the answer could not be recorded')
    } finally {
      setBusy(false)
      setArmed(null)
    }
  }

  return (
    <article className="desk-card attn" tabIndex={0} aria-label={KIND_LABELS[approval.kind] ?? approval.kind}>
      <div className="desk-card-hd">
        <span className="desk-card-kind">{KIND_LABELS[approval.kind] ?? approval.kind}</span>
        {tool && <Badge tone="outline">{tool}</Badge>}
        {left && <span className="sub" style={{ marginLeft: 'auto' }}>{left}</span>}
      </div>

      {envelope && (
        <EnvelopeBanner
          envelope={envelope}
          changes={(approval.payload.changes as string) ?? 'see the request above'}
          reversible={!twoStep}
        />
      )}

      <p>{question}</p>
      {fix && <p className="sub">Alternative: {fix}</p>}

      {evidence.length > 0 && (
        <div className="desk-actions">
          {evidence.map((e) => (
            <EvidenceChip key={e} id={e} resolve={resolve} />
          ))}
        </div>
      )}

      {approval.default_option && (
        <p className="sub">
          If you do nothing, this defaults to <span className="mono">{approval.default_option}</span>.
        </p>
      )}

      {error && <p className="inline-err">{error}</p>}

      {armed && (
        <input
          className="desk-approve-note"
          data-testid="inbox-note"
          type="text"
          value={note}
          placeholder="why — recorded permanently"
          onChange={(e) => setNote(e.target.value)}
        />
      )}

      <div className="desk-actions">
        {(options.length > 0 ? options : ['approve', 'deny']).map((o) => (
          <Button
            key={o}
            size="xs"
            variant={armed === o ? 'danger' : o === 'approve' ? 'primary' : 'secondary'}
            loading={busy}
            onClick={() => void answer(o)}
          >
            {armed === o ? `Confirm ${o} — ${envelope} cannot be undone` : o}
          </Button>
        ))}
      </div>
    </article>
  )
}

export function ApprovalsInbox({
  projectId,
  onAnswered,
  resolve,
}: {
  projectId?: string
  onAnswered?: () => void
  resolve?: IdResolver
}) {
  const queryClient = useQueryClient()
  const key = ['approvals', projectId ?? 'all']
  const { data: approvals = [], isLoading } = useQuery({
    queryKey: key,
    queryFn: async () => (await researchApi.approvals({ project_id: projectId })).data.approvals,
    refetchInterval: 15_000,
  })
  const refresh = () => {
    // Every list, not only this one: desk home's *Needs you* holds the same
    // approval under its own key.
    invalidateApprovals(queryClient)
    onAnswered?.()
  }

  if (isLoading) return <p className="sub">Loading approvals…</p>
  if (approvals.length === 0) {
    return <EmptyState message="Nothing is waiting on you" detail="Answered approvals leave this list in the same event." />
  }

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--s-2)' }}>
      {approvals.map((a) => (
        <ApprovalCard key={a.approval_id} approval={a} onAnswered={refresh} resolve={resolve} />
      ))}
    </div>
  )
}
