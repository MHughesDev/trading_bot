import { useQuery } from '@tanstack/react-query'
import { useNavigate } from 'react-router-dom'
import { TrendingUp, TrendingDown, Activity, Layers } from 'lucide-react'
import { systemApi, portfolioApi } from '@/lib/api'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Badge } from '@/components/ui/badge'
import { PnlChart } from '@/components/charts/PnlChart'
import { cn, fmtCurrency, fmtPct, pnlClass } from '@/lib/utils'

function StatCard({ label, value, sub, up }: { label: string; value: string; sub?: string; up?: boolean }) {
  return (
    <Card>
      <CardContent className="p-4 space-y-1">
        <p className="text-xs font-semibold uppercase tracking-widest text-text-dim">{label}</p>
        <p className={cn('text-2xl font-mono font-semibold', up == null ? 'text-text' : up ? 'text-pnl-up' : 'text-pnl-down')}>
          {value}
        </p>
        {sub && <p className="text-xs text-text-muted">{sub}</p>}
      </CardContent>
    </Card>
  )
}

export function DashboardPage() {
  const navigate = useNavigate()

  const { data: status } = useQuery({
    queryKey: ['status'],
    queryFn: () => systemApi.status().then((r) => r.data),
    refetchInterval: 5000,
  })

  const { data: pnl } = useQuery({
    queryKey: ['pnl-summary'],
    queryFn: () => portfolioApi.pnlSummary().then((r) => r.data),
    refetchInterval: 10000,
  })

  const { data: positions } = useQuery({
    queryKey: ['positions'],
    queryFn: () => portfolioApi.positions().then((r) => r.data),
    refetchInterval: 10000,
  })

  const { data: pnlSeries } = useQuery({
    queryKey: ['pnl-series'],
    queryFn: () => portfolioApi.pnlSeries(3600).then((r) => r.data),
    refetchInterval: 60000,
  })

  const symbols: Record<string, { lifecycle?: string }> = status?.symbols ?? {}
  const activeAssets = Object.entries(symbols)
    .filter(([, v]) => v?.lifecycle === 'active')
    .map(([k]) => k)

  const positionList = Array.isArray(positions) ? positions : (positions?.positions ?? [])

  const dayPnl = pnl?.day?.realized_pnl ?? 0
  const totalPnl = pnl?.all?.realized_pnl ?? 0
  const winRate = pnl?.all?.win_rate ?? null
  const unrealizedPnl = positionList.reduce((s: number, p: { unrealized_pnl?: number }) => s + (p.unrealized_pnl ?? 0), 0)

  return (
    <div className="p-6 space-y-6">
      <div className="flex items-center justify-between">
        <h1 className="text-xl font-semibold text-text">Dashboard</h1>
        <Badge variant={activeAssets.length > 0 ? 'active' : 'inactive'}>
          {activeAssets.length} active
        </Badge>
      </div>

      {/* Stats row */}
      <div className="grid grid-cols-2 lg:grid-cols-4 gap-4">
        <StatCard label="Day P&L" value={fmtCurrency(dayPnl)} up={dayPnl >= 0} />
        <StatCard label="Unrealized" value={fmtCurrency(unrealizedPnl)} up={unrealizedPnl >= 0} />
        <StatCard label="Total P&L" value={fmtCurrency(totalPnl)} up={totalPnl >= 0} />
        <StatCard
          label="Win Rate"
          value={winRate != null ? fmtPct(winRate * 100, 1) : '—'}
          sub="realized trades"
        />
      </div>

      {/* P&L chart */}
      <Card>
        <CardHeader className="pb-2">
          <CardTitle className="text-sm flex items-center gap-2">
            <TrendingUp className="h-4 w-4 text-blue-400" />
            Realized P&L (hourly)
          </CardTitle>
        </CardHeader>
        <CardContent className="p-0">
          <PnlChart data={pnlSeries} />
        </CardContent>
      </Card>

      <div className="grid grid-cols-1 lg:grid-cols-2 gap-4">
        {/* Active assets */}
        <Card>
          <CardHeader className="pb-2">
            <CardTitle className="text-sm flex items-center gap-2">
              <Activity className="h-4 w-4 text-emerald-400" />
              Active Assets
            </CardTitle>
          </CardHeader>
          <CardContent>
            {activeAssets.length === 0 ? (
              <p className="text-sm text-text-dim">No active assets. Navigate to an asset to start one.</p>
            ) : (
              <div className="space-y-1">
                {activeAssets.map((sym) => (
                  <button
                    key={sym}
                    onClick={() => navigate(`/asset/${sym}`)}
                    className="flex w-full items-center justify-between rounded-md px-3 py-2 text-sm hover:bg-surface-2 transition-colors"
                  >
                    <span className="font-mono text-text">{sym}</span>
                    <Badge variant="active">active</Badge>
                  </button>
                ))}
              </div>
            )}
          </CardContent>
        </Card>

        {/* Holdings */}
        <Card>
          <CardHeader className="pb-2">
            <CardTitle className="text-sm flex items-center gap-2">
              <Layers className="h-4 w-4 text-blue-400" />
              Open Positions
            </CardTitle>
          </CardHeader>
          <CardContent>
            {positionList.length === 0 ? (
              <p className="text-sm text-text-dim">No open positions.</p>
            ) : (
              <div className="space-y-1">
                {positionList.map((pos: { symbol: string; qty: number; unrealized_pnl?: number; side?: string }) => (
                  <button
                    key={pos.symbol}
                    onClick={() => navigate(`/asset/${pos.symbol}`)}
                    className="flex w-full items-center justify-between rounded-md px-3 py-2 text-sm hover:bg-surface-2 transition-colors"
                  >
                    <span className="font-mono text-text">{pos.symbol}</span>
                    <div className="flex items-center gap-3 text-xs font-mono">
                      <span className="text-text-muted">qty {pos.qty}</span>
                      {pos.unrealized_pnl != null && (
                        <span className={pnlClass(pos.unrealized_pnl)}>
                          {pos.unrealized_pnl >= 0 ? (
                            <TrendingUp className="inline h-3 w-3 mr-0.5" />
                          ) : (
                            <TrendingDown className="inline h-3 w-3 mr-0.5" />
                          )}
                          {fmtCurrency(pos.unrealized_pnl)}
                        </span>
                      )}
                    </div>
                  </button>
                ))}
              </div>
            )}
          </CardContent>
        </Card>
      </div>
    </div>
  )
}
