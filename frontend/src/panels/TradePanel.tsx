import { useState } from 'react'
import { useMutation } from '@tanstack/react-query'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
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
import { toast } from '@/hooks/useToast'
import { ordersApi } from '@/api/rest'

interface TradePanelProps {
  instrument: string
}

type Side = 'buy' | 'sell'
type OrderType = 'market' | 'limit'

export function TradePanel({ instrument }: TradePanelProps) {
  const [side, setSide] = useState<Side>('buy')
  const [orderType, setOrderType] = useState<OrderType>('market')
  const [qty, setQty] = useState('')
  const [limitPrice, setLimitPrice] = useState('')
  const [lastError, setLastError] = useState<string | null>(null)

  const orderMut = useMutation({
    mutationFn: () =>
      ordersApi.place({
        instrument_id: instrument,
        side,
        order_type: orderType,
        qty,
        limit_price: orderType !== 'market' && limitPrice ? limitPrice : undefined,
      }),
    onSuccess: () => {
      toast({ title: 'Order submitted', variant: 'success' })
      setQty('')
      setLimitPrice('')
      setLastError(null)
    },
    onError: (err: unknown) => {
      const msg =
        (err as { response?: { data?: { message?: string } } })?.response?.data?.message ??
        'Order rejected'
      setLastError(msg)
      toast({ title: msg, variant: 'error' })
    },
  })

  return (
    <Card>
      <CardHeader className="pb-2">
        <CardTitle className="text-sm">Manual Trade — {instrument}</CardTitle>
      </CardHeader>
      <CardContent className="space-y-3">
        {/* Side toggle */}
        <div className="flex gap-2">
          <Button
            size="sm"
            variant={side === 'buy' ? 'success' : 'outline'}
            className="flex-1"
            onClick={() => setSide('buy')}
          >
            Buy
          </Button>
          <Button
            size="sm"
            variant={side === 'sell' ? 'destructive' : 'outline'}
            className="flex-1"
            onClick={() => setSide('sell')}
          >
            Sell
          </Button>
        </div>

        {/* Order type */}
        <div className="space-y-1.5">
          <Label>Order Type</Label>
          <Select value={orderType} onValueChange={(v) => setOrderType(v as OrderType)}>
            <SelectTrigger>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="market">Market</SelectItem>
              <SelectItem value="limit">Limit</SelectItem>
            </SelectContent>
          </Select>
        </div>

        {/* Quantity */}
        <div className="space-y-1.5">
          <Label>Quantity</Label>
          <Input
            type="number"
            min="0"
            step="any"
            placeholder="0.00"
            value={qty}
            onChange={(e) => setQty(e.target.value)}
          />
        </div>

        {/* Limit price */}
        {orderType === 'limit' && (
          <div className="space-y-1.5">
            <Label>Limit Price</Label>
            <Input
              type="number"
              min="0"
              step="any"
              placeholder="0.00"
              value={limitPrice}
              onChange={(e) => setLimitPrice(e.target.value)}
            />
          </div>
        )}

        {/* Rejection message */}
        {lastError && (
          <p className="text-xs text-red-400 rounded bg-red-950/30 px-2 py-1">{lastError}</p>
        )}

        <Button
          className="w-full"
          variant={side === 'buy' ? 'success' : 'destructive'}
          disabled={!qty || orderMut.isPending}
          onClick={() => orderMut.mutate()}
        >
          {orderMut.isPending
            ? 'Submitting…'
            : `${side.toUpperCase()} ${instrument}`}
        </Button>
      </CardContent>
    </Card>
  )
}
