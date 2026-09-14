import { useEffect, useMemo, useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { useNavigate, useSearchParams } from 'react-router-dom'
import { AlertTriangle, Info, Pause, Play, Plus, Trash2, X, Zap } from 'lucide-react'
import { api } from '@/lib/api'
import { useModeStore } from '@/store/mode'
import { useToast } from '@/hooks/useToast'
import { assetClassInfo } from '@/lib/assetClass'
import { DASH, relativeTime } from '@/lib/format'
import { Panel, PanelBody, PanelFooter, PanelHeader } from '@/components/primitives/Panel'
import { Button, IconButton } from '@/components/primitives/Button'
import { Badge, KeyValue } from '@/components/primitives/Badge'
import { Label } from '@/components/primitives/Num'
import { Segmented, Tabs } from '@/components/primitives/Segmented'
import { Table, IdentityCell, TableScroll } from '@/components/primitives/Table'
import { ConfirmDialog } from '@/components/primitives/Modal'
import { EmptyState, PanelLoading } from '@/components/primitives/States'
import { Tooltip } from '@/components/primitives/Overlay'
import { SingleInstrumentFlow } from '@/components/automations/SingleInstrumentFlow'
import { PipelineFlow } from '@/components/automations/PipelineFlow'
import { StageBoard } from '@/components/automations/StageBoard'

/* =============================================================================
   Spec §4.4 — Automations.

   "A full-width table (Name · Strategy · Markets · State · Today P&L ·
    Total P&L · Win rate · Trades · Last signal · Actions) + a detail drawer.
    State is a badge: Running (accent) · Paused (warn) · Stopped (neutral) ·
    Error (neg)."

   Domain fact the UI must carry: automations are SERVER-SIDE and each one is
   bound to a paper or live account at creation. The environment pill does not
   filter them — it would be dangerous to hide a live automation because the
   window happens to be in paper mode. Both are always listed, and the account
   each one trades is a column.
   ============================================================================= */

export interface AutomationSummary {
  id: string
  kind: 'single_instrument' | 'pipeline'
  account_mode: string
  armed: boolean
  active?: boolean
  created_at: string
  name?: string
  spec?: {
    asset_class?: string
    instrument_id?: string
    execution_strategy_id?: string
    universe?: string[]
    stages?: { stage_id: string; strategy_id: string; label: string }[]
    execution_action?: { execution_strategy_id?: string }
    trigger?: { kind: string; timeframe?: string; interval_secs?: number }
    time_window?: { start: string | null; end: string | null; timezone: string }
  }
}

type CreateKind = 'none' | 'single' | 'pipeline'

function stateOf(a: AutomationSummary): { label: string; tone: 'accent' | 'warn' | 'neutral' | 'neg' } {
  if (!a.armed) return { label: 'Stopped', tone: 'neutral' }
  if (a.active === false) return { label: 'Paused', tone: 'warn' }
  return { label: 'Running', tone: 'accent' }
}

function marketsOf(a: AutomationSummary): string {
  if (a.spec?.instrument_id) return a.spec.instrument_id
  const n = a.spec?.universe?.length ?? 0
  return n > 0 ? `${n} symbols` : DASH
}

function triggerOf(a: AutomationSummary): string {
  const t = a.spec?.trigger
  if (!t) return DASH
  if (t.kind === 'timer') return `Every ${t.interval_secs ?? 0}s`
  return `On ${t.timeframe ?? '1m'} bar close`
}

export function AutomationsPage() {
  const qc = useQueryClient()
  const toast = useToast()
  const navigate = useNavigate()
  const { mode } = useModeStore()
  const [params, setParams] = useSearchParams()
  const [creating, setCreating] = useState<CreateKind>('none')
  const [openId, setOpenId] = useState<string | null>(params.get('open'))
  const [confirmDelete, setConfirmDelete] = useState<AutomationSummary | null>(null)
  const [accountFilter, setAccountFilter] = useState<'all' | 'paper' | 'live'>('all')

  const automations = useQuery({
    queryKey: ['automations'],
    queryFn: async () => {
      const { data } = await api.get<{ automations: AutomationSummary[] }>('/api/automations')
      return data.automations ?? []
    },
    refetchInterval: 15_000,
  })

  useEffect(() => {
    const open = params.get('open')
    if (open) setOpenId(open)
  }, [params])

  const arm = useMutation({
    mutationFn: ({ id, armed }: { id: string; armed: boolean }) =>
      api.post(`/api/automations/${id}/${armed ? 'disarm' : 'arm'}`),
    onSuccess: (_, v) => {
      toast({ title: v.armed ? 'Automation stopped' : 'Automation armed' })
      void qc.invalidateQueries({ queryKey: ['automations'] })
    },
  })

  const remove = useMutation({
    mutationFn: (id: string) => api.delete(`/api/automations/${id}`),
    onSuccess: () => {
      toast({ title: 'Automation deleted' })
      setOpenId(null)
      void qc.invalidateQueries({ queryKey: ['automations'] })
    },
  })

  const list = automations.data ?? []
  const filtered = useMemo(
    () => (accountFilter === 'all' ? list : list.filter((a) => a.account_mode === accountFilter)),
    [list, accountFilter],
  )
  const open = list.find((a) => a.id === openId) ?? null
  const running = list.filter((a) => a.armed).length
  const liveRunning = list.filter((a) => a.armed && a.account_mode === 'live').length

  return (
    <>
      <div className="pagehead">
        <h1 className="h1">Automations</h1>
        <Badge tone={running > 0 ? 'accent' : 'neutral'}>{running} running</Badge>
        {liveRunning > 0 && (
          <Tooltip content="These trade real funds regardless of which environment this window is showing.">
            <Badge tone="pos">{liveRunning} live</Badge>
          </Tooltip>
        )}
        <div className="spacer" />
        <Segmented
          ariaLabel="Account filter"
          value={accountFilter}
          onChange={setAccountFilter}
          options={[
            { value: 'all', label: 'All accounts' },
            { value: 'paper', label: 'Paper' },
            { value: 'live', label: 'Live' },
          ]}
        />
        <Button size="sm" icon={<Plus size={13} aria-hidden />} onClick={() => setCreating('single')}>
          Single instrument
        </Button>
        <Button size="sm" variant="primary" icon={<Plus size={13} aria-hidden />} onClick={() => setCreating('pipeline')}>
          Pipeline
        </Button>
      </div>

      <div className="page-body">
        <div className="callout info">
          <Info size={14} aria-hidden style={{ flex: 'none', marginTop: 2 }} />
          <span>
            Automations run on the server, not in this browser. Each one is bound to the paper or live
            account it was created against and keeps running when you close the tab. This window is
            currently showing <strong>{mode === 'LIVE' ? 'live' : 'paper'}</strong> — that changes what the
            terminal trades, not what these do.
          </span>
        </div>

        {creating !== 'none' && (
          <Panel>
            <PanelHeader
              title={creating === 'single' ? 'New single-instrument automation' : 'New pipeline automation'}
              actions={
                <IconButton label="Cancel" bare onClick={() => setCreating('none')}>
                  <X size={14} aria-hidden />
                </IconButton>
              }
            />
            <PanelBody>
              {creating === 'single' ? (
                <SingleInstrumentFlow
                  onArmed={() => {
                    setCreating('none')
                    void qc.invalidateQueries({ queryKey: ['automations'] })
                  }}
                />
              ) : (
                <PipelineFlow
                  onArmed={() => {
                    setCreating('none')
                    void qc.invalidateQueries({ queryKey: ['automations'] })
                  }}
                />
              )}
            </PanelBody>
          </Panel>
        )}

        <Panel style={{ minHeight: 320 }}>
          <PanelHeader title="All automations" badges={<Badge tone="neutral">{filtered.length}</Badge>} />
          <PanelBody flush>
            {automations.isLoading ? (
              <div style={{ padding: 'var(--s-4)' }}>
                <PanelLoading lines={4} />
              </div>
            ) : (
              <TableScroll>
                <Table
                  caption="Automations"
                  rows={filtered}
                  rowKey={(a) => a.id}
                  selectedKey={openId}
                  onRowClick={(a) => {
                    setOpenId(a.id)
                    setParams({ open: a.id })
                  }}
                  empty={
                    <EmptyState
                      icon={<Zap size={20} aria-hidden />}
                      message="Nothing is automated yet"
                      detail="An automation watches a market, runs a strategy against it and acts — without you at the desk."
                      action={
                        <Button size="sm" variant="primary" onClick={() => setCreating('single')}>
                          Create your first
                        </Button>
                      }
                    />
                  }
                  columns={[
                    {
                      key: 'name',
                      header: 'Name',
                      align: 'left',
                      numeric: false,
                      sortValue: (a) => a.name ?? a.kind,
                      cell: (a) => (
                        <IdentityCell
                          ticker={a.name ?? a.kind.replace(/_/g, ' ')}
                          name={assetClassInfo(a.spec?.asset_class).label}
                        />
                      ),
                    },
                    {
                      key: 'strategy',
                      header: 'Strategy',
                      numeric: false,
                      cell: (a) => (
                        <span className="mono mut truncate-1">
                          {a.spec?.execution_strategy_id ?? a.spec?.execution_action?.execution_strategy_id ?? DASH}
                        </span>
                      ),
                    },
                    { key: 'markets', header: 'Markets', numeric: false, cell: (a) => marketsOf(a) },
                    {
                      key: 'state',
                      header: 'State',
                      numeric: false,
                      sortValue: (a) => (a.armed ? 0 : 1),
                      cell: (a) => {
                        const s = stateOf(a)
                        return <Badge tone={s.tone}>{s.label}</Badge>
                      },
                    },
                    {
                      key: 'account',
                      header: 'Account',
                      numeric: false,
                      cell: (a) => <Badge tone={a.account_mode === 'live' ? 'pos' : 'warn'}>{a.account_mode}</Badge>,
                    },
                    { key: 'today', header: "Today's P&L", cell: () => <span className="mut">{DASH}</span> },
                    { key: 'total', header: 'Total P&L', cell: () => <span className="mut">{DASH}</span> },
                    { key: 'trades', header: 'Trades', cell: () => <span className="mut">{DASH}</span> },
                    { key: 'trigger', header: 'Trigger', numeric: false, cell: (a) => <span className="mut">{triggerOf(a)}</span> },
                    {
                      key: 'created',
                      header: 'Created',
                      numeric: false,
                      sortValue: (a) => a.created_at,
                      cell: (a) => <span className="mut">{relativeTime(a.created_at)}</span>,
                    },
                    {
                      key: 'act',
                      header: '',
                      numeric: false,
                      cell: (a) => (
                        <span className="row" style={{ justifyContent: 'flex-end', gap: 2 }}>
                          <IconButton
                            label={a.armed ? 'Stop automation' : 'Arm automation'}
                            bare
                            onClick={(e) => {
                              e.stopPropagation()
                              arm.mutate({ id: a.id, armed: a.armed })
                            }}
                          >
                            {a.armed ? <Pause size={12} aria-hidden /> : <Play size={12} aria-hidden />}
                          </IconButton>
                          <IconButton
                            label="Delete automation"
                            bare
                            onClick={(e) => {
                              e.stopPropagation()
                              setConfirmDelete(a)
                            }}
                          >
                            <Trash2 size={12} aria-hidden />
                          </IconButton>
                        </span>
                      ),
                    },
                  ]}
                />
              </TableScroll>
            )}
          </PanelBody>
          <PanelFooter>
            <Label>
              Per-automation P&L and trade counts need an attribution feed. Until the execution log tags
              fills with the automation that caused them, these columns stay honestly blank.
            </Label>
          </PanelFooter>
        </Panel>
      </div>

      {open && (
        <AutomationDrawer
          automation={open}
          onClose={() => {
            setOpenId(null)
            setParams({})
          }}
          onToggle={() => arm.mutate({ id: open.id, armed: open.armed })}
          onDelete={() => setConfirmDelete(open)}
          onOpenTerminal={(symbol) => navigate(`/terminal/${encodeURIComponent(symbol)}`)}
        />
      )}

      <ConfirmDialog
        open={!!confirmDelete}
        onCancel={() => setConfirmDelete(null)}
        onConfirm={() => {
          if (confirmDelete) remove.mutate(confirmDelete.id)
          setConfirmDelete(null)
        }}
        title="Delete this automation?"
        confirmLabel="Delete permanently"
        typeToConfirm={confirmDelete?.account_mode === 'live' ? 'DELETE' : undefined}
        consequence={
          <>
            <strong>{confirmDelete?.name ?? confirmDelete?.kind.replace(/_/g, ' ')}</strong> stops
            immediately and its configuration is removed. Positions it opened stay open and become your
            responsibility to manage.
          </>
        }
        alternative={
          confirmDelete?.armed ? (
            <Button
              size="sm"
              onClick={() => {
                if (confirmDelete) arm.mutate({ id: confirmDelete.id, armed: true })
                setConfirmDelete(null)
              }}
            >
              Stop it instead
            </Button>
          ) : undefined
        }
      />
    </>
  )
}

/* --- detail drawer ---------------------------------------------------------- */
function AutomationDrawer({
  automation,
  onClose,
  onToggle,
  onDelete,
  onOpenTerminal,
}: {
  automation: AutomationSummary
  onClose: () => void
  onToggle: () => void
  onDelete: () => void
  onOpenTerminal: (symbol: string) => void
}) {
  const [tab, setTab] = useState<'overview' | 'stages' | 'activity'>('overview')
  const s = stateOf(automation)
  const spec = automation.spec ?? {}

  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if (e.key === 'Escape') onClose()
    }
    document.addEventListener('keydown', onKey)
    return () => document.removeEventListener('keydown', onKey)
  }, [onClose])

  return (
    <>
      <div
        className="scrim"
        style={{ background: 'var(--bg-scrim)', display: 'block' }}
        onMouseDown={onClose}
        aria-hidden
      />
      <aside className="drawer" role="dialog" aria-modal="true" aria-label="Automation detail">
        <div className="panel-hd tall" style={{ paddingLeft: 'var(--s-5)', paddingRight: 'var(--s-4)' }}>
          <div style={{ minWidth: 0 }}>
            <div className="row" style={{ gap: 'var(--s-2)' }}>
              <span className="h2 truncate-1">{automation.name ?? automation.kind.replace(/_/g, ' ')}</span>
              <Badge tone={s.tone}>{s.label}</Badge>
            </div>
            <div className="lbl" style={{ marginTop: 2 }}>
              {assetClassInfo(spec.asset_class).label} {'·'} {marketsOf(automation)} {'·'}{' '}
              {automation.account_mode} account
            </div>
          </div>
          <span className="spacer" />
          <IconButton label="Close" bare onClick={onClose}>
            <X size={15} aria-hidden />
          </IconButton>
        </div>

        <Tabs
          ariaLabel="Automation detail"
          value={tab}
          onChange={setTab}
          tabs={[
            { value: 'overview', label: 'Overview' },
            ...(automation.kind === 'pipeline' ? [{ value: 'stages' as const, label: 'Stages' }] : []),
            { value: 'activity', label: 'Activity' },
          ]}
        />

        <div style={{ flex: 1, minHeight: 0, overflow: 'auto', padding: 'var(--s-5)' }}>
          {tab === 'overview' && (
            <>
              <Label>Configuration</Label>
              <div style={{ marginTop: 'var(--s-2)', marginBottom: 'var(--s-5)' }}>
                <KeyValue k="Kind" v={automation.kind.replace(/_/g, ' ')} />
                <KeyValue k="Asset class" v={assetClassInfo(spec.asset_class).label} />
                <KeyValue k="Markets" v={marketsOf(automation)} />
                <KeyValue k="Strategy" v={spec.execution_strategy_id ?? spec.execution_action?.execution_strategy_id ?? DASH} />
                <KeyValue k="Trigger" v={triggerOf(automation)} />
                <KeyValue
                  k="Session"
                  v={
                    spec.time_window?.start
                      ? `${spec.time_window.start}–${spec.time_window.end} ${spec.time_window.timezone}`
                      : 'Continuous'
                  }
                />
                <KeyValue k="Created" v={relativeTime(automation.created_at)} />
              </div>

              {spec.universe && spec.universe.length > 0 && (
                <>
                  <Label>Universe</Label>
                  <div className="row" style={{ flexWrap: 'wrap', gap: 6, marginTop: 'var(--s-2)', marginBottom: 'var(--s-5)' }}>
                    {spec.universe.map((u) => (
                      <button key={u} type="button" className="chip" onClick={() => onOpenTerminal(u)} title={`Open ${u}`}>
                        {u}
                      </button>
                    ))}
                  </div>
                </>
              )}

              {spec.instrument_id && (
                <Button size="sm" onClick={() => onOpenTerminal(spec.instrument_id!)}>
                  Open {spec.instrument_id} in the terminal
                </Button>
              )}

              <div className="callout warn" style={{ marginTop: 'var(--s-5)' }}>
                <AlertTriangle size={13} aria-hidden style={{ flex: 'none', marginTop: 2 }} />
                <span>
                  Performance attribution is not wired yet. When the execution log tags fills with the
                  automation that produced them, this panel gains P&L, win rate and a trade ledger.
                </span>
              </div>
            </>
          )}

          {tab === 'stages' && <StageBoard automationId={automation.id} />}

          {tab === 'activity' && (
            <EmptyState
              message="No activity feed yet"
              detail="Signals, orders and fills produced by this automation will stream here once the server publishes them per automation."
            />
          )}
        </div>

        <div className="panel-ft">
          <Button icon={automation.armed ? <Pause size={13} /> : <Play size={13} />} onClick={onToggle}>
            {automation.armed ? 'Stop' : 'Arm'}
          </Button>
          <span className="spacer" />
          <Button variant="danger" icon={<Trash2 size={13} />} onClick={onDelete}>
            Delete
          </Button>
        </div>
      </aside>
    </>
  )
}

