import { useEffect, useMemo, useState } from 'react'
import { useMutation, useQueryClient } from '@tanstack/react-query'
import { AlertTriangle, Info } from 'lucide-react'
import { tradeApi } from '@/lib/api'
import { useModeStore } from '@/store/mode'
import { usePrefs } from '@/store/prefs'
import { assetClassInfo, type PlatformAssetClass } from '@/lib/assetClass'
import { DASH, money, pct, price as fmtPrice, qty as fmtQty, instrumentMeta, signedPct } from '@/lib/format'
import { Panel, PanelBody, PanelHeader } from '@/components/primitives/Panel'
import { Button } from '@/components/primitives/Button'
import { Badge, KeyValue } from '@/components/primitives/Badge'
import { Label } from '@/components/primitives/Num'
import { Field, NumberField } from '@/components/primitives/Field'
import { PercentRow, Segmented } from '@/components/primitives/Segmented'
import { Select } from '@/components/primitives/Overlay'
import { ConfirmDialog } from '@/components/primitives/Modal'
import { useToast } from '@/hooks/useToast'
import { cn } from '@/lib/utils'

/* =============================================================================
   Spec §3.13 — order ticket. Fixed anatomy, top to bottom:
     1  side segmented (buy/sell), full width, tinted from the DIRECTION family
     2  order-type segmented — Market · Limit · Stop · Bracket
     3  price field (hidden for Market)
     4  size field + percent row + unit toggle
     5  time in force · leverage
     6  cost summary in a sunken well
     7  attached bracket block with an R:R badge and risk in BOTH ccy and % equity
     8  submit button whose label restates side, size and notional
     9  environment reminder line

   MUST: every derived figure recomputes on every input change within one frame;
   submit is disabled with an inline, field-adjacent reason when invalid.
   `--bg-accent` never touches buy/sell (spec §2.1.6).
   ============================================================================= */

export type Side = 'buy' | 'sell'
export type OrderType = 'market' | 'limit' | 'stop' | 'bracket'

const TIF_OPTIONS = [
  { value: 'gtc', label: 'GTC — good till cancelled', text: 'GTC' },
  { value: 'day', label: 'DAY — expires at session close', text: 'DAY' },
  { value: 'ioc', label: 'IOC — immediate or cancel', text: 'IOC' },
  { value: 'fok', label: 'FOK — fill or kill', text: 'FOK' },
] as const

const LEVERAGE_OPTIONS = ['1', '2', '3', '5', '10', '20'] as const

/** Maker fee used for the estimate until the venue publishes the real schedule. */
const EST_FEE_RATE = 0.0002

export interface OrderTicketProps {
  instrument: string
  assetClass?: PlatformAssetClass
  lastPrice?: number | null
  bid?: number | null
  ask?: number | null
  buyingPower?: number
  equity?: number
  /** A price clicked in the order book flows in here. */
  presetPrice?: number | null
  onPresetConsumed?: () => void
  className?: string
}

export function OrderTicket({
  instrument,
  assetClass = 'crypto_spot_cex',
  lastPrice = null,
  bid = null,
  ask = null,
  buyingPower = 0,
  equity = 0,
  presetPrice,
  onPresetConsumed,
  className,
}: OrderTicketProps) {
  const { mode } = useModeStore()
  const live = mode === 'LIVE'
  const meta = instrumentMeta(instrument)
  const info = assetClassInfo(assetClass)
  const qc = useQueryClient()
  const toast = useToast()

  const prefs = usePrefs()
  const [side, setSide] = useState<Side>('buy')
  const [type, setType] = useState<OrderType>(
    prefs.attachBracketByDefault && info.clob ? 'bracket' : prefs.defaultOrderType,
  )
  const [limitPrice, setLimitPrice] = useState('')
  const [stopPrice, setStopPrice] = useState('')
  const [size, setSize] = useState('')
  const [sizePct, setSizePct] = useState<number | null>(null)
  const [unit, setUnit] = useState<'base' | 'quote'>(prefs.defaultSizeUnit)
  const [tif, setTif] = useState<string>(prefs.defaultTif)
  const [leverage, setLeverage] = useState<string>(String(prefs.defaultLeverage))
  const [takeProfit, setTakeProfit] = useState('')
  const [stopLoss, setStopLoss] = useState('')
  const [confirming, setConfirming] = useState(false)

  // Seed the limit price from the touch when the instrument or side changes.
  useEffect(() => {
    const touch = side === 'buy' ? (bid ?? lastPrice) : (ask ?? lastPrice)
    if (touch != null && limitPrice === '') setLimitPrice(touch.toFixed(meta.priceDp))
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [instrument])

  // A price clicked in the order book writes straight into the ticket (§3.14).
  useEffect(() => {
    if (presetPrice == null) return
    if (type === 'market') setType('limit')
    setLimitPrice(presetPrice.toFixed(meta.priceDp))
    onPresetConsumed?.()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [presetPrice])

  const refPrice =
    type === 'market'
      ? (side === 'buy' ? (ask ?? lastPrice) : (bid ?? lastPrice))
      : type === 'stop'
        ? Number(stopPrice) || lastPrice
        : Number(limitPrice) || lastPrice

  const derived = useMemo(() => {
    const px = Number(refPrice) || 0
    const rawSize = Number(size) || 0
    const baseQty = unit === 'base' ? rawSize : px > 0 ? rawSize / px : 0
    const notional = baseQty * px
    const lev = Number(leverage) || 1
    const fee = notional * EST_FEE_RATE
    const margin = lev > 0 ? notional / lev : notional
    // A long liquidates below entry, a short above it, at roughly 1/leverage away.
    const liq =
      lev > 1 && px > 0
        ? side === 'buy'
          ? px * (1 - 1 / lev + 0.005)
          : px * (1 + 1 / lev - 0.005)
        : null
    const bpAfter = buyingPower - margin - fee

    const tp = Number(takeProfit) || 0
    const sl = Number(stopLoss) || 0
    const riskPerUnit = sl > 0 && px > 0 ? Math.abs(px - sl) : 0
    const rewardPerUnit = tp > 0 && px > 0 ? Math.abs(tp - px) : 0
    const riskCcy = riskPerUnit * baseQty
    const rr = riskPerUnit > 0 ? rewardPerUnit / riskPerUnit : null
    const riskPctEquity = equity > 0 ? (riskCcy / equity) * 100 : null

    return { px, baseQty, notional, fee, margin, liq, bpAfter, riskCcy, rr, riskPctEquity, tp, sl }
  }, [refPrice, size, unit, leverage, side, buyingPower, equity, takeProfit, stopLoss])

  /* --- validation: every reason is attached to the field that caused it ----- */
  const errors = useMemo(() => {
    const e: Partial<Record<'size' | 'limit' | 'stop' | 'tp' | 'sl' | 'form', string>> = {}
    if (!instrument) e.form = 'Choose an instrument first.'
    if (!size || derived.baseQty <= 0) e.size = 'Enter a size greater than zero.'
    if (type === 'limit' && !(Number(limitPrice) > 0)) e.limit = 'A limit order needs a price.'
    if (type === 'bracket' && !(Number(limitPrice) > 0)) e.limit = 'A bracket order needs an entry price.'
    if (type === 'stop' && !(Number(stopPrice) > 0)) e.stop = 'A stop order needs a trigger price.'
    if (derived.bpAfter < 0) e.size = `Exceeds buying power by ${money(Math.abs(derived.bpAfter))}.`
    if (type === 'bracket') {
      if (derived.tp <= 0) e.tp = 'Set a take-profit price.'
      if (derived.sl <= 0) e.sl = 'Set a stop-loss price.'
      if (side === 'buy' && derived.tp > 0 && derived.tp <= derived.px) e.tp = 'Take profit must be above the entry for a long.'
      if (side === 'buy' && derived.sl > 0 && derived.sl >= derived.px) e.sl = 'Stop loss must be below the entry for a long.'
      if (side === 'sell' && derived.tp > 0 && derived.tp >= derived.px) e.tp = 'Take profit must be below the entry for a short.'
      if (side === 'sell' && derived.sl > 0 && derived.sl <= derived.px) e.sl = 'Stop loss must be above the entry for a short.'
    }
    return e
  }, [instrument, size, derived, type, limitPrice, stopPrice, side])

  const firstError = errors.form ?? errors.size ?? errors.limit ?? errors.stop ?? errors.tp ?? errors.sl ?? null
  const valid = !firstError

  const submit = useMutation({
    mutationFn: async () => {
      const body: Record<string, unknown> = {
        instrument_id: instrument,
        side,
        order_type: type === 'bracket' ? 'limit' : type === 'stop' ? 'stop_limit' : type,
        qty: String(derived.baseQty),
        execution_mode: mode,
      }
      if (type === 'limit' || type === 'bracket') body.limit_price = limitPrice
      if (type === 'stop') body.stop_price = stopPrice
      if (type === 'bracket') {
        body.tp_price = takeProfit
        body.sl_price = stopLoss
      }
      if (!info.clob) body.tif = tif.toUpperCase()
      if (info.clob && Number(leverage) > 1) body.leverage = leverage
      return tradeApi.order(body)
    },
    onSuccess: () => {
      toast({
        title: 'Order submitted',
        description: `${side === 'buy' ? 'Buy' : 'Sell'} ${fmtQty(instrument, derived.baseQty)} ${instrument} · ${money(derived.notional)}`,
      })
      void qc.invalidateQueries({ queryKey: ['paper-activity', instrument] })
      void qc.invalidateQueries({ queryKey: ['dashboard-rollup'] })
      if (prefs.clearTicketAfterSubmit) {
        setSize('')
        setSizePct(null)
      }
    },
    onError: (err: unknown) => {
      const e = err as { response?: { data?: { message?: string; error?: string } } }
      toast({
        title: 'Order rejected',
        description: e.response?.data?.message ?? e.response?.data?.error ?? 'The risk gate refused this order.',
        variant: 'error',
      })
    },
  })

  function applyPct(p: number) {
    setSizePct(p)
    const px = derived.px
    if (!px) return
    const spend = (buyingPower * p) / 100
    const lev = Number(leverage) || 1
    const baseQty = (spend * lev) / px
    setSize(unit === 'base' ? baseQty.toFixed(meta.qtyDp) : spend.toFixed(2))
  }

  // An unusually large order earns one more step even on paper (spec §5.5).
  const isLargeOrder =
    (equity > 0 && derived.notional > equity * (prefs.largeOrderWarnPctOfEquity / 100)) ||
    (prefs.maxOrderNotional > 0 && derived.notional > prefs.maxOrderNotional)

  const submitLabel = `${live ? 'LIVE — ' : ''}${side === 'buy' ? 'Buy' : 'Sell'} ${
    derived.baseQty > 0 ? fmtQty(instrument, derived.baseQty) : fmtQty(instrument, 0)
  } ${instrument.split('-')[0]} · ${money(derived.notional)}`

  return (
    <>
      <Panel
        className={className}
        style={live ? { borderTop: '2px solid var(--line-accent)' } : undefined}
      >
        <PanelHeader title="Order ticket" actions={<Badge tone="neutral">{instrument}</Badge>} />
        <PanelBody style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
          {/* 1 — side */}
          <Segmented
            ariaLabel="Order side"
            variant="buysell"
            value={side}
            onChange={setSide}
            options={[
              { value: 'buy', label: 'Buy / Long', side: 'buy' },
              { value: 'sell', label: 'Sell / Short', side: 'sell' },
            ]}
          />

          {/* 2 — order type */}
          <Segmented
            ariaLabel="Order type"
            wide
            value={type}
            onChange={(t) => {
              setType(t)
              if (t !== 'market' && !limitPrice && lastPrice) setLimitPrice(lastPrice.toFixed(meta.priceDp))
              if (t === 'bracket' && lastPrice) {
                const base = Number(limitPrice) || lastPrice
                const dir = side === 'buy' ? 1 : -1
                if (!takeProfit) setTakeProfit((base * (1 + (dir * prefs.defaultTakeProfitPct) / 100)).toFixed(meta.priceDp))
                if (!stopLoss) setStopLoss((base * (1 - (dir * prefs.defaultStopPct) / 100)).toFixed(meta.priceDp))
              }
            }}
            options={[
              { value: 'market', label: 'Market' },
              { value: 'limit', label: 'Limit' },
              { value: 'stop', label: 'Stop', disabled: !info.clob, title: info.clob ? undefined : `${info.label} has no stop-order support.` },
              { value: 'bracket', label: 'Bracket', disabled: !info.clob, title: info.clob ? undefined : `${info.label} cannot attach brackets.` },
            ]}
          />

          {/* 3 — price */}
          {type !== 'market' && (
            <NumberField
              label={type === 'stop' ? 'Stop trigger' : type === 'bracket' ? 'Entry price' : 'Limit price'}
              labelAside={
                lastPrice != null ? (
                  <button
                    type="button"
                    className="lbl acc"
                    style={{ background: 'none', border: 0, cursor: 'pointer' }}
                    onClick={() => (type === 'stop' ? setStopPrice(lastPrice.toFixed(meta.priceDp)) : setLimitPrice(lastPrice.toFixed(meta.priceDp)))}
                  >
                    Use last
                  </button>
                ) : undefined
              }
              unit={meta.quote ?? 'USD'}
              dp={meta.priceDp}
              step={Math.pow(10, -meta.priceDp)}
              min={0}
              value={type === 'stop' ? stopPrice : limitPrice}
              onValueChange={type === 'stop' ? setStopPrice : setLimitPrice}
              error={type === 'stop' ? errors.stop ?? null : errors.limit ?? null}
            />
          )}

          {/* 4 — size */}
          <Field
            label="Size"
            labelAside={
              <Segmented
                ariaLabel="Size unit"
                value={unit}
                onChange={setUnit}
                options={[
                  { value: 'base', label: instrument.split('-')[0] || 'Base' },
                  { value: 'quote', label: meta.quote ?? 'USD' },
                ]}
              />
            }
            error={errors.size ?? null}
          >
            <div className="input">
              <input
                className="n"
                inputMode="decimal"
                value={size}
                placeholder="0.000"
                aria-label="Order size"
                onChange={(e) => {
                  setSize(e.target.value.replace(/[,\s_]/g, ''))
                  setSizePct(null)
                }}
              />
              <span className="unit">{unit === 'base' ? instrument.split('-')[0] : (meta.quote ?? 'USD')}</span>
            </div>
          </Field>

          <PercentRow value={sizePct} onChange={applyPct} />

          {/* 5 — TIF / leverage */}
          <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 'var(--s-2)' }}>
            <Field label="Time in force">
              <Select
                ariaLabel="Time in force"
                small
                value={tif}
                onChange={setTif}
                options={TIF_OPTIONS.map((o) => ({ value: o.value, label: o.label, text: o.text }))}
              />
            </Field>
            <Field label="Leverage">
              <Select
                ariaLabel="Leverage"
                small
                disabled={!info.clob}
                value={leverage}
                onChange={setLeverage}
                options={LEVERAGE_OPTIONS.map((l) => ({ value: l, label: `${l}×`, text: `${l}×` }))}
              />
            </Field>
          </div>

          {/* 6 — cost summary */}
          <div className="well">
            <KeyValue k="Order value" v={money(derived.notional)} />
            <KeyValue k={`Est. fee (maker ${pct(EST_FEE_RATE * 100, 2)})`} v={money(derived.fee)} />
            <KeyValue k="Margin required" v={money(derived.margin)} />
            <KeyValue
              k="Est. liquidation"
              v={derived.liq ? fmtPrice(instrument, derived.liq) : DASH}
              tone={derived.liq ? 'neg' : undefined}
            />
            <div className="hr" style={{ margin: 'var(--s-2) 0' }} />
            <KeyValue
              k="Buying power after"
              v={money(derived.bpAfter)}
              tone={derived.bpAfter < 0 ? 'neg' : undefined}
            />
          </div>

          {/* 7 — attached bracket */}
          {type === 'bracket' && (
            <div
              className="well"
              style={{ background: 'var(--bg-pos-subtle)', borderColor: 'var(--line-pos)' }}
            >
              <div className="between" style={{ marginBottom: 'var(--s-2)' }}>
                <Label>Attached bracket</Label>
                <Badge tone={derived.rr && derived.rr >= 1.5 ? 'pos' : 'warn'}>
                  R : R {derived.rr ? derived.rr.toFixed(1) : DASH}
                </Badge>
              </div>
              <NumberField
                label="Take profit"
                labelAside={
                  derived.tp > 0 && derived.px > 0 ? (
                    <span className="num pos" style={{ fontSize: 'var(--t-11)' }}>
                      {signedPct(((derived.tp - derived.px) / derived.px) * 100 * (side === 'buy' ? 1 : -1))}
                    </span>
                  ) : undefined
                }
                dp={meta.priceDp}
                step={Math.pow(10, -meta.priceDp)}
                value={takeProfit}
                onValueChange={setTakeProfit}
                error={errors.tp ?? null}
              />
              <div style={{ height: 6 }} />
              <NumberField
                label="Stop loss"
                labelAside={
                  derived.sl > 0 && derived.px > 0 ? (
                    <span className="num neg" style={{ fontSize: 'var(--t-11)' }}>
                      {signedPct(((derived.sl - derived.px) / derived.px) * 100 * (side === 'buy' ? 1 : -1))}
                    </span>
                  ) : undefined
                }
                dp={meta.priceDp}
                step={Math.pow(10, -meta.priceDp)}
                value={stopLoss}
                onValueChange={setStopLoss}
                error={errors.sl ?? null}
              />
              <div className="hr" style={{ margin: 'var(--s-3) 0 var(--s-2)' }} />
              <KeyValue
                k="Risk on this trade"
                v={
                  <>
                    {money(derived.riskCcy)}
                    {derived.riskPctEquity !== null && (
                      <span className="mut"> {'·'} {pct(derived.riskPctEquity)} of equity</span>
                    )}
                  </>
                }
              />
            </div>
          )}

          <div className="spacer" />

          {/* inline blocking reason, stated once more next to the button */}
          {firstError && (
            <div className="field-err" role="status">
              <AlertTriangle size={11} aria-hidden />
              {firstError}
            </div>
          )}

          {/* 8 — submit */}
          <Button
            size="xl"
            block
            variant={side === 'buy' ? 'buy' : 'sell'}
            disabled={!valid}
            loading={submit.isPending}
            onClick={() => (live || prefs.confirmEveryOrder || isLargeOrder ? setConfirming(true) : submit.mutate())}
          >
            {submitLabel}
          </Button>

          {/* 9 — environment reminder */}
          {live ? (
            <div className="row" style={{ justifyContent: 'center', gap: 6 }}>
              <AlertTriangle size={11} className="warn" aria-hidden />
              <Label style={{ color: 'var(--fg-warn)' }}>Live account — this order uses real funds</Label>
            </div>
          ) : (
            <div className="row" style={{ justifyContent: 'center', gap: 6 }}>
              <Info size={11} className="mut" aria-hidden />
              <Label>Paper account — no live funds at risk</Label>
            </div>
          )}
        </PanelBody>
      </Panel>

      <ConfirmDialog
        open={confirming}
        onCancel={() => setConfirming(false)}
        onConfirm={() => {
          setConfirming(false)
          submit.mutate()
        }}
        title={live ? 'Submit a live order?' : isLargeOrder ? 'This is a large order' : 'Confirm this order'}
        confirmLabel={submitLabel}
        busy={submit.isPending}
        consequence={
          live ? (
          <>
            This places a <strong>real {side}</strong> order for{' '}
            <strong>{fmtQty(instrument, derived.baseQty)} {instrument}</strong> with a notional of{' '}
            <strong>{money(derived.notional)}</strong>
            {derived.margin > 0 && <> using {money(derived.margin)} of margin</>}. Live orders cannot be undone
            once filled.
          </>
          ) : (
            <>
              {isLargeOrder && (
                <>
                  This order&apos;s notional of <strong>{money(derived.notional)}</strong> is above your
                  warning threshold of {pct(prefs.largeOrderWarnPctOfEquity, 0)} of equity.{' '}
                </>
              )}
              It will {side} <strong>{fmtQty(instrument, derived.baseQty)} {instrument}</strong> on the
              paper account. No live funds are at risk.
            </>
          )
        }
        alternative={
          <Button
            size="sm"
            onClick={() => {
              setConfirming(false)
              useModeStore.getState().setMode('PAPER')
            }}
          >
            Place it on paper instead
          </Button>
        }
      />
    </>
  )
}

/** Compact ticket used inside a workspace panel, same engine, less chrome. */
export function InlineOrderTicket(props: OrderTicketProps) {
  return <OrderTicket {...props} className={cn('col-flex', props.className)} />
}
