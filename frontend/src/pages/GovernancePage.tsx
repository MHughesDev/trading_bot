// Governance: why something is allowed to run, and how to undo it (FB-RX-004).
//
// Read-only, over the evidence the platform already records:
//
//   * Release evidence — the platform's own health signals (SPEC §16.2) and the
//     sixteen-gate stack for one experiment or trial (`/api/platform/gates/{subject}`);
//   * Timeline — what the agent tried and what it chose from, with what was
//     denied first (`/api/platform/timeline`);
//   * Model aliases — which version each alias points at, with the one write on
//     this page: the existing rollback route, behind a confirm step;
//   * Deployments — what is deployed, from the deployment records.
//
// Out of scope and said so: a config-diff audit. There is no server-side config
// history to diff, and the tab says that instead of rendering a diff of nothing.
// There is likewise no alias *history* route; the current alias map is what is
// shown, and a rollback is recorded in the timeline.
import { useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Badge } from '@/components/primitives/Badge'
import { Button } from '@/components/primitives/Button'
import { Input } from '@/components/primitives/Field'
import { ConfirmDialog } from '@/components/primitives/Modal'
import { Panel, PanelBody, PanelHeader } from '@/components/primitives/Panel'
import { Segmented } from '@/components/primitives/Segmented'
import { EmptyState } from '@/components/primitives/States'
import { ReadGate } from '@/components/ops/ReadGate'
import { governanceApi, read, refusalWords, type GovernedModel } from '@/api/ops'
import { platformHealthApi, valueLabel } from '@/api/platformHealth'
import { dateTime } from '@/lib/format'
import { deploymentStanding, gateStackWords, preventedRows, rollbackConsequence } from '@/lib/ops/governance'

type Tab = 'evidence' | 'timeline' | 'aliases' | 'deployments' | 'config'

// ── Release evidence ─────────────────────────────────────────────────────────

function HealthPanel() {
  const q = useQuery({
    queryKey: ['governance', 'health'],
    queryFn: () => read(platformHealthApi.get()),
  })
  return (
    <Panel>
      <PanelHeader title="Platform health signals" />
      <PanelBody>
        <ReadGate query={q} route="/api/platform/health" what="the platform health signals">
          {(h) => (
            <table className="tbl" data-testid="health-signals">
              <thead>
                <tr>
                  <th style={{ textAlign: 'left' }}>signal</th>
                  <th>state</th>
                  <th>value</th>
                  <th style={{ textAlign: 'left' }}>detail</th>
                </tr>
              </thead>
              <tbody>
                {h.signals.map((s) => (
                  <tr key={s.id}>
                    <td style={{ textAlign: 'left' }}>{s.title}</td>
                    <td>
                      <Badge tone={s.state === 'ok' ? 'pos' : s.state === 'alarm' ? 'neg' : 'warn'}>
                        {s.state.replace('_', ' ')}
                      </Badge>
                    </td>
                    <td>{valueLabel(s)}</td>
                    <td style={{ textAlign: 'left' }}>{s.detail}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </ReadGate>
      </PanelBody>
    </Panel>
  )
}

function GatesPanel() {
  const [typed, setTyped] = useState('')
  const [subject, setSubject] = useState<string | null>(null)
  const q = useQuery({
    queryKey: ['governance', 'gates', subject],
    queryFn: () => governanceApi.gates(subject ?? ''),
    enabled: subject !== null,
  })
  return (
    <Panel>
      <PanelHeader title="Gate stack" />
      <PanelBody>
        <form
          onSubmit={(e) => {
            e.preventDefault()
            if (typed.trim()) setSubject(typed.trim())
          }}
          style={{ display: 'flex', gap: 'var(--s-3)', alignItems: 'end' }}
        >
          <Input
            label="Experiment or trial id"
            value={typed}
            onChange={(e) => setTyped(e.target.value)}
            placeholder="The id a run or proposal names"
            autoComplete="off"
          />
          <Button type="submit" variant="primary" disabled={typed.trim() === ''}>
            Show gates
          </Button>
        </form>
        {subject === null ? (
          <EmptyState message="Name a subject" detail="The gate stack is recorded per experiment or trial." />
        ) : (
          <ReadGate query={q} route="/api/platform/gates/{subject}" what="the gate stack">
            {(g) => {
              const w = gateStackWords(g)
              return (
                <>
                  <p data-testid="gate-summary">
                    <Badge tone={w.tone}>{w.word}</Badge> <span className="sub">{w.sentence}</span>
                  </p>
                  {g.verdicts.length > 0 && (
                    <table className="tbl" data-testid="gate-table">
                      <thead>
                        <tr>
                          <th style={{ textAlign: 'left' }}>gate</th>
                          <th>result</th>
                          <th>statistic / threshold</th>
                          <th>trials</th>
                          <th style={{ textAlign: 'left' }}>detail</th>
                        </tr>
                      </thead>
                      <tbody>
                        {g.verdicts.map((v) => (
                          <tr key={v.gate_no}>
                            <td style={{ textAlign: 'left' }}>
                              {v.gate_no}. {v.gate_name}
                            </td>
                            <td>
                              <Badge tone={v.passed ? 'pos' : 'neg'}>{v.passed ? 'passed' : 'failed'}</Badge>
                            </td>
                            <td>
                              {v.statistic ?? 'not recorded'} / {v.threshold ?? 'none'}
                            </td>
                            <td>{v.trial_count_at_eval ?? 'not recorded'}</td>
                            <td style={{ textAlign: 'left' }}>{v.detail}</td>
                          </tr>
                        ))}
                      </tbody>
                    </table>
                  )}
                </>
              )
            }}
          </ReadGate>
        )}
      </PanelBody>
    </Panel>
  )
}

// ── Timeline ─────────────────────────────────────────────────────────────────

function TimelinePanel() {
  const q = useQuery({ queryKey: ['governance', 'timeline'], queryFn: () => governanceApi.timeline(200) })
  return (
    <Panel>
      <PanelHeader title="Agent action timeline" />
      <PanelBody>
        <ReadGate query={q} route="/api/platform/timeline" what="the timeline">
          {(d) =>
            d.rows.length === 0 ? (
              <EmptyState message="Nothing recorded" detail="No audit or decision row exists for this account yet." />
            ) : (
              <>
                <p className="sub" data-testid="timeline-prevented">
                  {preventedRows(d.rows).length} of the last {d.rows.length} rows are actions that were denied or
                  held for approval. A denied action leaves no other trace.
                </p>
                <table className="tbl" data-testid="timeline-table">
                  <thead>
                    <tr>
                      <th style={{ textAlign: 'left' }}>when</th>
                      <th>kind</th>
                      <th style={{ textAlign: 'left' }}>what</th>
                      <th>verdict</th>
                      <th>actor</th>
                    </tr>
                  </thead>
                  <tbody>
                    {d.rows.map((r, i) => (
                      <tr key={`${r.at}/${i}`}>
                        <td style={{ textAlign: 'left' }}>{dateTime(r.at)}</td>
                        <td>{r.kind}</td>
                        <td style={{ textAlign: 'left' }}>
                          {r.what} <span className="sub">({r.phase})</span>
                        </td>
                        <td>
                          {r.verdict ? (
                            <Badge tone={r.verdict === 'denied' ? 'neg' : r.verdict === 'pending_approval' ? 'warn' : 'neutral'}>
                              {r.verdict.replace('_', ' ')}
                            </Badge>
                          ) : (
                            'not a gated action'
                          )}
                        </td>
                        <td>
                          {r.actor_kind}:{r.actor_id}
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </>
            )
          }
        </ReadGate>
      </PanelBody>
    </Panel>
  )
}

// ── Model aliases ────────────────────────────────────────────────────────────

function AliasRow({ model }: { model: GovernedModel }) {
  const qc = useQueryClient()
  const [rolling, setRolling] = useState<string | null>(null)
  const q = useQuery({
    queryKey: ['governance', 'aliases', model.model_id],
    queryFn: () => governanceApi.aliases(model.model_id),
  })
  const rollback = useMutation({
    mutationFn: async (alias: string) => governanceApi.rollback(model.model_id, alias),
    onSuccess: () => {
      setRolling(null)
      void qc.invalidateQueries({ queryKey: ['governance', 'aliases', model.model_id] })
      void qc.invalidateQueries({ queryKey: ['governance', 'timeline'] })
    },
  })
  const aliases = q.data?.state === 'ok' ? Object.entries(q.data.data) : null
  return (
    <tr data-testid={`alias-row-${model.model_id}`}>
      <td style={{ textAlign: 'left' }}>
        {model.display_name} <span className="sub">{model.model_kind}</span>
      </td>
      <td style={{ textAlign: 'left' }}>
        {q.isPending ? (
          'reading…'
        ) : aliases === null ? (
          <span className="sub">aliases could not be read</span>
        ) : aliases.length === 0 ? (
          <span className="sub">no alias set</span>
        ) : (
          aliases.map(([alias, version]) => (
            <span key={alias} style={{ marginRight: 'var(--s-3)' }}>
              {alias} → v{version}{' '}
              <Button
                size="xs"
                variant="ghost"
                onClick={() => {
                  rollback.reset()
                  setRolling(alias)
                }}
              >
                Roll back…
              </Button>
            </span>
          ))
        )}
        <ConfirmDialog
          open={rolling !== null}
          onCancel={() => setRolling(null)}
          onConfirm={() => rolling && rollback.mutate(rolling)}
          title={`Roll back "${rolling ?? ''}"?`}
          consequence={rollbackConsequence(
            model.display_name,
            rolling ?? '',
            aliases?.find(([a]) => a === rolling)?.[1],
          )}
          confirmLabel="Roll back"
          busy={rollback.isPending}
        />
        {rollback.isError && (
          <div className="sub" role="alert" data-testid="rollback-error">
            Refused: {refusalWords(rollback.error)}
          </div>
        )}
      </td>
    </tr>
  )
}

function AliasesPanel() {
  const q = useQuery({ queryKey: ['governance', 'models'], queryFn: () => governanceApi.models() })
  return (
    <Panel>
      <PanelHeader title="Model aliases" />
      <PanelBody>
        <ReadGate query={q} route="/api/models" what="the model list">
          {(d) =>
            d.models.length === 0 ? (
              <EmptyState message="No model is registered" />
            ) : (
              <table className="tbl" data-testid="alias-table">
                <thead>
                  <tr>
                    <th style={{ textAlign: 'left' }}>model</th>
                    <th style={{ textAlign: 'left' }}>alias → version</th>
                  </tr>
                </thead>
                <tbody>
                  {d.models.slice(0, 50).map((m) => (
                    <AliasRow key={m.model_id} model={m} />
                  ))}
                </tbody>
              </table>
            )
          }
        </ReadGate>
        <p className="sub">
          Rolling back moves an alias to the version it pointed at before, through the same route the model
          studio uses. No alias history is served, so the previous version is not shown until it is restored.
        </p>
      </PanelBody>
    </Panel>
  )
}

// ── Deployments ──────────────────────────────────────────────────────────────

function DeploymentsPanel() {
  const q = useQuery({ queryKey: ['governance', 'deployments'], queryFn: () => governanceApi.deployments() })
  return (
    <Panel>
      <PanelHeader title="Deployments" />
      <PanelBody>
        <ReadGate query={q} route="/api/deployments" what="the deployment records">
          {(rows) =>
            rows.length === 0 ? (
              <EmptyState message="Nothing has been deployed" />
            ) : (
              <table className="tbl" data-testid="deployments-table">
                <thead>
                  <tr>
                    <th style={{ textAlign: 'left' }}>strategy</th>
                    <th>revision</th>
                    <th>target</th>
                    <th>environment</th>
                    <th>standing</th>
                    <th>started</th>
                    <th style={{ textAlign: 'left' }}>stop reason</th>
                  </tr>
                </thead>
                <tbody>
                  {rows.map((d) => {
                    const st = deploymentStanding(d)
                    return (
                      <tr key={d.deployment_id}>
                        <td style={{ textAlign: 'left' }}>{d.strategy_name}</td>
                        <td>{d.revision ? d.revision.slice(0, 10) : 'not recorded'}</td>
                        <td>{d.instrument_id ?? 'a selection'}</td>
                        <td>{d.environment}</td>
                        <td>
                          <Badge tone={st.tone}>{st.word}</Badge>
                        </td>
                        <td>{dateTime(d.started_at)}</td>
                        <td style={{ textAlign: 'left' }}>{d.stop_reason ?? (d.stopped_at ? 'none recorded' : '')}</td>
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

function ConfigPanel() {
  return (
    <Panel>
      <PanelHeader title="Config diff and rollback playbook" />
      <PanelBody>
        <EmptyState
          message="No config history is recorded server-side"
          detail="A config-diff audit would diff two recorded configurations, and the platform records only the current one. This tab will not draw a diff of nothing."
        />
        <p className="sub">
          To undo a release today: roll the model alias back (Model aliases), and stop the deployment from its
          record (Deployments). Every step lands in the timeline.
        </p>
      </PanelBody>
    </Panel>
  )
}

export function GovernancePage() {
  const [tab, setTab] = useState<Tab>('evidence')
  return (
    <>
      <div className="pagehead">
        <h1 className="h1">Governance</h1>
        <div className="spacer" />
        <Segmented
          ariaLabel="Governance section"
          value={tab}
          onChange={setTab}
          options={[
            { value: 'evidence', label: 'Release evidence' },
            { value: 'timeline', label: 'Timeline' },
            { value: 'aliases', label: 'Model aliases' },
            { value: 'deployments', label: 'Deployments' },
            { value: 'config', label: 'Config' },
          ]}
        />
      </div>
      <div className="page-body" style={{ display: 'flex', flexDirection: 'column', gap: 'var(--s-3)' }}>
        {tab === 'evidence' && (
          <>
            <HealthPanel />
            <GatesPanel />
          </>
        )}
        {tab === 'timeline' && <TimelinePanel />}
        {tab === 'aliases' && <AliasesPanel />}
        {tab === 'deployments' && <DeploymentsPanel />}
        {tab === 'config' && <ConfigPanel />}
      </div>
    </>
  )
}
