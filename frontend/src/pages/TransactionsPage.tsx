import { useQuery } from '@tanstack/react-query'
import { useNavigate } from 'react-router-dom'
import { Receipt, ArrowUpRight, ArrowDownRight } from 'lucide-react'
import { portfolioApi } from '@/lib/api'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Badge } from '@/components/ui/badge'
import { cn, formatDate, formatTime } from '@/lib/utils'

interface Transaction {
  ts: string
  symbol: string
  side: string
  quantity: string
  source: string
  correlation_id: string | null
  execution_mode: string | null
}

export function TransactionsPage() {
  const navigate = useNavigate()

  const { data, isLoading } = useQuery({
    queryKey: ['account-transactions'],
    queryFn: () => portfolioApi.transactions({ limit: 500 }).then((r) => r.data),
    refetchInterval: 30000,
  })

  const transactions: Transaction[] = data?.transactions ?? []

  return (
    <div className="p-6 space-y-6">
      <div className="flex items-center justify-between">
        <h1 className="text-xl font-semibold text-text flex items-center gap-2">
          <Receipt className="h-5 w-5 text-blue-400" />
          Transactions
        </h1>
        {data?.count != null && <Badge variant="outline">{data.count} total</Badge>}
      </div>

      <Card>
        <CardHeader className="pb-2">
          <CardTitle className="text-sm">Account Activity</CardTitle>
        </CardHeader>
        <CardContent>
          {isLoading ? (
            <p className="text-sm text-text-dim">Loading transactions…</p>
          ) : transactions.length === 0 ? (
            <p className="text-sm text-text-dim">No transactions yet. Trades submitted by the engine will appear here.</p>
          ) : (
            <div className="space-y-1">
              {/* Header row */}
              <div className="hidden sm:grid grid-cols-[1fr_1fr_1fr_1fr_1fr] gap-3 px-3 py-1.5 text-xs font-semibold uppercase tracking-widest text-text-dim">
                <span>Time</span>
                <span>Symbol</span>
                <span>Side</span>
                <span>Quantity</span>
                <span>Mode</span>
              </div>
              {transactions.map((tx, i) => (
                <button
                  key={`${tx.correlation_id ?? tx.ts}-${i}`}
                  onClick={() => navigate(`/asset/${tx.symbol}`)}
                  className="flex w-full sm:grid sm:grid-cols-[1fr_1fr_1fr_1fr_1fr] flex-wrap items-center gap-3 rounded-md px-3 py-2 text-sm text-left hover:bg-surface-2 transition-colors"
                >
                  <span className="text-text-muted font-mono text-xs">
                    {formatDate(tx.ts)} {formatTime(tx.ts)}
                  </span>
                  <span className="font-mono text-text">{tx.symbol}</span>
                  <span className={cn('flex items-center gap-1 text-xs font-semibold', tx.side === 'buy' ? 'text-pnl-up' : 'text-pnl-down')}>
                    {tx.side === 'buy' ? <ArrowUpRight className="h-3 w-3" /> : <ArrowDownRight className="h-3 w-3" />}
                    {tx.side.toUpperCase()}
                  </span>
                  <span className="font-mono text-text-muted">{tx.quantity}</span>
                  <span>
                    {tx.execution_mode && (
                      <Badge variant={tx.execution_mode === 'live' ? 'warning' : 'outline'}>
                        {tx.execution_mode}
                      </Badge>
                    )}
                  </span>
                </button>
              ))}
            </div>
          )}
        </CardContent>
      </Card>
    </div>
  )
}
