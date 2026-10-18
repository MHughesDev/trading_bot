// Reconciliation: divergence halts and the fleet dashboard (Set T T-0.3, T-2.2).
//
// Two questions, in the order a person arrives with them:
//
//   1. *Which strategies are refused orders right now, and why?* The halt list.
//      "The halt is visible in the UI with its reason" is the acceptance clause
//      this page exists to close, and the only way a halt is lifted is here: a
//      named human, the strategy id typed out, and a note. There is no control
//      on this page that continues on divergence, and no bulk clear — one
//      strategy per confirmation.
//   2. *Does the simulator still match the fleet?* The fleet verdict: per
//      deployment realised-versus-expected, the aggregate, and whether the
//      divergence is one common-mode finding or several unrelated ones.
//
// Both read through `Read<T>`. A halt list that did not load is "unknown",
// which is not the same sentence as "nothing is halted".
import { useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { RotateCw } from 'lucide-react'
import { Badge } from '@/components/primitives/Badge'
import { Button, IconButton } from '@/components/primitives/Button'
import { Input } from '@/components/primitives/Field'
import { Modal } from '@/components/primitives/Modal'
import { Panel, PanelBody, PanelHeader } from '@/components/primitives/Panel'
import { EmptyState } from '@/components/primitives/States'
import { ReadGate } from '@/components/ops/ReadGate'
import { reconciliationApi, refusalWords, type DivergenceHalt } from '@/api/ops'
import { dateTime } from '@/lib/format'
import { bpsWords, clearReadiness, fleetView, haltStanding } from '@/lib/ops/reconciliation'

const HALTS = ['reconciliation', 'halts'] as const
const FLEET = ['reconciliation', 'fleet'] as const

function ClearHaltDialog({ halt, onClose }: { halt: DivergenceHalt | null; onClose: () => void }) {
  const qc = useQueryClient()
  const [typed, setTyped] = useState('')
  const [note, setNote] = useState('')
  const readiness = halt ? clearReadiness(halt, typed, note) : { ready: false, why: null }

  const clear = useMutation({
    mutationFn: async () => {
      if (!halt) throw new Error('no halt selected')
      return (
        await reconciliationApi.clearHalt(halt.strategy_id, {
          confirm_strategy: typed,
          note: note.trim(),
        })
      ).data
    },
    onSuccess: () => {
      // Re-read: the registry decides whether it is cleared, and a list that
      // dropped the row locally would show a clear a refusal may have undone.
      void qc.invalidateQueries({ queryKey: HALTS })
      onClose()
    },
  })

  return (
    <Modal
      open={halt !== null}
      onClose={onClose}
      // Not dismissible by Esc or a scrim click: resuming a halted strategy is a
      // decision, and a stray keypress is not one.
      dismissible={false}
      title="Clear a divergence halt"
      description={halt ? `${halt.strategy_id} · ${halt.kind}` : undefined}
      footer={
        <>
          <span className="spacer" />
          <Button onClick={onClose}>Cancel</Button>
          <Button
            variant="danger"
            disabled={!readiness.ready}
            loading={clear.isPending}
            onClick={() => clear.mutate()}
          >
            Clear this halt
          </Button>
        </>
      }
    >
      {halt && (
        <>
          <p className="sub" data-testid="clear-halt-reason">
            Halted because: {halt.reason}
          </p>
          <p className="sub">
            Clearing lets this strategy submit orders at its next decision. It is recorded with your
            name and note. The divergence that raised it is not re-measured here: if it is still
            there, the next check raises it again.
          </p>
          <Input
            label={
              <>
                Type <strong>{halt.strategy_id}</strong> to confirm
              </>
            }
            value={typed}
            onChange={(e) => setTyped(e.target.value)}
            autoComplete="off"
            spellCheck={false}
            placeholder={halt.strategy_id}
          />
          <Input
            label="Why it is safe to resume"
            value={note}
            onChange={(e) => setNote(e.target.value)}
            autoComplete="off"
            placeholder="What you checked, in one sentence"
          />
          {readiness.why && (
            <p className="sub" data-testid="clear-halt-why">
              {readiness.why}
            </p>
          )}
          {clear.isError && (
            <p className="sub" role="alert" data-testid="clear-halt-error">
              Refused: {refusalWords(clear.error)}
            </p>
          )}
        </>
      )}
    </Modal>
  )
}

function HaltsPanel() {
  const [pending, setPending] = useState<DivergenceHalt | null>(null)
  const q = useQuery({
    queryKey: HALTS,
    queryFn: () => reconciliationApi.halts(),
    refetchInterval: 15_000,
  })
  return (
    <Panel>
      <PanelHeader
        title="Divergence halts"
        badges={
          q.data?.state === 'ok' ? (
            <Badge tone={q.data.data.halts.length > 0 ? 'neg' : 'pos'} data-testid="halt-count">
              {q.data.data.halts.length} in force
            </Badge>
          ) : undefined
        }
      />
      <PanelBody>
        <ReadGate query={q} route="/api/reconciliation/halts" what="the halt list">
          {(d) => (
            <>
              <p className="sub">
                {d.blocked_orders} {d.blocked_orders === 1 ? 'order has' : 'orders have'} been refused by
                these halts since the platform started.
              </p>
              {d.halts.length === 0 ? (
                <EmptyState
                  message="No strategy is halted for divergence"
                  detail="The route answered with an empty list. Divergence halts are only raised for deployments that record forward observations."
                />
              ) : (
                <table className="tbl" data-testid="halts-table">
                  <thead>
                    <tr>
                      <th style={{ textAlign: 'left' }}>strategy</th>
                      <th>kind</th>
                      <th>standing</th>
                      <th>raised</th>
                      <th style={{ textAlign: 'left' }}>reason</th>
                      <th />
                    </tr>
                  </thead>
                  <tbody>
                    {d.halts.map((h) => {
                      const standing = haltStanding(h)
                      return (
                        <tr key={h.halt_id}>
                          <td style={{ textAlign: 'left' }}>{h.strategy_id}</td>
                          <td>{h.kind}</td>
                          <td>
                            <Badge tone={standing.tone} title={standing.meaning}>
                              {standing.word}
                            </Badge>
                          </td>
                          <td>{dateTime(h.raised_at)}</td>
                          <td style={{ textAlign: 'left' }}>{h.reason}</td>
                          <td>
                            <Button size="xs" variant="ghost" onClick={() => setPending(h)}>
                              Clear…
                            </Button>
                          </td>
                        </tr>
                      )
                    })}
                  </tbody>
                </table>
              )}
            </>
          )}
        </ReadGate>
      </PanelBody>
      <ClearHaltDialog key={pending?.halt_id ?? 'none'} halt={pending} onClose={() => setPending(null)} />
    </Panel>
  )
}

function FleetPanel() {
  const q = useQuery({
    queryKey: FLEET,
    queryFn: () => reconciliationApi.fleet(),
    refetchInterval: 60_000,
  })
  return (
    <Panel>
      <PanelHeader title="Fleet divergence" />
      <PanelBody>
        <ReadGate query={q} route="/api/reconciliation/fleet" what="the fleet report">
          {(r) => {
            const v = fleetView(r)
            return (
              <>
                <p data-testid="fleet-verdict">
                  <Badge tone={v.tone}>{v.headline}</Badge> <span className="sub">{v.sentence}</span>
                </p>
                <p className="sub">
                  Window {dateTime(r.window.from)} to {dateTime(r.window.to)} ({r.window.days} days).
                  Policy: tolerance {bpsWords(r.policy.tolerance_per_bar, 0)} per bar, quorum{' '}
                  {r.policy.quorum} deployments, at least {r.policy.min_bars} observed bars each.
                </p>
                {r.defect_id && (
                  <p className="sub" data-testid="fleet-defect">
                    Filed as platform defect {r.defect_id}. It is an observation awaiting a human replay,
                    not a confirmed simulator defect.
                  </p>
                )}
                {r.aggregate && (
                  <p className="sub" data-testid="fleet-aggregate">
                    Aggregate: {r.aggregate.measurable} of {r.aggregate.deployments} deployments measurable
                    over {r.aggregate.bars} bars, mean divergence {bpsWords(r.aggregate.mean_divergence)}{' '}
                    per bar in {r.aggregate.numeraire}; {r.aggregate.diverging} diverging.
                  </p>
                )}
                {r.deployments.length === 0 ? (
                  <EmptyState
                    message="No deployment has forward observations in the window"
                    detail="Nothing to compare. That reads as 'not computable', never as 'no divergence'."
                  />
                ) : (
                  <table className="tbl" data-testid="fleet-table">
                    <thead>
                      <tr>
                        <th style={{ textAlign: 'left' }}>strategy</th>
                        <th>bars</th>
                        <th>mean divergence / bar</th>
                        <th>cumulative</th>
                        <th>slippage over model</th>
                      </tr>
                    </thead>
                    <tbody>
                      {r.deployments.map((d) => (
                        <tr key={d.deployment_id}>
                          <td style={{ textAlign: 'left' }}>{d.strategy_id}</td>
                          <td>
                            {d.bars}
                            {d.backfilled_excluded > 0 && ` (+${d.backfilled_excluded} backfilled, excluded)`}
                          </td>
                          <td>{bpsWords(d.mean_divergence)}</td>
                          <td>{bpsWords(d.cumulative_divergence)}</td>
                          <td>
                            {d.slippage_excess_bps === null ? 'no traded window' : `${d.slippage_excess_bps.toFixed(2)} bps`}
                          </td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                )}
              </>
            )
          }}
        </ReadGate>
      </PanelBody>
    </Panel>
  )
}

export function ReconciliationPage() {
  const qc = useQueryClient()
  return (
    <>
      <div className="pagehead">
        <h1 className="h1">Reconciliation</h1>
        <div className="spacer" />
        <IconButton
          label="Refresh"
          onClick={() => {
            void qc.invalidateQueries({ queryKey: HALTS })
            void qc.invalidateQueries({ queryKey: FLEET })
          }}
        >
          <RotateCw size={14} aria-hidden />
        </IconButton>
      </div>
      <div className="page-body" style={{ display: 'flex', flexDirection: 'column', gap: 'var(--s-3)' }}>
        <HaltsPanel />
        <FleetPanel />
      </div>
    </>
  )
}
