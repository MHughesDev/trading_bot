import { useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Loader2 } from 'lucide-react'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
import { backtestsApi, strategyPickerApi, type CreateBacktestRequest } from '@/api/backtests'
import { toast } from '@/hooks/useToast'

const ASSET_CLASSES = [
  { value: 'crypto_spot_cex', label: 'Crypto (spot, CEX)' },
  { value: 'perpetual_swap', label: 'Crypto perpetual swap' },
  { value: 'equity', label: 'Equity' },
  { value: 'etf', label: 'ETF' },
  { value: 'fx', label: 'FX' },
  { value: 'futures_expiring', label: 'Futures' },
  { value: 'option', label: 'Option' },
]

const TIMEFRAMES = ['1m', '5m', '15m', '1h', '4h', '1d']

const DURATIONS = [
  { value: '7', label: '1 week' },
  { value: '30', label: '1 month' },
  { value: '90', label: '3 months' },
  { value: '180', label: '6 months' },
  { value: '365', label: '1 year' },
  { value: '730', label: '2 years' },
]

function isoDaysAgo(days: number): string {
  const d = new Date()
  d.setUTCDate(d.getUTCDate() - days)
  d.setUTCHours(0, 0, 0, 0)
  return d.toISOString()
}

export function CreateBacktestDialog({
  open,
  onOpenChange,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
}) {
  const qc = useQueryClient()

  const [strategyRef, setStrategyRef] = useState('')
  const [name, setName] = useState('')
  const [assetClass, setAssetClass] = useState('crypto_spot_cex')
  const [instrument, setInstrument] = useState('BTC-USDT')
  const [timeframe, setTimeframe] = useState('1h')
  const [durationDays, setDurationDays] = useState('90')
  const [balance, setBalance] = useState('100000')
  const [quote, setQuote] = useState('USD')
  const [autoCollect, setAutoCollect] = useState(true)

  const { data: strategies } = useQuery({
    queryKey: ['backtest-strategy-picker'],
    queryFn: () => strategyPickerApi.list().then((r) => r.data.strategies),
    enabled: open,
  })

  const create = useMutation({
    mutationFn: (req: CreateBacktestRequest) => backtestsApi.create(req),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ['backtests'] })
      toast({ title: 'Backtest started', variant: 'success' })
      onOpenChange(false)
    },
    onError: (err: unknown) => {
      const msg =
        (err as { response?: { data?: { message?: string; error?: string } } })
          ?.response?.data?.message ??
        (err as { response?: { data?: { error?: string } } })?.response?.data
          ?.error ??
        'Failed to start backtest'
      toast({ title: 'Could not start backtest', description: msg, variant: 'error' })
    },
  })

  function submit() {
    if (!strategyRef) {
      toast({ title: 'Pick a strategy', variant: 'error' })
      return
    }
    if (!instrument.trim()) {
      toast({ title: 'Enter an instrument', variant: 'error' })
      return
    }
    const days = Number(durationDays)
    create.mutate({
      name: name.trim() || undefined,
      strategy_ref: strategyRef,
      instrument_id: instrument.trim(),
      // venue_id omitted — the backend picks a sensible default per asset class.
      asset_class: assetClass,
      timeframe,
      start: isoDaysAgo(days),
      end: isoDaysAgo(0),
      initial_balance: balance.trim() || '100000',
      quote_currency: quote.trim() || 'USD',
      auto_collect: autoCollect,
    })
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>New backtest</DialogTitle>
          <DialogDescription>
            Replay a strategy over historical data. Missing history is
            collected automatically before the run when auto-collect is on.
          </DialogDescription>
        </DialogHeader>

        <div className="grid grid-cols-2 gap-4">
          <div className="col-span-2 flex flex-col gap-1.5">
            <Label>Strategy</Label>
            <Select value={strategyRef} onValueChange={setStrategyRef}>
              <SelectTrigger>
                <SelectValue placeholder="Select a strategy…" />
              </SelectTrigger>
              <SelectContent>
                {(strategies ?? []).map((s) => (
                  <SelectItem key={s.id} value={s.id}>
                    {s.strategy_id}
                  </SelectItem>
                ))}
                {(strategies ?? []).length === 0 && (
                  <div className="px-3 py-2 text-sm text-text-dim">
                    No strategies yet — create one first.
                  </div>
                )}
              </SelectContent>
            </Select>
          </div>

          <div className="flex flex-col gap-1.5">
            <Label>Asset class</Label>
            <Select value={assetClass} onValueChange={setAssetClass}>
              <SelectTrigger>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {ASSET_CLASSES.map((a) => (
                  <SelectItem key={a.value} value={a.value}>
                    {a.label}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>

          <div className="flex flex-col gap-1.5">
            <Label>Instrument</Label>
            <Input
              value={instrument}
              onChange={(e) => setInstrument(e.target.value)}
              placeholder="BTC-USDT"
            />
          </div>

          <div className="flex flex-col gap-1.5">
            <Label>Timeframe</Label>
            <Select value={timeframe} onValueChange={setTimeframe}>
              <SelectTrigger>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {TIMEFRAMES.map((t) => (
                  <SelectItem key={t} value={t}>
                    {t}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>

          <div className="flex flex-col gap-1.5">
            <Label>Duration</Label>
            <Select value={durationDays} onValueChange={setDurationDays}>
              <SelectTrigger>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {DURATIONS.map((d) => (
                  <SelectItem key={d.value} value={d.value}>
                    {d.label}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>

          <div className="flex flex-col gap-1.5">
            <Label>Starting balance</Label>
            <Input
              value={balance}
              onChange={(e) => setBalance(e.target.value)}
              inputMode="decimal"
            />
          </div>

          <div className="flex flex-col gap-1.5">
            <Label>Quote currency</Label>
            <Input value={quote} onChange={(e) => setQuote(e.target.value)} />
          </div>

          <div className="col-span-2 flex flex-col gap-1.5">
            <Label>Name (optional)</Label>
            <Input
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="Defaults to strategy · instrument · timeframe"
            />
          </div>

          <label className="col-span-2 flex items-center gap-2 text-sm text-text-muted">
            <input
              type="checkbox"
              checked={autoCollect}
              onChange={(e) => setAutoCollect(e.target.checked)}
              className="h-4 w-4 accent-blue-500"
            />
            Automatically collect missing historical data before running
          </label>
        </div>

        <div className="mt-6 flex justify-end gap-2">
          <Button variant="ghost" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button onClick={submit} disabled={create.isPending}>
            {create.isPending && <Loader2 className="mr-2 h-4 w-4 animate-spin" />}
            Start backtest
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  )
}
