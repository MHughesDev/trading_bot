import { useEffect, useState } from 'react'
import { useParams, useNavigate } from 'react-router-dom'
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import {
  Star, Play, Square, RefreshCw, AlertTriangle, Zap, Search,
} from 'lucide-react'
import { assetApi, strategiesApi, tradeApi, universeApi } from '@/lib/api'
import { useWatchlistStore } from '@/store/watchlist'
import { toast } from '@/hooks/useToast'
import { OhlcvChart } from '@/components/charts/OhlcvChart'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { Badge } from '@/components/ui/badge'
import {
  Select, SelectContent, SelectItem, SelectTrigger, SelectValue,
} from '@/components/ui/select'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { cn } from '@/lib/utils'
import { subHours, subDays, format } from 'date-fns'

const INTERVALS = [
  { label: '1m', seconds: 60 },
  { label: '5m', seconds: 300 },
  { label: '1h', seconds: 3600 },
  { label: '1D', seconds: 86400 },
  { label: '1W', seconds: 604800 },
]

function lifecycleBadge(lc: string | undefined) {
  if (lc === 'active') return <Badge variant="active">active</Badge>
  if (lc === 'initialized_not_active') return <Badge variant="warning">initialized</Badge>
  return <Badge variant="inactive">uninitialized</Badge>
}

type UniverseRow = { canonical_symbol: string; name?: string | null }

function SymbolSearch({ onSelect }: { onSelect: (sym: string) => void }) {
  const [q, setQ] = useState('')
  const [debounced, setDebounced] = useState('')

  useEffect(() => {
    const t = setTimeout(() => setDebounced(q.trim()), 250)
    return () => clearTimeout(t)
  }, [q])

  const { data, isFetching } = useQuery({
    queryKey: ['universe-search', debounced],
    queryFn: () => universeApi.search(debounced).then((r) => r.data?.rows ?? []),
  })

  const results: UniverseRow[] = Array.isArray(data) ? data : []
  const trimmed = q.trim()
  const exactMatch = results.find((r) => r.canonical_symbol.toUpperCase() === trimmed.toUpperCase())
  const showNotFound = trimmed.length > 0 && !isFetching && results.length === 0

  return (
    <div className="space-y-3">
      <div className="relative max-w-md">
        <Search className="absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-text-dim" />
        <Input
          placeholder="Search by symbol or name…"
          value={q}
          onChange={(e) => setQ(e.target.value)}
          onKeyDown={(e) => { if (e.key === 'Enter' && exactMatch) onSelect(exactMatch.canonical_symbol) }}
          className="pl-9"
          autoFocus
        />
      </div>
      <div className="rounded-lg border border-border divide-y divide-border max-h-[28rem] overflow-auto">
        {isFetching && results.length === 0 ? (
          <div className="px-4 py-3 text-sm text-text-dim">Loading…</div>
        ) : showNotFound ? (
          <div className="px-4 py-3 text-sm text-text-dim">
            Not found — “{trimmed}” isn’t an available asset. Choose one from the list.
          </div>
        ) : results.length === 0 ? (
          <div className="px-4 py-3 text-sm text-text-dim">Start typing to search available assets…</div>
        ) : (
          results.map((r) => (
            <button
              key={r.canonical_symbol}
              onClick={() => onSelect(r.canonical_symbol)}
              className="flex w-full items-center justify-between gap-3 px-4 py-2.5 text-sm text-text-muted hover:bg-border hover:text-text transition-colors"
            >
              <span className="font-mono">{r.canonical_symbol}</span>
              {r.name && <span className="text-xs text-text-dim truncate">{r.name}</span>}
            </button>
          ))
        )}
      </div>
    </div>
  )
}

export function AssetPage() {
  const { symbol } = useParams<{ symbol: string }>()
  const navigate = useNavigate()
  const qc = useQueryClient()
  const { toggle, has } = useWatchlistStore()

  const [interval, setInterval] = useState(3600)
  const [orderSide, setOrderSide] = useState<'buy' | 'sell'>('buy')
  const [orderType, setOrderType] = useState('market')
  const [orderQty, setOrderQty] = useState('')
  const [orderLimit, setOrderLimit] = useState('')

  const now = new Date()
  const end = format(now, "yyyy-MM-dd'T'HH:mm:ss")

  const { data: lifecycle } = useQuery({
    queryKey: ['lifecycle', symbol],
    queryFn: () => symbol ? assetApi.lifecycle(symbol).then((r) => r.data) : null,
    enabled: !!symbol,
    refetchInterval: 5000,
  })

  const { data: bars = [], isFetching: barsLoading } = useQuery({
    queryKey: ['bars', symbol, interval],
    queryFn: () => symbol
      ? assetApi.chartBars(symbol, format(subDays(now, Math.ceil(interval > 3600 ? 365 : 7)), "yyyy-MM-dd'T'HH:mm:ss"), end, interval).then((r) => r.data?.bars ?? r.data ?? [])
      : [],
    enabled: !!symbol,
    refetchInterval: interval <= 60 ? 10000 : 60000,
  })

  const { data: markers = [] } = useQuery({
    queryKey: ['markers', symbol],
    queryFn: () => symbol ? assetApi.tradeMarkers(symbol, format(subHours(now, 168), "yyyy-MM-dd'T'HH:mm:ss"), end).then((r) => r.data?.markers ?? r.data ?? []) : [],
    enabled: !!symbol,
  })

  const { data: strategyData } = useQuery({
    queryKey: ['asset-strategy', symbol],
    queryFn: () => symbol ? assetApi.strategy(symbol).then((r) => r.data) : null,
    enabled: !!symbol,
  })

  const { data: execMode } = useQuery({
    queryKey: ['exec-mode', symbol],
    queryFn: () => symbol ? assetApi.executionMode(symbol).then((r) => r.data) : null,
    enabled: !!symbol,
  })

  const { data: strategies = [] } = useQuery({
    queryKey: ['strategies'],
    queryFn: () => strategiesApi.list().then((r) => r.data?.strategies ?? r.data ?? []),
  })

  const { data: modelManifest } = useQuery({
    queryKey: ['models', symbol],
    queryFn: () => symbol ? assetApi.models(symbol).then((r) => r.data) : null,
    enabled: !!symbol,
  })

  const initMut = useMutation({
    mutationFn: () => assetApi.init(symbol!),
    onSuccess: () => { toast({ title: 'Initialization started', variant: 'success' }); qc.invalidateQueries({ queryKey: ['lifecycle', symbol] }) },
    onError: () => toast({ title: 'Init failed', variant: 'error' }),
  })

  const startMut = useMutation({
    mutationFn: () => assetApi.start(symbol!),
    onSuccess: () => { toast({ title: `${symbol} started`, variant: 'success' }); qc.invalidateQueries({ queryKey: ['lifecycle', symbol] }) },
    onError: () => toast({ title: 'Start failed', variant: 'error' }),
  })

  const stopMut = useMutation({
    mutationFn: () => assetApi.stop(symbol!),
    onSuccess: () => { toast({ title: `${symbol} stopped` }); qc.invalidateQueries({ queryKey: ['lifecycle', symbol] }) },
    onError: () => toast({ title: 'Stop failed', variant: 'error' }),
  })

  const setStrategyMut = useMutation({
    mutationFn: (id: string) => assetApi.setStrategy(symbol!, id),
    onSuccess: () => { toast({ title: 'Strategy updated', variant: 'success' }); qc.invalidateQueries({ queryKey: ['asset-strategy', symbol] }) },
    onError: () => toast({ title: 'Strategy update failed', variant: 'error' }),
  })

  const setExecModeMut = useMutation({
    mutationFn: (mode: string) => assetApi.setExecutionMode(symbol!, mode),
    onSuccess: () => { toast({ title: 'Execution mode updated', variant: 'success' }); qc.invalidateQueries({ queryKey: ['exec-mode', symbol] }) },
    onError: () => toast({ title: 'Update failed', variant: 'error' }),
  })

  const orderMut = useMutation({
    mutationFn: () => tradeApi.order({ symbol, side: orderSide, qty: parseFloat(orderQty), order_type: orderType, limit_price: orderLimit ? parseFloat(orderLimit) : undefined }),
    onSuccess: () => { toast({ title: 'Order submitted', variant: 'success' }); setOrderQty(''); setOrderLimit('') },
    onError: () => toast({ title: 'Order failed', variant: 'error' }),
  })

  const flattenMut = useMutation({
    mutationFn: () => tradeApi.flatten(symbol!),
    onSuccess: () => toast({ title: `${symbol} flattened`, variant: 'success' }),
    onError: () => toast({ title: 'Flatten failed', variant: 'error' }),
  })

  const lc = lifecycle?.lifecycle ?? lifecycle?.state
  const watchlisted = symbol ? has(symbol) : false

  if (!symbol) {
    return (
      <div className="p-6 space-y-4 max-w-2xl">
        <h1 className="text-xl font-semibold text-text">Find an Asset</h1>
        <p className="text-text-muted text-sm">Search by symbol or name to open its asset page.</p>
        <SymbolSearch onSelect={(sym) => navigate(`/asset/${sym}`)} />
      </div>
    )
  }

  return (
    <div className="p-6 space-y-4">
      {/* Header */}
      <div className="flex flex-wrap items-center gap-3">
        <h1 className="text-xl font-semibold font-mono text-text">{symbol}</h1>
        {lifecycleBadge(lc)}
        <Button
          size="icon-sm"
          variant="ghost"
          onClick={() => toggle(symbol)}
          title={watchlisted ? 'Remove from watchlist' : 'Add to watchlist'}
        >
          <Star className={cn('h-4 w-4', watchlisted ? 'fill-amber-400 text-amber-400' : 'text-text-dim')} />
        </Button>

        <div className="ml-auto flex items-center gap-2">
          {lc === 'uninitialized' && (
            <Button size="sm" onClick={() => initMut.mutate()} disabled={initMut.isPending}>
              <RefreshCw className="h-4 w-4" />
              Initialize
            </Button>
          )}
          {lc === 'initialized_not_active' && (
            <Button size="sm" variant="success" onClick={() => startMut.mutate()} disabled={startMut.isPending}>
              <Play className="h-4 w-4" />
              Start
            </Button>
          )}
          {lc === 'active' && (
            <Button size="sm" variant="destructive" onClick={() => stopMut.mutate()} disabled={stopMut.isPending}>
              <Square className="h-4 w-4" />
              Stop
            </Button>
          )}
          <Button size="sm" variant="outline" onClick={() => flattenMut.mutate()} disabled={flattenMut.isPending}>
            <AlertTriangle className="h-4 w-4" />
            Flatten
          </Button>
        </div>
      </div>

      {/* Chart */}
      <Card>
        <CardHeader className="pb-2 flex-row items-center justify-between">
          <CardTitle className="text-sm">Price Chart</CardTitle>
          <div className="flex gap-1">
            {INTERVALS.map((iv) => (
              <Button
                key={iv.seconds}
                size="sm"
                variant={interval === iv.seconds ? 'secondary' : 'ghost'}
                className="h-6 px-2 text-xs"
                onClick={() => setInterval(iv.seconds)}
              >
                {iv.label}
              </Button>
            ))}
          </div>
        </CardHeader>
        <CardContent className="p-0 pb-2">
          {barsLoading && bars.length === 0 ? (
            <div className="flex h-72 items-center justify-center text-text-dim text-sm">Loading chart…</div>
          ) : bars.length === 0 ? (
            <div className="flex h-72 items-center justify-center text-text-dim text-sm">No data — initialize the asset first.</div>
          ) : (
            <OhlcvChart bars={bars} markers={markers} height={300} />
          )}
        </CardContent>
      </Card>

      <div className="grid grid-cols-1 lg:grid-cols-3 gap-4">
        {/* Controls */}
        <Card>
          <CardHeader className="pb-2">
            <CardTitle className="text-sm flex items-center gap-2">
              <Zap className="h-4 w-4 text-blue-400" />
              Controls
            </CardTitle>
          </CardHeader>
          <CardContent className="space-y-4">
            <div className="space-y-1.5">
              <Label>Strategy</Label>
              <Select
                value={strategyData?.strategy_id ?? ''}
                onValueChange={(v) => setStrategyMut.mutate(v)}
              >
                <SelectTrigger>
                  <SelectValue placeholder="Select strategy…" />
                </SelectTrigger>
                <SelectContent>
                  {(Array.isArray(strategies) ? strategies : []).map((s: { id: string; name?: string }) => (
                    <SelectItem key={s.id} value={s.id}>{s.name ?? s.id}</SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>

            <div className="space-y-1.5">
              <Label>Execution Mode</Label>
              <Select
                value={execMode?.mode ?? 'paper'}
                onValueChange={(v) => setExecModeMut.mutate(v)}
              >
                <SelectTrigger>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="paper">Paper</SelectItem>
                  <SelectItem value="live">Live</SelectItem>
                </SelectContent>
              </Select>
            </div>
          </CardContent>
        </Card>

        {/* Manual trade */}
        <Card>
          <CardHeader className="pb-2">
            <CardTitle className="text-sm">Manual Trade</CardTitle>
          </CardHeader>
          <CardContent className="space-y-3">
            <div className="flex gap-2">
              <Button
                size="sm"
                variant={orderSide === 'buy' ? 'success' : 'outline'}
                className="flex-1"
                onClick={() => setOrderSide('buy')}
              >Buy</Button>
              <Button
                size="sm"
                variant={orderSide === 'sell' ? 'destructive' : 'outline'}
                className="flex-1"
                onClick={() => setOrderSide('sell')}
              >Sell</Button>
            </div>
            <div className="space-y-1.5">
              <Label>Order Type</Label>
              <Select value={orderType} onValueChange={setOrderType}>
                <SelectTrigger><SelectValue /></SelectTrigger>
                <SelectContent>
                  <SelectItem value="market">Market</SelectItem>
                  <SelectItem value="limit">Limit</SelectItem>
                  <SelectItem value="stop">Stop</SelectItem>
                </SelectContent>
              </Select>
            </div>
            <div className="space-y-1.5">
              <Label>Quantity</Label>
              <Input
                type="number"
                min="0"
                step="any"
                placeholder="0.00"
                value={orderQty}
                onChange={(e) => setOrderQty(e.target.value)}
              />
            </div>
            {orderType !== 'market' && (
              <div className="space-y-1.5">
                <Label>{orderType === 'limit' ? 'Limit Price' : 'Stop Price'}</Label>
                <Input
                  type="number"
                  min="0"
                  step="any"
                  placeholder="0.00"
                  value={orderLimit}
                  onChange={(e) => setOrderLimit(e.target.value)}
                />
              </div>
            )}
            <Button
              className="w-full"
              variant={orderSide === 'buy' ? 'success' : 'destructive'}
              disabled={!orderQty || orderMut.isPending}
              onClick={() => orderMut.mutate()}
            >
              {orderMut.isPending ? 'Submitting…' : `${orderSide.toUpperCase()} ${symbol}`}
            </Button>
          </CardContent>
        </Card>

        {/* Model manifest */}
        <Card>
          <CardHeader className="pb-2">
            <CardTitle className="text-sm">Model Manifest</CardTitle>
          </CardHeader>
          <CardContent>
            {!modelManifest ? (
              <p className="text-sm text-text-dim">No model data.</p>
            ) : (
              <div className="space-y-1 text-sm">
                {Object.entries(modelManifest).map(([k, v]) => (
                  <div key={k} className="flex justify-between text-xs">
                    <span className="text-text-muted font-mono">{k}</span>
                    <span className="text-text-dim font-mono truncate max-w-24" title={String(v)}>{String(v)}</span>
                  </div>
                ))}
              </div>
            )}
          </CardContent>
        </Card>
      </div>
    </div>
  )
}
