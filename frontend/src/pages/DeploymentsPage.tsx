// Deployments: what is running, whether it was allowed to, and how to stop it
// (Set T T-1.3, with T-1.1's decision and T-2.1's ceilings).
//
// Four questions, in the order a person arrives with them:
//
//   1. *What is running, and on whose authority?* One row per deployment: its
//      gate verdict, the legacy marker where it has no proposal behind it, the
//      ramp size, the review-period countdown and its divergence state.
//   2. *Stop it.* One deployment, or all of them. Stop-all is the kill switch's
//      own action (the server trips it through the monitor's store, then
//      withdraws every deployment) and says so before it is confirmed.
//   3. *What is waiting for me?* Deployment proposals. A person decides them; an
//      approved one is deployed from here and the request carries its
//      `proposal_id`, which is the door the gate's trigger checks.
//   4. *What bounds the fleet?* The declared ceilings. An undeclared ceiling
//      refuses every deployment, and the page says that rather than hiding it.
//
// Every read goes through `ReadGate`: a list that did not load is "unknown",
// which is not the sentence "nothing is deployed". Built on the Meridian
// primitives only.
import { useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Badge } from '@/components/primitives/Badge'
import { Button } from '@/components/primitives/Button'
import { Input } from '@/components/primitives/Field'
import { Modal } from '@/components/primitives/Modal'
import { Panel, PanelBody, PanelHeader } from '@/components/primitives/Panel'
import { EmptyState } from '@/components/primitives/States'
import { ReadGate } from '@/components/ops/ReadGate'
import { refusalWords } from '@/api/ops'
import {
  deploymentsApi,
  type DeploymentProposal,
  type DeploymentRow,
} from '@/api/deployments'
import { dateTime } from '@/lib/format'
import {
  divergenceWords,
  periodWords,
  proposalAction,
  shortRevision,
  sizeWords,
  verdictWords,
} from '@/lib/ops/deployments'

const OVERVIEW = ['deployments', 'overview'] as const
const PROPOSALS = ['deployments', 'proposals'] as const
const CEILINGS = ['deployments', 'ceilings'] as const

function RevokeDialog({ row, onClose }: { row: DeploymentRow | null; onClose: () => void }) {
  const qc = useQueryClient()
  const revoke = useMutation({
    mutationFn: async () => {
      if (!row) throw new Error('no deployment selected')
      return (await deploymentsApi.revoke(row.deployment_id)).data
    },
    onSuccess: () => {
      // Re-read: the server decides what was withdrawn.
      void qc.invalidateQueries({ queryKey: OVERVIEW })
      onClose()
    },
  })
  return (
    <Modal
      open={row !== null}
      onClose={onClose}
      dismissible={false}
      title="Stop this deployment"
      description={row ? `${row.strategy_name} · ${row.instrument_id}` : undefined}
      footer={
        <>
          <span className="spacer" />
          <Button onClick={onClose}>Cancel</Button>
          <Button variant="danger" loading={revoke.isPending} onClick={() => revoke.mutate()}>
            Stop it now
          </Button>
        </>
      }
    >
      <p className="sub">
        Withdraws the deployment and has the order gate refuse its strategy scope at once, so it
        stops submitting now rather than at its next tick. The record is closed with your name.
      </p>
      {revoke.isError && (
        <p className="sub" role="alert" data-testid="revoke-error">
          Refused: {refusalWords(revoke.error)}
        </p>
      )}
    </Modal>
  )
}

function StopAllDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  const qc = useQueryClient()
  const [reason, setReason] = useState('')
  const stop = useMutation({
    mutationFn: async () => (await deploymentsApi.stopAll(reason.trim())).data,
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: OVERVIEW })
      setReason('')
      onClose()
    },
  })
  return (
    <Modal
      open={open}
      onClose={onClose}
      dismissible={false}
      title="Stop every deployment"
      description="This trips the kill switch"
      footer={
        <>
          <span className="spacer" />
          <Button onClick={onClose}>Cancel</Button>
          <Button
            variant="danger"
            disabled={reason.trim() === ''}
            loading={stop.isPending}
            onClick={() => stop.mutate()}
          >
            Trip the kill switch and stop all
          </Button>
        </>
      }
    >
      <p className="sub">
        This is the same action as the monitor&apos;s kill switch: the global scope is halted, then
        every deployment is withdrawn. Nothing resumes by itself; lifting the halt is a separate,
        deliberate step on the monitor.
      </p>
      <Input
        label="Why (recorded with your name)"
        value={reason}
        onChange={(e) => setReason(e.target.value)}
        autoComplete="off"
        placeholder="One sentence a later reader can act on"
      />
      {stop.isError && (
        <p className="sub" role="alert" data-testid="stop-all-error">
          Refused: {refusalWords(stop.error)}
        </p>
      )}
    </Modal>
  )
}

function DeploymentsPanel({ onRevoke }: { onRevoke: (r: DeploymentRow) => void }) {
  const q = useQuery({
    queryKey: OVERVIEW,
    queryFn: () => deploymentsApi.overview(),
    refetchInterval: 30_000,
  })
  return (
    <Panel>
      <PanelHeader
        title="Running and recent deployments"
        badges={
          q.data?.state === 'ok' ? (
            <Badge tone="neutral" data-testid="running-count">
              {q.data.data.filter((d) => d.running).length} running
            </Badge>
          ) : undefined
        }
      />
      <PanelBody>
        <ReadGate query={q} route="/api/deployment-overview" what="the deployments">
          {(rows) =>
            rows.length === 0 ? (
              <EmptyState
                message="Nothing is deployed"
                detail="The route answered with an empty list: no deployment record exists for this account."
              />
            ) : (
              <table className="tbl" data-testid="deployments-table">
                <thead>
                  <tr>
                    <th style={{ textAlign: 'left' }}>strategy</th>
                    <th>state</th>
                    <th>authority</th>
                    <th>size</th>
                    <th>review period</th>
                    <th>divergence</th>
                    <th />
                  </tr>
                </thead>
                <tbody>
                  {rows.map((d) => {
                    const verdict = verdictWords(d)
                    const size = sizeWords(d.size_halvings)
                    const div = divergenceWords(d)
                    return (
                      <tr key={d.deployment_id} data-testid={`deployment-${d.deployment_id}`}>
                        <td style={{ textAlign: 'left' }}>
                          <strong>{d.strategy_name}</strong>{' '}
                          <span className="sub">
                            {shortRevision(d.revision)} · {d.instrument_id} · {d.environment}/{d.venue}
                          </span>
                          <br />
                          <span className="sub">
                            ${d.starting_capital} from {dateTime(d.started_at)}
                          </span>
                        </td>
                        <td>
                          {d.running ? (
                            <Badge tone="pos">running</Badge>
                          ) : (
                            <Badge tone="neutral" title={d.stop_reason ?? undefined}>
                              stopped
                            </Badge>
                          )}
                          {!d.running && d.stop_reason && <div className="sub">{d.stop_reason}</div>}
                        </td>
                        <td>
                          <Badge tone={verdict.tone} data-testid="verdict">
                            {verdict.headline}
                          </Badge>
                          {verdict.failed.length > 0 && (
                            <div className="sub" data-testid="failed-gates">
                              not passed: gate {verdict.failed.join(', ')}
                            </div>
                          )}
                          {d.profile_id && <div className="sub">profile {d.profile_id}</div>}
                          {d.legacy_ungated && (
                            <Badge
                              tone="warn"
                              data-testid="legacy-ungated"
                              title="Running without a gate verdict behind it. A fact shown beside its numbers, never a permission."
                            >
                              legacy, ungated
                            </Badge>
                          )}
                        </td>
                        <td>
                          <Badge tone={size.tone}>{size.word}</Badge>
                        </td>
                        <td>{periodWords(d)}</td>
                        <td>
                          <Badge tone={div.tone} title={div.detail} data-testid="divergence">
                            {div.word}
                          </Badge>
                          <div className="sub">{div.detail}</div>
                        </td>
                        <td>
                          {d.running && (
                            <Button size="xs" variant="danger" onClick={() => onRevoke(d)}>
                              Stop…
                            </Button>
                          )}
                        </td>
                      </tr>
                    )
                  })}
                </tbody>
              </table>
            )
          }
        </ReadGate>
      </PanelBody>
    </Panel>
  )
}

function DecideDialog({
  proposal,
  onClose,
}: {
  proposal: DeploymentProposal | null
  onClose: () => void
}) {
  const qc = useQueryClient()
  const [note, setNote] = useState('')
  const decide = useMutation({
    mutationFn: async (decision: 'approved' | 'rejected') => {
      if (!proposal) throw new Error('no proposal selected')
      return (await deploymentsApi.decide(proposal.proposal_id, decision, note.trim())).data
    },
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: PROPOSALS })
      setNote('')
      onClose()
    },
  })
  return (
    <Modal
      open={proposal !== null}
      onClose={onClose}
      dismissible={false}
      title="Decide a deployment proposal"
      description={proposal ? `${proposal.strategy_name} · ${proposal.instrument_id}` : undefined}
      footer={
        <>
          <span className="spacer" />
          <Button onClick={onClose}>Cancel</Button>
          <Button
            variant="danger"
            loading={decide.isPending}
            onClick={() => decide.mutate('rejected')}
          >
            Reject
          </Button>
          <Button
            variant="primary"
            loading={decide.isPending}
            onClick={() => decide.mutate('approved')}
          >
            Approve
          </Button>
        </>
      }
    >
      {proposal && (
        <>
          <p className="sub">
            Revision {shortRevision(proposal.revision)}, profile {proposal.profile_id},{' '}
            {proposal.gates_passed} gates passed, ${proposal.starting_capital} for{' '}
            {proposal.review_period_days} days, proposed by {proposal.proposed_by_kind}. Approving
            records who, when and the evidence hash; it does not deploy by itself.
          </p>
          <Input
            label="Note (recorded with your name)"
            value={note}
            onChange={(e) => setNote(e.target.value)}
            autoComplete="off"
          />
          {decide.isError && (
            <p className="sub" role="alert" data-testid="decide-error">
              Refused: {refusalWords(decide.error)}
            </p>
          )}
        </>
      )}
    </Modal>
  )
}

function ProposalsPanel() {
  const qc = useQueryClient()
  const [deciding, setDeciding] = useState<DeploymentProposal | null>(null)
  const q = useQuery({
    queryKey: PROPOSALS,
    queryFn: () => deploymentsApi.proposals(),
    refetchInterval: 30_000,
  })
  const deploy = useMutation({
    mutationFn: async (p: DeploymentProposal) => (await deploymentsApi.deploy(p)).data,
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: PROPOSALS })
      void qc.invalidateQueries({ queryKey: OVERVIEW })
    },
  })
  return (
    <Panel>
      <PanelHeader
        title="Deployment proposals"
        badges={
          q.data?.state === 'ok' ? (
            <Badge
              tone={q.data.data.some((p) => p.decision === 'pending') ? 'warn' : 'neutral'}
              data-testid="pending-count"
            >
              {q.data.data.filter((p) => p.decision === 'pending').length} waiting for you
            </Badge>
          ) : undefined
        }
      />
      <PanelBody>
        <ReadGate query={q} route="/api/deployment-proposals" what="the proposals">
          {(rows) =>
            rows.length === 0 ? (
              <EmptyState
                message="No deployment has been proposed"
                detail="A research run proposes; a person decides. Nothing deploys without an approved proposal behind it."
              />
            ) : (
              <table className="tbl" data-testid="proposals-table">
                <thead>
                  <tr>
                    <th style={{ textAlign: 'left' }}>strategy</th>
                    <th>profile</th>
                    <th>capital</th>
                    <th>decision</th>
                    <th />
                  </tr>
                </thead>
                <tbody>
                  {rows.map((p) => {
                    const action = proposalAction(p)
                    return (
                      <tr key={p.proposal_id} data-testid={`proposal-${p.proposal_id}`}>
                        <td style={{ textAlign: 'left' }}>
                          <strong>{p.strategy_name}</strong>{' '}
                          <span className="sub">
                            {shortRevision(p.revision)} · {p.instrument_id}
                          </span>
                          <br />
                          <span className="sub">
                            by {p.proposed_by_kind}, {dateTime(p.created_at)}
                          </span>
                        </td>
                        <td>
                          {p.profile_id} · {p.gates_passed} gates passed
                        </td>
                        <td>
                          ${p.starting_capital} / {p.review_period_days} d
                        </td>
                        <td>
                          <Badge
                            tone={
                              p.decision === 'approved' ? 'pos' : p.decision === 'rejected' ? 'neg' : 'warn'
                            }
                          >
                            {p.decision}
                          </Badge>
                          {p.decided_by && <div className="sub">{p.decided_by}</div>}
                          {p.deployment_id && <div className="sub">deployed</div>}
                        </td>
                        <td>
                          {action === 'decide' && (
                            <Button size="xs" onClick={() => setDeciding(p)}>
                              Decide…
                            </Button>
                          )}
                          {action === 'deploy' && (
                            <Button
                              size="xs"
                              variant="primary"
                              loading={deploy.isPending && deploy.variables?.proposal_id === p.proposal_id}
                              onClick={() => deploy.mutate(p)}
                            >
                              Deploy under this approval
                            </Button>
                          )}
                        </td>
                      </tr>
                    )
                  })}
                </tbody>
              </table>
            )
          }
        </ReadGate>
        {deploy.isError && (
          <p className="sub" role="alert" data-testid="deploy-error">
            Refused: {refusalWords(deploy.error)}
          </p>
        )}
      </PanelBody>
      <DecideDialog key={deciding?.proposal_id ?? 'none'} proposal={deciding} onClose={() => setDeciding(null)} />
    </Panel>
  )
}

function CeilingsPanel() {
  const q = useQuery({ queryKey: CEILINGS, queryFn: () => deploymentsApi.ceilings() })
  return (
    <Panel>
      <PanelHeader title="Fleet ceilings" />
      <PanelBody>
        <ReadGate query={q} route="/api/deployment-ceilings" what="the ceilings">
          {(c) =>
            c.declared.length === 0 ? (
              <EmptyState
                message="No ceiling is declared"
                detail={
                  c.undeclared_refuses_every_deployment
                    ? 'An undeclared ceiling refuses every deployment, so nothing can be deployed until a person declares them.'
                    : 'No ceiling is declared.'
                }
              />
            ) : (
              <table className="tbl" data-testid="ceilings-table">
                <thead>
                  <tr>
                    <th style={{ textAlign: 'left' }}>ceiling</th>
                    <th>value</th>
                    <th>declared by</th>
                    <th>declared</th>
                  </tr>
                </thead>
                <tbody>
                  {c.declared.map((d) => (
                    <tr key={d.name}>
                      <td style={{ textAlign: 'left' }}>{d.name}</td>
                      <td>{d.value}</td>
                      <td>{d.declared_by}</td>
                      <td>{dateTime(d.declared_at)}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            )
          }
        </ReadGate>
      </PanelBody>
    </Panel>
  )
}

export function DeploymentsPage() {
  const [revoking, setRevoking] = useState<DeploymentRow | null>(null)
  const [stopAll, setStopAll] = useState(false)
  return (
    <div className="page" style={{ display: 'flex', flexDirection: 'column', gap: 'var(--s-4)' }}>
      <div className="desk-hd">
        <span className="title">Deployments</span>
        <span className="sub">what is running, on whose authority, and how to stop it</span>
        <span className="spacer" />
        <Button variant="danger" onClick={() => setStopAll(true)}>
          Stop everything…
        </Button>
      </div>
      <DeploymentsPanel onRevoke={setRevoking} />
      <ProposalsPanel />
      <CeilingsPanel />
      <RevokeDialog key={revoking?.deployment_id ?? 'none'} row={revoking} onClose={() => setRevoking(null)} />
      <StopAllDialog open={stopAll} onClose={() => setStopAll(false)} />
    </div>
  )
}
