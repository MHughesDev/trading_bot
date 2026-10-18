// Version timeline, promotion gate and deployments (N-5.8).
//
// Promotion is a PROPOSAL a human decides (N-4.3). This page therefore has no
// "promote" button. It shows what a decider needs (the gate's checks, the
// usage-rule verdict, the baselines), lets anyone propose, and offers
// approve / reject ONLY on a proposal that is still pending. The server refuses
// a decision from an agent token (403); that refusal is shown, not swallowed.
import { useState } from 'react'
import { isAxiosError } from 'axios'
import { Button } from '@/components/primitives/Button'
import { Badge } from '@/components/primitives/Badge'
import { TextArea } from '@/components/primitives/Field'
import { Panel, PanelBody, PanelHeader } from '@/components/primitives/Panel'
import { EmptyState, ErrorState, PanelLoading } from '@/components/primitives/States'
import { useModelAliases, useModelDeployments, useModelVersions } from '@/hooks/useMlOps'
import {
  useDecideProposal,
  usePromotionEvidence,
  usePromotionProposals,
  useProposePromotion,
} from '@/lib/models/queries'
import { useSelectedVersion } from '@/lib/models/useSelectedVersion'
import { pendingProposals } from '@/lib/models/verdict'
import type { PromotionProposal } from '@/api/modelsStudio'
import { VerdictBody } from './VerdictPanel'
import { VersionPicker } from './VersionPicker'

function proposalTone(s: PromotionProposal['status']) {
  return s === 'approved' ? ('pos' as const) : s === 'rejected' ? ('neg' as const) : ('warn' as const)
}

function errorText(err: unknown): string {
  if (isAxiosError(err) && err.response?.status === 403) {
    return 'Only a human session can decide a promotion. This credential cannot.'
  }
  return 'The server did not accept that. Nothing changed.'
}

function PendingProposal({ modelId, p }: { modelId: string; p: PromotionProposal }) {
  const decide = useDecideProposal(modelId)
  const [note, setNote] = useState('')
  return (
    <li data-testid="pending-proposal" style={{ padding: 'var(--s-3) 0', borderBottom: '1px solid var(--line-hairline)' }}>
      <div style={{ display: 'flex', gap: 'var(--s-2)', alignItems: 'center' }}>
        <strong>v{p.version}</strong>
        <span className="mut">to {p.environment}</span>
        <Badge tone="warn">pending</Badge>
        <span className="mut" style={{ fontSize: 'var(--t-11)' }}>
          proposed by {p.proposed_by}
        </span>
      </div>
      <p style={{ margin: 'var(--s-1) 0' }}>{p.rationale}</p>
      <TextArea
        aria-label="Decision note"
        placeholder="Decision note (required)"
        rows={2}
        value={note}
        onChange={(e) => setNote(e.target.value)}
      />
      <div style={{ display: 'flex', gap: 'var(--s-2)', marginTop: 'var(--s-2)' }}>
        <Button
          size="sm"
          variant="primary"
          disabled={note.trim() === ''}
          loading={decide.isPending}
          onClick={() => decide.mutate({ proposalId: p.proposal_id, decision: 'approved', note: note.trim() })}
        >
          Approve
        </Button>
        <Button
          size="sm"
          variant="danger"
          disabled={note.trim() === ''}
          loading={decide.isPending}
          onClick={() => decide.mutate({ proposalId: p.proposal_id, decision: 'rejected', note: note.trim() })}
        >
          Reject
        </Button>
      </div>
      {decide.isError && (
        <div className="inline-err" role="alert" style={{ marginTop: 'var(--s-2)' }}>
          <span className="msg">{errorText(decide.error)}</span>
        </div>
      )}
    </li>
  )
}

export function PromotionPanel({ modelId }: { modelId: string }) {
  const sel = useSelectedVersion(modelId)
  const versions = useModelVersions(modelId)
  const aliases = useModelAliases(modelId)
  const deployments = useModelDeployments(modelId)
  const evidence = usePromotionEvidence(modelId, sel.version)
  const proposals = usePromotionProposals(modelId)
  const propose = useProposePromotion(modelId)
  const [rationale, setRationale] = useState('')
  const [environment, setEnvironment] = useState('paper')

  const pending = pendingProposals(proposals.data)
  const decided = (proposals.data ?? []).filter((p) => p.status !== 'pending')

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--s-3)' }}>
      <Panel>
        <PanelHeader title="Version timeline" />
        <PanelBody flush={(versions.data ?? []).length > 0}>
          {versions.isLoading ? (
            <PanelLoading lines={3} />
          ) : versions.isError ? (
            <ErrorState message="Versions could not be read." onRetry={() => void versions.refetch()} />
          ) : (versions.data ?? []).length === 0 ? (
            <EmptyState message="No versions yet" detail="A version appears here when a training run finishes." />
          ) : (
            <table className="tbl dense" aria-label="Version timeline">
              <tbody>
                {[...(versions.data ?? [])]
                  .sort((a, b) => b.version - a.version)
                  .map((v) => {
                    const held = Object.entries(aliases.data ?? {})
                      .filter(([, n]) => n === v.version)
                      .map(([a]) => a)
                    return (
                      <tr key={v.version} data-testid="timeline-row">
                        <td className="mono">v{v.version}</td>
                        <td>{v.status}</td>
                        <td>
                          {held.map((a) => (
                            <Badge key={a} tone={a === 'production' ? 'pos' : 'neutral'}>
                              {a}
                            </Badge>
                          ))}
                        </td>
                        <td className="mut">{v.created_at.slice(0, 10)}</td>
                      </tr>
                    )
                  })}
              </tbody>
            </table>
          )}
        </PanelBody>
      </Panel>

      <Panel>
        <PanelHeader
          title="Promotion gate"
          actions={<VersionPicker versions={sel.versions} aliases={sel.aliases} value={sel.version} onChange={sel.select} />}
        />
        <PanelBody>
          {sel.version === null && !sel.query.isLoading ? (
            <EmptyState message="No version to promote" />
          ) : sel.query.isLoading || evidence.isLoading ? (
            <PanelLoading lines={5} />
          ) : evidence.isError || !evidence.data ? (
            <ErrorState
              message="The promotion evidence could not be read."
              detail="A decider needs the gate and the verdict; without them there is nothing to propose against."
              onRetry={() => void evidence.refetch()}
            />
          ) : (
            <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--s-3)' }}>
              <div style={{ display: 'flex', gap: 'var(--s-2)', alignItems: 'center' }}>
                <Badge tone={evidence.data.gate_passed ? 'pos' : 'neg'} data-testid="gate-status">
                  {evidence.data.gate_passed ? 'Gate passed' : 'Gate not passed'}
                </Badge>
                <span className="mut">to {evidence.data.environment}</span>
              </div>
              <table className="tbl dense" aria-label="Gate checks">
                <tbody>
                  {evidence.data.gate_checks.map((c) => (
                    <tr key={c.check} data-testid="gate-check" data-passed={c.passed ? 'true' : 'false'}>
                      <td>{c.check}</td>
                      <td>{c.required ? 'required' : 'advisory'}</td>
                      <td>
                        <Badge tone={c.passed ? 'pos' : c.required ? 'neg' : 'warn'}>{c.passed ? 'pass' : 'fail'}</Badge>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
              <VerdictBody verdict={evidence.data.verdict} />

              <div>
                <span className="lbl">Propose promotion of v{evidence.data.version}</span>
                <TextArea
                  aria-label="Rationale"
                  placeholder="Why this version should be promoted (required)"
                  rows={3}
                  value={rationale}
                  onChange={(e) => setRationale(e.target.value)}
                />
                <div style={{ display: 'flex', gap: 'var(--s-2)', marginTop: 'var(--s-2)', alignItems: 'center' }}>
                  <select
                    aria-label="Environment"
                    value={environment}
                    onChange={(e) => setEnvironment(e.target.value)}
                    style={{ height: 28, background: 'var(--bg-sunken)', color: 'var(--fg-primary)', border: '1px solid var(--line-hairline)', borderRadius: 'var(--r-sm)' }}
                  >
                    <option value="paper">paper</option>
                    <option value="live">live</option>
                  </select>
                  <Button
                    size="sm"
                    variant="primary"
                    disabled={rationale.trim() === ''}
                    loading={propose.isPending}
                    onClick={() =>
                      propose.mutate(
                        { version: evidence.data!.version, environment, rationale: rationale.trim() },
                        { onSuccess: () => setRationale('') },
                      )
                    }
                  >
                    Propose promotion
                  </Button>
                  <span className="mut" style={{ fontSize: 'var(--t-11)' }}>
                    A person decides. Nothing moves until they approve.
                  </span>
                </div>
                {propose.isSuccess && (
                  <p className="mut" role="status">
                    Proposal recorded as pending.
                  </p>
                )}
                {propose.isError && (
                  <div className="inline-err" role="alert">
                    <span className="msg">{errorText(propose.error)}</span>
                  </div>
                )}
              </div>
            </div>
          )}
        </PanelBody>
      </Panel>

      <Panel>
        <PanelHeader title="Proposals" />
        <PanelBody>
          {proposals.isLoading ? (
            <PanelLoading lines={3} />
          ) : proposals.isError ? (
            <ErrorState message="Proposals could not be read." onRetry={() => void proposals.refetch()} />
          ) : (proposals.data ?? []).length === 0 ? (
            <EmptyState message="No proposals" detail="A proposed promotion waits here for a person to decide." />
          ) : (
            <>
              {pending.length > 0 && (
                <ul style={{ listStyle: 'none', margin: 0, padding: 0 }} aria-label="Pending proposals">
                  {pending.map((p) => (
                    <PendingProposal key={p.proposal_id} modelId={modelId} p={p} />
                  ))}
                </ul>
              )}
              {decided.length > 0 && (
                <table className="tbl dense" aria-label="Decided proposals">
                  <tbody>
                    {decided.map((p) => (
                      <tr key={p.proposal_id} data-testid="decided-proposal">
                        <td className="mono">v{p.version}</td>
                        <td>
                          <Badge tone={proposalTone(p.status)}>{p.status}</Badge>
                        </td>
                        <td>{p.decision_note ?? 'no note'}</td>
                        <td className="mut">{p.decided_by ?? ''}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              )}
            </>
          )}
        </PanelBody>
      </Panel>

      <Panel>
        <PanelHeader title="Deployments" />
        <PanelBody flush={(deployments.data ?? []).length > 0}>
          {deployments.isLoading ? (
            <PanelLoading lines={3} />
          ) : deployments.isError ? (
            <ErrorState message="Deployments could not be read." onRetry={() => void deployments.refetch()} />
          ) : (deployments.data ?? []).length === 0 ? (
            <EmptyState message="No deployments" detail="Nothing is deployed: no version is serving." />
          ) : (
            <table className="tbl dense" aria-label="Deployments">
              <tbody>
                {(deployments.data ?? []).map((d) => (
                  <tr key={d.deployment_id}>
                    <td className="mono">v{d.version}</td>
                    <td>{d.environment}</td>
                    <td>{d.alias}</td>
                    <td>{d.status}</td>
                    <td className="n">{d.traffic_pct}%</td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </PanelBody>
      </Panel>
    </div>
  )
}
