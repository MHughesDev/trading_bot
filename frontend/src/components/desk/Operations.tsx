// Usage, monitor and the strategy editor (AGENT-012 §8, §11, §12; task S-6.21).
//
// Three surfaces that share one property: they are the operator's, not the
// researcher's. Each is wired against the endpoint its owning phase will provide
// and says plainly when that phase has not landed, rather than rendering a
// placeholder number somebody might believe.
import { useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Badge } from '@/components/primitives/Badge'
import { Button } from '@/components/primitives/Button'
import { Modal } from '@/components/primitives/Modal'
import { Panel, PanelBody, PanelHeader } from '@/components/primitives/Panel'
import { Field, Input, TextArea } from '@/components/primitives/Field'
import { EmptyState } from '@/components/primitives/States'
import { api } from '@/lib/api'
import {
  deskApi,
  type HaltPolicy,
  type StrategyAction,
  type StrategyActionBody,
} from '@/api/desk'
import { formatRate, nothingRecordedInLlmUsage, usageRows, killSwitchBadge, monitorRow, environmentBadge, strategyScope, strategyActionBody, strategyActionRefusal, globalActionBody } from './Operations.helpers'
import { GLOBAL_SCOPE } from './Operations.helpers'
import { MonitorLive } from './MonitorLive'

// ── What the runs produced (AGENT-013 §10; S-10.13) ──────────────────────────
//
// The one number here that is not about whether the harness *worked*. A barren
// run finished perfectly: it spent from the trial budget — the quantity every
// selection-adjusted statistic on this platform depends on — and produced
// nothing the next run can use.
//
// Rendered as rows and never as a single rate. "12% barren" is something nobody
// can act on; "`survey` on the 7B is barren 60% of the time" names a tier and a
// workflow, so somebody can change one of them.

function BarrenPanel() {
  const q = useQuery({
    queryKey: ['desk', 'barren'],
    queryFn: () => deskApi.barren(30),
  })
  const d = q.data

  return (
    <Panel>
      <PanelHeader title="What the runs produced" />
      <PanelBody>
        {d === null && (
          <EmptyState
            message="Not available"
            detail="/api/lifecycle/barren is served, so this is the endpoint failing rather than the endpoint missing."
          />
        )}
        {d && d.rows.length === 0 ? (
          <EmptyState
            message="Nothing measured"
            detail={`No run finished in the last ${d.days} days, so there is no rate — which is not the same as a rate of zero.`}
          />
        ) : (
          d && (
            <table className="tbl">
              <thead>
                <tr>
                  <th style={{ textAlign: 'left' }}>Tier</th>
                  <th style={{ textAlign: 'left' }}>Workflow</th>
                  <th>Runs</th>
                  <th>Barren</th>
                  <th>Rate</th>
                </tr>
              </thead>
              <tbody>
                {d.rows.map((r) => (
                  <tr key={`${r.tier}|${r.workflow}`}>
                    <td style={{ textAlign: 'left' }}>{r.tier}</td>
                    <td style={{ textAlign: 'left' }}>{r.workflow}</td>
                    {/* The denominator travels with the rate: 60% over five runs
                        and over five hundred are different claims. */}
                    <td>{r.runs}</td>
                    <td>{r.barren}</td>
                    <td>{formatRate(r.rate, r.runs)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          )
        )}
        {d && d.unresolved.length > 0 && (
          <p className="sub">
            {/* Named rather than folded into a guessed tier or dropped: folding
                attributes one tier's waste to another, and dropping makes the
                platform look less wasteful the more configurations it retires. */}
            Not attributed to a tier — the platform no longer ships these profiles:{' '}
            {d.unresolved.map((u) => `${u.profile} (${u.runs})`).join(', ')}
          </p>
        )}
        <p className="sub">
          A barren run spent a trial and produced no knowledge unit. Over the last {d?.days ?? 30} days.
        </p>
      </PanelBody>
    </Panel>
  )
}

// ── Usage (§12) ──────────────────────────────────────────────────────────────

/** One line of the spend table, already decided and already in words. */
export interface UsageLine {
  key: string
  label: string
  /** What to print. Never a `0` standing in for a number nobody has. */
  value: string
}

// ── Monitor (§11 — the UI of COMP-007) ───────────────────────────────────────

/**
 * One row of `/api/monitor/strategies`, read under the names it is served with.
 *
 * The table read `name`, `status` and `breaching_metric`; the route sends
 * `strategy_id`, `state` and `scale_pct`. Every row therefore rendered as two
 * blanks and a dash — a monitor showing a table of empty rows for live
 * strategies, which looks like a monitor with nothing to report. The same class
 * of defect as `cited` against `cited_by_chain`: no error, no clue, and the
 * wrong reading is the reassuring one.
 */
export interface MonitorRow {
  strategy_id: string
  state: string | null
  /**
   * `null` when the row carries no scale.
   *
   * Never folded into `0`: a strategy scaled to 0% has been shut down by the
   * monitor, and one whose scale was not recorded has not. Those are opposite
   * facts and they were one `?? 0` apart.
   */
  scale_pct: number | null
  /**
   * Which environment this strategy's orders were executed in, from
   * `orders.execution_mode` — a recorded fact per order, never the strategy's
   * name read as an environment.
   *
   * `null` is **unknown, not paper**: the strategy has placed no order, so
   * nothing anywhere says where its orders would go. `mixed` is a strategy with
   * orders in more than one mode, which the route refuses to collapse to one
   * word because both answers are true.
   */
  environment: string | null
}

// ── The per-row controls (§11; COMP-007 §6) ──────────────────────────────────
//
// `Scale down` was a `<Button>` with no `onClick`. It rendered, it took the
// hover, it took the click, and `POST /api/monitor/strategies/{id}/{action}` had
// been served since COMP-007 landed. A control that looks like a control and
// reaches nothing is worse on this page than no control at all: the operator
// walks away believing a losing strategy has been scaled back.

export interface StrategyActionInput {
  strategyId: string
  reason: string
  scalePct?: number | null
  haltPolicy?: HaltPolicy
  /** What the person typed into the confirmation box, for `resume`. */
  typed?: string
}

export type BuiltAction =
  | { ok: true; body: StrategyActionBody }
  | { ok: false; refusal: string }

export type GlobalAction = 'halt' | 'resume'

export type BuiltGlobal =
  | { ok: true; action: 'halt'; reason: string }
  | { ok: true; action: 'resume'; body: { confirm_scope: string; reason: string } }
  | { ok: false; refusal: string }

/**
 * The two-step confirm for the global halt and the global reset (Meridian §5.5).
 *
 * Prop-driven for the same reason `StrategyActionDialog` is: the refusal rules
 * are the part worth testing and they should not need a query client to reach.
 */
export function GlobalActionDialog({
  action,
  halted,
  busy = false,
  error = null,
  onCancel,
  onConfirm,
}: {
  action: GlobalAction | null
  /** The scopes the monitor says are halted, or `null` when it did not answer. */
  halted: string[] | null
  busy?: boolean
  error?: string | null
  onCancel: () => void
  onConfirm: (built: BuiltGlobal & { ok: true }) => void
}) {
  const [reason, setReason] = useState('')
  const [typed, setTyped] = useState('')

  if (action === null) return null
  const built = globalActionBody(action, { reason, typed })

  // A global reset is refused outright while any venue is still halted
  // (`ResetRefusal::VenuesNotReconciled`). Said here rather than discovered as a
  // 409, and said as "unknown" when the scope read did not answer — the desk
  // does not get to promise this will go through on a list it has not seen.
  const venueNote =
    action === 'resume'
      ? halted === null
        ? 'The halted scopes could not be read, so whether a venue still blocks this reset is unknown.'
        : halted.some((s) => s.startsWith('venue:'))
          ? `The monitor will refuse while a venue is halted: ${halted
              .filter((s) => s.startsWith('venue:'))
              .join(', ')}.`
          : null
      : null

  return (
    <Modal
      open
      onClose={onCancel}
      dismissible={false}
      title={action === 'halt' ? 'Halt all trading?' : 'Reset the global kill switch?'}
      footer={
        <>
          <span className="spacer" />
          <Button onClick={onCancel}>Cancel</Button>
          <Button
            variant={action === 'halt' ? 'danger' : 'primary'}
            disabled={!built.ok || busy}
            loading={busy}
            data-testid="global-action-confirm"
            onClick={() => built.ok && onConfirm(built)}
          >
            {action === 'halt' ? 'Halt all' : 'Reset'}
          </Button>
        </>
      }
    >
      <div className="callout neg" style={{ marginBottom: 'var(--s-4)' }} data-testid="global-action-consequence">
        {action === 'halt'
          ? 'Every strategy stops placing orders immediately, on every venue. Open positions are held, not closed. Only a reset — which is rate-limited and typed — undoes it.'
          : 'Order flow resumes across every scope that is not separately halted. The reset is rate-limited, logged against your name, and refused if another reset was made too recently.'}
      </div>
      {venueNote && (
        <p className="sub" data-testid="global-venue-note">
          {venueNote}
        </p>
      )}
      <Field label="Reason (required)">
        <TextArea rows={2} aria-label="Reason" value={reason} onChange={(e) => setReason(e.target.value)} />
      </Field>
      <Field label={`Type ${GLOBAL_SCOPE} to confirm`}>
        <Input aria-label="Typed confirmation" value={typed} onChange={(e) => setTyped(e.target.value)} />
      </Field>
      {!built.ok && (
        <p className="sub" data-testid="global-action-refusal">
          {built.refusal}
        </p>
      )}
      {error && (
        <p className="sub" data-testid="global-action-error" role="alert">
          The monitor refused: {error}
        </p>
      )}
    </Modal>
  )
}

const ACTION_TITLE: Record<StrategyAction, string> = {
  scale: 'Scale down',
  halt: 'Halt',
  resume: 'Resume',
}

/**
 * The two-step confirm for one strategy action (Meridian §5.5).
 *
 * Presentational and prop-driven: it owns the fields and nothing else, so the
 * refusal rules can be exercised without a query client, a router or a network.
 */
export function StrategyActionDialog({
  action,
  strategyId,
  currentScale,
  busy = false,
  error = null,
  onCancel,
  onConfirm,
}: {
  action: StrategyAction | null
  strategyId: string
  /** What the row says the strategy runs at now, or `null` if nobody said. */
  currentScale: number | null
  busy?: boolean
  error?: string | null
  onCancel: () => void
  onConfirm: (body: StrategyActionBody) => void
}) {
  const [reason, setReason] = useState('')
  const [pct, setPct] = useState('50')
  const [typed, setTyped] = useState('')

  // The caller remounts this per action with a `key`, so the fields start empty
  // every time. An effect that cleared them on close would leave the last
  // reason typed into the next strategy's dialog for one render — and a reason
  // is written into the monitor's event log against a name.
  if (action === null) return null

  const parsed = /^\d+$/.test(pct.trim()) ? Number.parseInt(pct.trim(), 10) : null
  const built = strategyActionBody(action, {
    strategyId,
    reason,
    scalePct: parsed,
    typed,
  })

  const consequence =
    action === 'scale'
      ? `${strategyId} will be recorded as scaled down and run at ${
          parsed === null ? 'the percentage you give' : `${parsed}%`
        } of full size${
          currentScale === null
            ? ' — the monitor has not reported its current scale, so this is not a change from a known number'
            : `, down from ${currentScale}%`
        }. The monitor writes a ScaledDown event against your name.`
      : action === 'halt'
        ? `${strategyId} stops placing orders immediately. Open positions are held, not closed. This trips the kill switch for ${strategyScope(
            strategyId,
          )} and only a resume — which is a reset, rate-limited — undoes it.`
        : `${strategyId} returns to nominal at 100% of full size and may place orders again. This is a reset: it is rate-limited, logged against your name, and refused if another reset was made too recently.`

  return (
    <Modal
      open
      onClose={onCancel}
      dismissible={false}
      title={`${ACTION_TITLE[action]} ${strategyId}?`}
      footer={
        <>
          <span className="spacer" />
          <Button onClick={onCancel}>Cancel</Button>
          <Button
            variant={action === 'resume' ? 'primary' : 'danger'}
            disabled={!built.ok || busy}
            loading={busy}
            data-testid="strategy-action-confirm"
            onClick={() => built.ok && onConfirm(built.body)}
          >
            {ACTION_TITLE[action]}
          </Button>
        </>
      }
    >
      <div className="callout neg" style={{ marginBottom: 'var(--s-4)' }} data-testid="strategy-action-consequence">
        {consequence}
      </div>
      {action === 'scale' && (
        <Field label="Percentage of full size (1–100)">
          <Input
            aria-label="Percentage of full size"
            inputMode="numeric"
            value={pct}
            onChange={(e) => setPct(e.target.value)}
          />
        </Field>
      )}
      <Field
        label={action === 'scale' ? 'Reason (recorded with the event)' : 'Reason (required)'}
      >
        <TextArea
          rows={2}
          aria-label="Reason"
          value={reason}
          onChange={(e) => setReason(e.target.value)}
        />
      </Field>
      {action === 'resume' && (
        <Field label={`Type ${strategyScope(strategyId)} to confirm`}>
          <Input
            aria-label="Typed confirmation"
            value={typed}
            onChange={(e) => setTyped(e.target.value)}
          />
        </Field>
      )}
      {/* The refusal the monitor would give, given before the request rather
          than after it. A disabled button with no reason beside it is a dead
          end the person cannot debug. */}
      {!built.ok && (
        <p className="sub" data-testid="strategy-action-refusal">
          {built.refusal}
        </p>
      )}
      {/* …and the refusal it actually gave, if it did. Never swallowed: a
          failed control that says nothing reads exactly like one that worked. */}
      {error && (
        <p className="sub" data-testid="strategy-action-error" role="alert">
          The monitor refused: {error}
        </p>
      )}
    </Modal>
  )
}

export function Monitor() {
  const [global, setGlobal] = useState<GlobalAction | null>(null)
  const [pending, setPending] = useState<{ action: StrategyAction; row: MonitorRow } | null>(null)
  const qc = useQueryClient()
  const strategies = useQuery({ queryKey: ['desk', 'monitor'], queryFn: () => deskApi.monitor() })
  const kill = useQuery({
    queryKey: ['desk', 'kill-switch'],
    queryFn: () => deskApi.killSwitch(),
    refetchInterval: 3_000,
  })
  const scopes = useQuery({
    queryKey: ['desk', 'monitor', 'kill-switch'],
    queryFn: () => deskApi.monitorKillSwitch(),
    refetchInterval: 3_000,
  })
  const tripped = kill.data?.kill_switch_active === true
  const badge = killSwitchBadge(kill.data)
  const windows = scopes.data?.maintenance_windows ?? []

  const act = useMutation({
    mutationFn: async ({ body }: { body: StrategyActionBody }) => {
      if (!pending) throw new Error('no action is pending')
      return (await deskApi.strategyAction(pending.row.strategy_id, pending.action, body)).data
    },
    onSuccess: () => {
      setPending(null)
      // Re-read rather than patching the row locally: the monitor decides what
      // the state and the scale now are, and a table showing what we *asked*
      // for would show a change a refusal may have undone.
      void qc.invalidateQueries({ queryKey: ['desk', 'monitor'] })
      void qc.invalidateQueries({ queryKey: ['desk', 'kill-switch'] })
    },
  })

  const globalAct = useMutation({
    mutationFn: async (built: BuiltGlobal & { ok: true }) =>
      built.action === 'halt'
        ? (await deskApi.haltAll(built.reason)).data
        : (await deskApi.resumeAll(built.body)).data,
    onSuccess: () => {
      setGlobal(null)
      void qc.invalidateQueries({ queryKey: ['desk', 'kill-switch'] })
      void qc.invalidateQueries({ queryKey: ['desk', 'monitor'] })
      void qc.invalidateQueries({ queryKey: ['desk', 'monitor', 'kill-switch'] })
    },
  })

  return (
    <div className="desk-scroll" style={{ display: 'flex', flexDirection: 'column', gap: 'var(--s-3)' }}>
      <Panel>
        <PanelHeader
          title="Kill switch"
          badges={
            <Badge tone={badge.tone} data-testid="kill-switch-badge">
              {badge.word}
            </Badge>
          }
        />
        <PanelBody>
          <p className="sub">
            {/* `scope` is not on `/api/trading/status` and never has been, so
                the old `?? 'global'` printed a word the endpoint never sent —
                and "global" is the widest possible claim to invent. The halted
                scopes below come from the route that does know. */}
            scope {kill.data?.scope ?? 'not recorded on this endpoint'} · tripped by{' '}
            {kill.data?.tripped_by ?? 'not recorded'} · {kill.data?.reason ?? 'no reason recorded'}
          </p>
          <p className="sub" data-testid="halted-scopes">
            {scopes.data === null || scopes.data === undefined
              ? 'halted scopes unknown — /api/monitor/kill-switch did not answer'
              : scopes.data.halted_scopes.length === 0
                ? 'no scope is halted · orders ' +
                  (scopes.data.orders_permitted ? 'permitted' : 'not permitted') +
                  ' · lease ' +
                  scopes.data.lease
                : `halted: ${scopes.data.halted_scopes.join(', ')} · lease ${scopes.data.lease}`}
          </p>
          {/* §4.3: an open maintenance window is a hole in the limits, and it is
              as visible here as a trip. The desk showed it nowhere. */}
          {windows.length > 0 && (
            <p className="sub" data-testid="maintenance-windows">
              {windows.length} open maintenance{' '}
              {windows.length === 1 ? 'window' : 'windows'} — limits are not enforced:{' '}
              {windows
                .map((w) => `${w.reason} (${w.opened_by}, until ${w.expires_at})`)
                .join('; ')}
            </p>
          )}
          {/* Both controls, always — the badge above may say `state unknown`,
              and an operator who cannot tell whether trading is halted still
              needs to be able to halt it. Gating the halt on a read that did not
              answer would make the failure of the read disable the response to
              it. */}
          <div className="desk-actions">
            <Button
              size="xs"
              variant="danger"
              disabled={tripped}
              onClick={() => {
                globalAct.reset()
                setGlobal('halt')
              }}
            >
              {tripped ? 'Halted' : 'Halt all'}
            </Button>
            <Button
              size="xs"
              variant="secondary"
              onClick={() => {
                globalAct.reset()
                setGlobal('resume')
              }}
            >
              Reset (logged)
            </Button>
          </div>
          <GlobalActionDialog
            key={global ?? 'closed'}
            action={global}
            halted={scopes.data ? scopes.data.halted_scopes : null}
            busy={globalAct.isPending}
            error={globalAct.isError ? strategyActionRefusal(globalAct.error) : null}
            onCancel={() => setGlobal(null)}
            onConfirm={(built) => globalAct.mutate(built)}
          />
        </PanelBody>
      </Panel>

      <Panel>
        <PanelHeader title="Strategies" />
        <PanelBody>
          {strategies.data === null || strategies.data === undefined ? (
            <EmptyState
              message="The monitor did not answer"
              detail="/api/monitor/strategies is served (COMP-007), so this is the request failing rather than the phase being unbuilt. The live strategies are unknown, not none."
            />
          ) : strategies.data.strategies.length === 0 ? (
            <EmptyState
              message="No strategy is under the monitor"
              detail="The endpoint answered with an empty list, which is a fact about the monitor rather than about this page."
            />
          ) : (
            <table className="tbl">
              <thead>
                <tr>
                  <th style={{ textAlign: 'left' }}>strategy</th>
                  <th>state</th>
                  <th>scale</th>
                  <th>environment</th>
                  <th />
                </tr>
              </thead>
              <tbody>
                {strategies.data.strategies.map((s) => {
                  const row = monitorRow(s as Record<string, unknown>)
                  return (
                    <tr key={row.strategy_id}>
                      <td style={{ textAlign: 'left' }}>{row.strategy_id}</td>
                      <td>{row.state ?? 'not reported'}</td>
                      {/* 0% is "the monitor shut this down"; an absent scale is
                          "nobody said". They must not look the same. */}
                      <td>{row.scale_pct === null ? 'not reported' : `${row.scale_pct}%`}</td>
                      {/* Read from the order journal, and `null` there means no
                          order has ever been placed — unknown, not paper. */}
                      <td>
                        {(() => {
                          const env = environmentBadge(row.environment)
                          return (
                            <Badge tone={env.tone} data-testid={`env-${row.strategy_id}`}>
                              {env.word}
                            </Badge>
                          )
                        })()}
                      </td>
                      <td>
                        <Button
                          size="xs"
                          variant="ghost"
                          onClick={() => {
                            act.reset()
                            setPending({ action: 'scale', row })
                          }}
                        >
                          Scale down
                        </Button>
                        {/* Halt and resume are offered on the state the route
                            reports. When it reports none, both are offered and
                            the dialog says the current state is unknown —
                            guessing which one applies would be the monitor
                            answering a question nobody asked it. */}
                        {row.state !== 'halted' && (
                          <Button
                            size="xs"
                            variant="ghost"
                            onClick={() => {
                              act.reset()
                              setPending({ action: 'halt', row })
                            }}
                          >
                            Halt
                          </Button>
                        )}
                        {row.state !== 'nominal' && (
                          <Button
                            size="xs"
                            variant="ghost"
                            onClick={() => {
                              act.reset()
                              setPending({ action: 'resume', row })
                            }}
                          >
                            Resume
                          </Button>
                        )}
                      </td>
                    </tr>
                  )
                })}
              </tbody>
            </table>
          )}
          <p className="sub">
            {/* The column that was here read `breaching_metric`, which the route
                does not send. An always-blank column is worse than a missing
                one: it reads as "nothing is breaching". */}
            Which limit a strategy is breaching is not on this route. `/api/monitor/events` carries
            the breaches and is not joined to these rows yet, so this table says what each strategy
            is, not what it did wrong.
          </p>
        </PanelBody>
      </Panel>

      {/* What the monitor knows: the live stream, feeds, venues, coverage,
          defects and drills (S-7.10). */}
      <MonitorLive />

      <StrategyActionDialog
        key={pending ? `${pending.action}:${pending.row.strategy_id}` : 'none'}
        action={pending?.action ?? null}
        strategyId={pending?.row.strategy_id ?? ''}
        currentScale={pending?.row.scale_pct ?? null}
        busy={act.isPending}
        error={act.isError ? strategyActionRefusal(act.error) : null}
        onCancel={() => {
          act.reset()
          setPending(null)
        }}
        onConfirm={(body) => act.mutate({ body })}
      />
    </div>
  )
}

// ── Strategy editor (§8) ─────────────────────────────────────────────────────

export interface ValidatorError {
  line: number
  message: string
  /** The edit that fixes it, applied verbatim. */
  fix: string | null
}

export function StrategyEditor({ initial = '' }: { initial?: string }) {
  const [text, setText] = useState(initial)
  const [errors, setErrors] = useState<ValidatorError[] | null>(null)
  const [preview, setPreview] = useState<Record<string, unknown> | null>(null)

  async function validate() {
    try {
      const { data } = await api.post<{ errors: ValidatorError[] }>('/api/strategies/compile', { source: text })
      setErrors(data.errors ?? [])
    } catch {
      setErrors(null)
    }
  }

  async function runPreview() {
    try {
      const { data } = await api.post<Record<string, unknown>>('/api/strategies/preview', { source: text })
      setPreview(data)
    } catch {
      setPreview(null)
    }
  }

  return (
    <div className="desk-scroll" style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 'var(--s-3)' }}>
      <Panel>
        <PanelHeader
          title="SLv2"
          actions={
            <>
              <Button size="xs" onClick={() => void validate()}>
                Validate
              </Button>
              <Button size="xs" variant="secondary" onClick={() => void runPreview()}>
                Preview
              </Button>
            </>
          }
        />
        <PanelBody>
          <TextArea
            rows={18}
            aria-label="Strategy source"
            value={text}
            onChange={(e) => setText(e.target.value)}
            className="mono"
          />
          {errors === null ? (
            <p className="sub">
              The compiler endpoint (/api/strategies/compile) belongs to AGENT-011, Set S phase 5. Until it
              answers, this editor does not claim your strategy is valid.
            </p>
          ) : errors.length === 0 ? (
            <p className="sub">No validator errors.</p>
          ) : (
            <ul style={{ fontSize: 'var(--t-12)', paddingLeft: 'var(--s-4)' }}>
              {errors.map((e, i) => (
                <li key={i}>
                  line {e.line}: {e.message}{' '}
                  {e.fix && (
                    <Button size="xs" variant="ghost" onClick={() => setText(e.fix as string)}>
                      Apply fix
                    </Button>
                  )}
                </li>
              ))}
            </ul>
          )}
        </PanelBody>
      </Panel>
      <Panel>
        <PanelHeader title="preview_signals" />
        <PanelBody>
          {preview === null ? (
            <EmptyState message="No preview" detail="Preview needs the feature engine's preview_signals (AGENT-011)." />
          ) : (
            <pre className="mono well" style={{ fontSize: 'var(--t-11)', overflow: 'auto', maxHeight: '50vh' }}>
              {JSON.stringify(preview, null, 2)}
            </pre>
          )}
        </PanelBody>
      </Panel>
    </div>
  )
}

/// The spend surface for `/desk/usage`.
///
/// Lives here rather than in `Operations.helpers.ts` because `DeskPage` and
/// `an-empty-usage-table-is-not-a-desk-that-spent-nothing` both import it from
/// `./Operations`, and because that file is deliberately JSX-free: its whole
/// value is that `usageRows`, `monitorRow` and the rest can be unit-tested
/// without rendering anything.
export function Usage() {
  const usage = useQuery({
    queryKey: ['desk', 'usage'],
    queryFn: () => deskApi.usage().then((r) => r.data),
  })
  const raw = usage.data as Record<string, unknown> | undefined
  const rows = usageRows(raw)
  const unrecorded = nothingRecordedInLlmUsage(raw)

  return (
    <div className="desk-scroll">
      <Panel>
        <PanelHeader title="Spend" />
        <PanelBody>
          {/* A request in flight is not an answer of nothing. The old copy
              asserted "/api/agent/usage answered with nothing" from the first
              render, before it had been asked. */}
          {usage.isPending ? (
            <p className="sub">Reading the spend totals…</p>
          ) : rows === null ? (
            <EmptyState
              message="The spend totals did not answer"
              detail="/api/agent/usage is served, so this is the request failing rather than the endpoint missing. What this desk has spent is unknown, not zero."
            />
          ) : (
            <table className="tbl">
              <tbody>
                {rows.map((r) => (
                  <tr key={r.key}>
                    <td style={{ textAlign: 'left' }}>{r.label}</td>
                    <td>{r.value}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
          {unrecorded && (
            // Stated, not left to be inferred from four zeros. The reader
            // cannot tell an empty table from an idle desk by looking at the
            // numbers, so the surface has to say which question is open.
            <p className="sub" data-testid="usage-unrecorded-note">
              Every total <code>/api/agent/usage</code> sums came back zero. That endpoint reads one table,{' '}
              <code>llm_usage</code>, which only the LLM proxy writes — and this desk's driver does not call
              the agent through it. So this desk may have spent nothing, or it may have spent a great deal
              that nothing recorded, and these totals cannot tell you which. Treat them as unread, not as
              nil. The run's own <code>tokens_in</code>/<code>tokens_out</code> are kept on{' '}
              <code>agent_runs</code>, which no endpoint exposes yet.
            </p>
          )}
          <p className="sub">
            Spend by day, by conversation, by model and by layer, cost per finding and per gate, and budget
            policies come from AGENT-009's published numbers and the per-layer accounting in AGENT-005. Neither
            is exposed by an endpoint yet, so the desk shows the totals that are. What the runs
            *produced* is below, from `/api/lifecycle/barren`.
          </p>
          <p className="sub">
            {/* Stated rather than left for somebody to deduce from a permanent
                zero: the count is over `agent_sessions` that filed a report, and
                the desk's conversations are `agent_runs`. Cost per verdict is
                therefore not a number this surface can produce today. */}
            `verdicts` counts sessions that filed a report. The desk's conversations are runs, not
            sessions, so this stays at zero however many dossiers the desk compiles — a gap in the
            endpoint, not a desk that has concluded nothing.
          </p>
          <Button
            size="xs"
            variant="secondary"
            disabled={rows === null}
            onClick={() => {
              if (rows === null) return
              // The rendered values, not the raw ones. A CSV carrying a cost of
              // 0 makes the same claim the table used to, somewhere nobody can
              // see the caveat beside it.
              const csv = [
                'metric,value',
                ...rows.map((r) => `${r.label},"${r.value.replace(/"/g, '""')}"`),
              ].join('\n')
              const url = URL.createObjectURL(new Blob([csv], { type: 'text/csv' }))
              const a = document.createElement('a')
              a.href = url
              a.download = 'desk-usage.csv'
              a.click()
              URL.revokeObjectURL(url)
            }}
          >
            Export CSV
          </Button>
        </PanelBody>
      </Panel>

      <BarrenPanel />
    </div>
  )
}
