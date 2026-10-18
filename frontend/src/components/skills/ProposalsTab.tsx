// The proposal queue, with verify and admit (RA-4.3).
//
// An agent proposes; the platform admits (ADR-0028). Verify runs what the platform can run
// (lint, procedure, triggers, coexistence, replay) and reports the first step still owed, which
// is usually the Reviewer's verdict: a person or the harness records it elsewhere. Admit is
// offered only for a proposal this page has just verified admissible, and then only through a
// confirmation that names the proposal and says it becomes a *provisional* skill. The server
// holds the real gate (`skills.admit`, never an agent scope); a refusal is shown as it came.
import { useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Badge } from '@/components/primitives/Badge'
import { Button } from '@/components/primitives/Button'
import { ConfirmDialog } from '@/components/primitives/Modal'
import { Panel, PanelBody, PanelHeader } from '@/components/primitives/Panel'
import { EmptyState } from '@/components/primitives/States'
import { ReadGate } from '@/components/ops/ReadGate'
import { refusalWords } from '@/api/ops'
import { skillsApi, type Proposal, type Verification } from '@/api/skills'
import {
  GLOSSARY_KEY,
  PROPOSALS_KEY,
  admitControl,
  openProposals,
  proposalTone,
  similarityWords,
  stepRows,
} from '@/lib/skills/view'
import { dateTime } from '@/lib/format'

function ProposalRow({ p }: { p: Proposal }) {
  const qc = useQueryClient()
  const [verification, setVerification] = useState<Verification | null>(null)
  const [confirming, setConfirming] = useState(false)
  const verify = useMutation({
    mutationFn: () => skillsApi.verify(p.proposal_id),
    onSuccess: (r) => setVerification(r.data),
  })
  const admit = useMutation({
    mutationFn: () => skillsApi.admit(p.proposal_id),
    onSuccess: () => {
      setConfirming(false)
      void qc.invalidateQueries({ queryKey: PROPOSALS_KEY })
      void qc.invalidateQueries({ queryKey: GLOSSARY_KEY })
    },
  })
  const control = admitControl(p.status, verification)

  return (
    <tr data-testid={`proposal-${p.proposal_id}`}>
      <td style={{ textAlign: 'left' }}>
        <div>{p.name}</div>
        <div className="sub">
          {p.reason} · {similarityWords(p.similarity)} · {dateTime(p.created_at)}
        </div>
        {verification && (
          <ul data-testid="verification">
            {stepRows(verification).map((s) => (
              <li key={s.step}>
                {s.step}: {s.state}
              </li>
            ))}
          </ul>
        )}
        {verify.isError && (
          <div role="alert" className="sub">
            Refused: {refusalWords(verify.error)}
          </div>
        )}
        {admit.isError && (
          <div role="alert" className="sub">
            Refused: {refusalWords(admit.error)}
          </div>
        )}
        {!control.enabled && control.why && <div className="sub">Admit unavailable: {control.why}</div>}
      </td>
      <td>
        <Badge tone={proposalTone(p.status)}>{p.status}</Badge>
      </td>
      <td>
        <Button size="xs" variant="ghost" loading={verify.isPending} onClick={() => verify.mutate()}>
          Verify
        </Button>
        <Button size="xs" variant="primary" disabled={!control.enabled} onClick={() => setConfirming(true)}>
          Admit
        </Button>
        <ConfirmDialog
          open={confirming}
          onCancel={() => setConfirming(false)}
          onConfirm={() => admit.mutate()}
          busy={admit.isPending}
          title={`Admit ${p.name}`}
          consequence={
            <>
              <strong>{p.name}</strong> becomes a <strong>provisional</strong> skill in the registry and is offered to
              sessions from the next start. It is admitted on this verification only; it earns <em>admitted</em> by use.
            </>
          }
          confirmLabel="Admit as provisional"
          typeToConfirm="ADMIT"
        />
      </td>
    </tr>
  )
}

export function ProposalsTab() {
  const q = useQuery({ queryKey: PROPOSALS_KEY, queryFn: () => skillsApi.proposals() })
  return (
    <Panel>
      <PanelHeader title="Proposal queue" />
      <PanelBody>
        <ReadGate query={q} route="/api/skills/proposals" what="the proposal queue">
          {(d) => {
            const open = openProposals(d.proposals)
            return open.length === 0 ? (
              <EmptyState
                message="No proposal is waiting"
                detail={`${d.proposals.length} decided proposals are on record; an agent proposes and the platform admits.`}
              />
            ) : (
              <table className="tbl" data-testid="proposals-table">
                <thead>
                  <tr>
                    <th style={{ textAlign: 'left' }}>proposal</th>
                    <th>status</th>
                    <th />
                  </tr>
                </thead>
                <tbody>
                  {open.map((p) => (
                    <ProposalRow key={p.proposal_id} p={p} />
                  ))}
                </tbody>
              </table>
            )
          }}
        </ReadGate>
      </PanelBody>
    </Panel>
  )
}
