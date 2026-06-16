// Terminal panel: 1-min OHLCV chart + asset-class-specific order ticket +
// positions + working orders + fills.  No order book, no DOM.

import { useState } from 'react'
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { useModeStore } from '@/store/mode'
import { tradeApi, paperApi } from '@/lib/api'
import { cn } from '@/lib/utils'
import { ChevronDown, ChevronRight as ChevronRightIcon } from 'lucide-react'

// Shared react-query key for an instrument's paper activity (orders + position).
const activityKey = (instrument: string) => ['paper-activity', instrument]

// ── Asset-class helpers ────────────────────────────────────────────────────────

type OrderSide = 'buy' | 'sell'
type OrderType = 'market' | 'limit' | 'stop'
type Tif = 'day' | 'gtc' | 'ioc' | 'fok'

export type TerminalAssetClass =
  | 'crypto_spot_cex'
  | 'crypto_spot_dex'
  | 'perpetual_swap'
  | 'futures_expiring'
  | 'equity'
  | 'etf'
  | 'bond'
  | 'fx'
  | 'option'
  | 'prediction_market'

function isClobClass(ac: TerminalAssetClass): boolean {
  return (
    ac === 'crypto_spot_cex' ||
    ac === 'crypto_spot_dex' ||
    ac === 'perpetual_swap' ||
    ac === 'futures_expiring'
  )
}

function isPredictionClass(ac: TerminalAssetClass): boolean {
  return ac === 'prediction_market'
}

// ── Sub-components ─────────────────────────────────────────────────────────────

function SideToggle({
  side,
  onChange,
}: {
  side: OrderSide
  onChange: (s: OrderSide) => void
}) {
  return (
    <div className="flex rounded-lg overflow-hidden border border-border text-sm">
      {(['buy', 'sell'] as OrderSide[]).map((s) => (
        <button
          key={s}
          onClick={() => onChange(s)}
          className={cn(
            'flex-1 py-1.5 font-medium capitalize transition-colors',
            s === side
              ? s === 'buy'
                ? 'bg-green-500/20 text-green-400'
                : 'bg-red-500/20 text-red-400'
              : 'text-text-muted hover:text-text',
          )}
        >
          {s}
        </button>
      ))}
    </div>
  )
}

function FieldLabel({ children }: { children: React.ReactNode }) {
  return (
    <label className="block text-xs text-text-dim mb-1">{children}</label>
  )
}

function Input({
  value,
  onChange,
  placeholder,
  type = 'text',
  disabled,
}: {
  value: string
  onChange: (v: string) => void
  placeholder?: string
  type?: string
  disabled?: boolean
}) {
  return (
    <input
      type={type}
      value={value}
      onChange={(e) => onChange(e.target.value)}
      placeholder={placeholder}
      disabled={disabled}
      className={cn(
        'w-full rounded-lg px-3 py-1.5 text-sm bg-surface-2 border border-border text-text',
        'placeholder:text-text-dim focus:outline-none focus:ring-1 focus:ring-accent',
        'disabled:opacity-50',
      )}
    />
  )
}

function SelectField({
  value,
  onChange,
  options,
}: {
  value: string
  onChange: (v: string) => void
  options: Array<{ value: string; label: string }>
}) {
  return (
    <select
      value={value}
      onChange={(e) => onChange(e.target.value)}
      className={cn(
        'w-full rounded-lg px-3 py-1.5 text-sm bg-surface-2 border border-border text-text',
        'focus:outline-none focus:ring-1 focus:ring-accent appearance-none',
      )}
    >
      {options.map((o) => (
        <option key={o.value} value={o.value}>
          {o.label}
        </option>
      ))}
    </select>
  )
}

// ── Order ticket ───────────────────────────────────────────────────────────────

function OrderTicket({
  instrument,
  assetClass,
}: {
  instrument: string
  assetClass: TerminalAssetClass
}) {
  const { mode } = useModeStore()
  const qc = useQueryClient()

  const clob = isClobClass(assetClass)
  const prediction = isPredictionClass(assetClass)

  const [side, setSide] = useState<OrderSide>('buy')
  const [orderType, setOrderType] = useState<OrderType>('market')
  const [qty, setQty] = useState('')
  const [limitPrice, setLimitPrice] = useState('')
  const [tpPrice, setTpPrice] = useState('')
  const [slPrice, setSlPrice] = useState('')
  const [tif, setTif] = useState<Tif>('gtc')
  const [predSide, setPredSide] = useState<'yes' | 'no'>('yes')

  const mutation = useMutation({
    mutationFn: () => {
      const body: Record<string, unknown> = {
        instrument_id: instrument,
        side: prediction ? predSide : side,
        // Backend order types are market | limit | stop_limit.
        order_type: orderType === 'stop' ? 'stop_limit' : orderType,
        qty,
        execution_mode: mode,
      }
      if (orderType === 'limit' || orderType === 'stop') {
        body.limit_price = limitPrice
      }
      if (clob && tpPrice) body.tp_price = tpPrice
      if (clob && slPrice) body.sl_price = slPrice
      if (!clob && !prediction) body.tif = tif
      return tradeApi.order(body)
    },
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: activityKey(instrument) })
      setQty('')
      setLimitPrice('')
      setTpPrice('')
      setSlPrice('')
    },
  })

  const canSubmit = !!qty && !mutation.isPending

  return (
    <div className="p-3 space-y-3">
      {/* Side / prediction toggle */}
      {prediction ? (
        <div className="flex rounded-lg overflow-hidden border border-border text-sm">
          {(['yes', 'no'] as const).map((s) => (
            <button
              key={s}
              onClick={() => setPredSide(s)}
              className={cn(
                'flex-1 py-1.5 font-semibold uppercase transition-colors',
                predSide === s
                  ? s === 'yes'
                    ? 'bg-green-500/20 text-green-400'
                    : 'bg-red-500/20 text-red-400'
                  : 'text-text-muted hover:text-text',
              )}
            >
              {s.toUpperCase()}
            </button>
          ))}
        </div>
      ) : (
        <SideToggle side={side} onChange={setSide} />
      )}

      {/* Order type */}
      <div>
        <FieldLabel>Order type</FieldLabel>
        <SelectField
          value={orderType}
          onChange={(v) => setOrderType(v as OrderType)}
          options={[
            { value: 'market', label: 'Market' },
            { value: 'limit', label: 'Limit' },
            ...(clob ? [{ value: 'stop', label: 'Stop' }] : []),
          ]}
        />
      </div>

      {/* Quantity */}
      <div>
        <FieldLabel>Qty</FieldLabel>
        <Input
          value={qty}
          onChange={setQty}
          placeholder={prediction ? 'Contracts' : 'Amount'}
          type="text"
        />
      </div>

      {/* Limit / stop price */}
      {(orderType === 'limit' || orderType === 'stop') && (
        <div>
          <FieldLabel>{orderType === 'stop' ? 'Stop price' : 'Limit price'}</FieldLabel>
          <Input
            value={limitPrice}
            onChange={setLimitPrice}
            placeholder="0.00"
            type="text"
          />
        </div>
      )}

      {/* TIF — non-CLOB only */}
      {!clob && !prediction && (
        <div>
          <FieldLabel>Time in force</FieldLabel>
          <SelectField
            value={tif}
            onChange={(v) => setTif(v as Tif)}
            options={[
              { value: 'day', label: 'Day' },
              { value: 'gtc', label: 'GTC' },
              { value: 'ioc', label: 'IOC' },
              { value: 'fok', label: 'FOK' },
            ]}
          />
        </div>
      )}

      {/* Bracket TP/SL — CLOB only */}
      {clob && (
        <>
          <div>
            <FieldLabel>Take profit (optional)</FieldLabel>
            <Input value={tpPrice} onChange={setTpPrice} placeholder="TP price" />
          </div>
          <div>
            <FieldLabel>Stop loss (optional)</FieldLabel>
            <Input value={slPrice} onChange={setSlPrice} placeholder="SL price" />
          </div>
        </>
      )}

      {mutation.isError && (
        <p className="text-xs text-red-400">Order failed — please try again.</p>
      )}

      <button
        disabled={!canSubmit}
        onClick={() => mutation.mutate()}
        className={cn(
          'w-full rounded-lg py-2 text-sm font-semibold transition-colors',
          side === 'buy' && !prediction
            ? 'bg-green-600 hover:bg-green-500 text-white disabled:opacity-40'
            : 'bg-red-600 hover:bg-red-500 text-white disabled:opacity-40',
          prediction &&
            (predSide === 'yes'
              ? 'bg-green-600 hover:bg-green-500 text-white disabled:opacity-40'
              : 'bg-red-600 hover:bg-red-500 text-white disabled:opacity-40'),
        )}
      >
        {mutation.isPending
          ? 'Submitting…'
          : `${mode} ${prediction ? predSide.toUpperCase() : side.toUpperCase()}`}
      </button>
    </div>
  )
}

// ── Paper activity hook ─────────────────────────────────────────────────────────

function useInstrumentActivity(instrument: string) {
  return useQuery({
    queryKey: activityKey(instrument),
    queryFn: () => paperApi.instrumentActivity(instrument).then((r) => r.data),
    refetchInterval: 4000,
  })
}

// ── Positions ────────────────────────────────────────────────────────────────────

function TerminalPositions({ instrument }: { instrument: string }) {
  const { data } = useInstrumentActivity(instrument)
  const pos = data?.position

  if (!pos) {
    return <div className="px-3 py-2 text-xs text-text-dim">No open position</div>
  }

  const qtyNum = parseFloat(pos.quantity)
  return (
    <div className="px-3 py-2">
      <div className="flex items-center justify-between text-xs">
        <span className="font-mono text-text">{pos.instrument_id}</span>
        <span className={cn('font-mono', qtyNum >= 0 ? 'text-green-400' : 'text-red-400')}>
          {pos.quantity}
        </span>
        <span className="text-text-muted">@ {pos.average_entry_price}</span>
      </div>
    </div>
  )
}

// ── Working orders ─────────────────────────────────────────────────────────────

function WorkingOrders({ instrument }: { instrument: string }) {
  const { data } = useInstrumentActivity(instrument)
  const orders = (data?.orders ?? []).filter(
    (o) => o.status === 'new' || o.status === 'partially_filled',
  )

  if (orders.length === 0) {
    return <div className="px-3 py-2 text-xs text-text-dim">No working orders</div>
  }

  return (
    <div className="px-3 py-2 space-y-1">
      {orders.map((o) => (
        <div key={o.order_id} className="flex items-center justify-between text-xs">
          <span
            className={cn(
              'font-semibold uppercase',
              o.side === 'buy' ? 'text-green-400' : 'text-red-400',
            )}
          >
            {o.side}
          </span>
          <span className="font-mono text-text">{o.qty}</span>
          <span className="text-text-muted">{o.order_type}</span>
          <span className="text-text-dim">{o.status}</span>
        </div>
      ))}
    </div>
  )
}

// ── Fills ──────────────────────────────────────────────────────────────────────

function Fills({ instrument }: { instrument: string }) {
  const { data } = useInstrumentActivity(instrument)
  const fills = (data?.orders ?? []).filter(
    (o) => o.status === 'filled' || o.status === 'partially_filled',
  )

  if (fills.length === 0) {
    return <div className="px-3 py-2 text-xs text-text-dim">No fills</div>
  }

  return (
    <div className="px-3 py-2 space-y-1 max-h-40 overflow-y-auto">
      {fills.slice(0, 20).map((o) => (
        <div key={o.order_id} className="flex items-center justify-between text-xs">
          <span
            className={cn(
              'font-semibold uppercase',
              o.side === 'buy' ? 'text-green-400' : 'text-red-400',
            )}
          >
            {o.side}
          </span>
          <span className="font-mono text-text">{o.filled_qty}</span>
          <span className="text-text-muted">
            {o.avg_fill_price ? `@ ${parseFloat(o.avg_fill_price).toLocaleString()}` : '—'}
          </span>
          <span className="text-text-dim">
            {new Date(o.updated_at).toLocaleTimeString()}
          </span>
        </div>
      ))}
    </div>
  )
}

// ── Collapsible section ────────────────────────────────────────────────────────

function CollapsibleSection({
  title,
  defaultOpen = true,
  children,
}: {
  title: string
  defaultOpen?: boolean
  children: React.ReactNode
}) {
  const [open, setOpen] = useState(defaultOpen)
  return (
    <>
      <button
        onClick={() => setOpen((v) => !v)}
        className="flex items-center gap-1.5 w-full px-3 py-1.5 border-t border-b border-border bg-surface-2 hover:bg-border transition-colors text-left"
      >
        {open
          ? <ChevronDown className="h-3 w-3 text-text-dim shrink-0" />
          : <ChevronRightIcon className="h-3 w-3 text-text-dim shrink-0" />}
        <span className="text-xs font-semibold uppercase tracking-wider text-text-dim">
          {title}
        </span>
      </button>
      {open && children}
    </>
  )
}

// ── TerminalPanel ──────────────────────────────────────────────────────────────

interface TerminalPanelProps {
  instrument: string
  assetClass?: TerminalAssetClass
}

export function TerminalPanel({
  instrument,
  assetClass = 'crypto_spot_cex',
}: TerminalPanelProps) {
  return (
    <div className="flex flex-col h-full">
      <CollapsibleSection title="Order">
        <OrderTicket instrument={instrument} assetClass={assetClass} />
      </CollapsibleSection>

      <CollapsibleSection title="Positions">
        <TerminalPositions instrument={instrument} />
      </CollapsibleSection>

      <CollapsibleSection title="Working orders">
        <WorkingOrders instrument={instrument} />
      </CollapsibleSection>

      <CollapsibleSection title="Fills">
        <Fills instrument={instrument} />
      </CollapsibleSection>
    </div>
  )
}
