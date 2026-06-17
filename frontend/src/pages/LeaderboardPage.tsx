/**
 * I-6.6 — AI Model Leaderboard page.
 *
 * Shows a ranked table of all models sorted by composite score.
 * Each row links to the ModelDetailPage. A secondary evaluation
 * report viewer is accessible via a slide-over panel.
 */

import { useState } from 'react'
import { useNavigate } from 'react-router-dom'
import { useQuery } from '@tanstack/react-query'
import { api as client } from '@/lib/api'
import {
  Trophy, TrendingUp, TrendingDown, Minus,
  ChevronDown, ChevronUp, ExternalLink, Loader2,
} from 'lucide-react'
import { cn } from '@/lib/utils'

interface LeaderboardEntry {
  rank: number
  model_id: string
  slug: string
  display_name: string
  model_kind: string
  asset_class: string
  score: number
  crps?: number
  coverage_90?: number
  win_rate?: number
  sharpe?: number
  version: number
  last_eval_at?: string
}

interface LeaderboardResponse {
  entries: LeaderboardEntry[]
}

function useLeaderboard() {
  return useQuery({
    queryKey: ['model-leaderboard'],
    queryFn: () =>
      client.get<LeaderboardResponse>('/api/models/leaderboard').then((r) => r.data),
    refetchInterval: 120_000,
  })
}

type SortKey = 'rank' | 'score' | 'crps' | 'coverage_90' | 'sharpe'

function ScoreBadge({ score }: { score: number }) {
  const color =
    score >= 80
      ? 'bg-green-500/15 text-green-400 border-green-500/30'
      : score >= 60
        ? 'bg-yellow-500/15 text-yellow-400 border-yellow-500/30'
        : 'bg-red-500/15 text-red-400 border-red-500/30'

  return (
    <span className={cn('inline-flex items-center rounded-md border px-2 py-0.5 text-xs font-mono font-medium', color)}>
      {score.toFixed(1)}
    </span>
  )
}

function RankMedal({ rank }: { rank: number }) {
  if (rank === 1) return <span className="text-yellow-400 font-bold text-sm">🥇</span>
  if (rank === 2) return <span className="text-zinc-300 font-bold text-sm">🥈</span>
  if (rank === 3) return <span className="text-amber-600 font-bold text-sm">🥉</span>
  return <span className="text-xs text-text-muted font-mono w-5 text-right">{rank}</span>
}

function DeltaIcon({ value }: { value?: number }) {
  if (value == null) return <Minus className="h-3 w-3 text-text-muted" />
  if (value > 0) return <TrendingUp className="h-3 w-3 text-pnl-up" />
  return <TrendingDown className="h-3 w-3 text-pnl-down" />
}

export function LeaderboardPage() {
  const navigate = useNavigate()
  const { data, isLoading } = useLeaderboard()
  const [sortKey, setSortKey] = useState<SortKey>('rank')
  const [sortAsc, setSortAsc] = useState(true)
  const [selectedId, setSelectedId] = useState<string | null>(null)

  function toggleSort(key: SortKey) {
    if (sortKey === key) setSortAsc((p) => !p)
    else { setSortKey(key); setSortAsc(key === 'rank') }
  }

  const sorted = [...(data?.entries ?? [])].sort((a, b) => {
    const av = (a[sortKey] ?? 0) as number
    const bv = (b[sortKey] ?? 0) as number
    return sortAsc ? av - bv : bv - av
  })

  const SortHeader = ({ col, label }: { col: SortKey; label: string }) => (
    <button
      onClick={() => toggleSort(col)}
      className="flex items-center gap-1 text-xs font-medium text-text-muted hover:text-text transition-colors"
    >
      {label}
      {sortKey === col
        ? sortAsc
          ? <ChevronUp className="h-3 w-3" />
          : <ChevronDown className="h-3 w-3" />
        : <ChevronDown className="h-3 w-3 opacity-30" />}
    </button>
  )

  return (
    <div className="mx-auto w-full max-w-5xl px-6 py-6">
      <div className="flex items-center gap-3 mb-6">
        <div className="flex h-9 w-9 items-center justify-center rounded-xl bg-surface-2 text-accent">
          <Trophy className="h-4.5 w-4.5" />
        </div>
        <div>
          <h1 className="text-lg font-semibold text-text">Model Leaderboard</h1>
          <p className="text-xs text-text-muted">Ranked by composite forecast quality score</p>
        </div>
      </div>

      {isLoading && (
        <div className="flex items-center gap-2 text-sm text-text-muted py-12 justify-center">
          <Loader2 className="h-4 w-4 animate-spin" />
          Loading leaderboard…
        </div>
      )}

      {!isLoading && sorted.length === 0 && (
        <div className="text-center py-16 text-text-muted text-sm">
          No evaluated models yet. Train and evaluate a model to appear here.
        </div>
      )}

      {sorted.length > 0 && (
        <div className="rounded-xl border border-border bg-surface overflow-hidden">
          <table className="w-full text-sm">
            <thead>
              <tr className="border-b border-border bg-surface-2">
                <th className="px-4 py-3 text-left w-12">
                  <SortHeader col="rank" label="Rank" />
                </th>
                <th className="px-4 py-3 text-left">Model</th>
                <th className="px-4 py-3 text-left hidden sm:table-cell">Kind</th>
                <th className="px-4 py-3 text-right">
                  <SortHeader col="score" label="Score" />
                </th>
                <th className="px-4 py-3 text-right hidden md:table-cell">
                  <SortHeader col="crps" label="CRPS" />
                </th>
                <th className="px-4 py-3 text-right hidden md:table-cell">
                  <SortHeader col="coverage_90" label="Cov 90%" />
                </th>
                <th className="px-4 py-3 text-right hidden lg:table-cell">
                  <SortHeader col="sharpe" label="Sharpe" />
                </th>
                <th className="px-4 py-3 w-8" />
              </tr>
            </thead>
            <tbody className="divide-y divide-border">
              {sorted.map((entry) => (
                <tr
                  key={entry.model_id}
                  className={cn(
                    'hover:bg-surface-2 transition-colors cursor-pointer',
                    selectedId === entry.model_id && 'bg-accent/5',
                  )}
                  onClick={() => setSelectedId(selectedId === entry.model_id ? null : entry.model_id)}
                >
                  <td className="px-4 py-3">
                    <RankMedal rank={entry.rank} />
                  </td>
                  <td className="px-4 py-3">
                    <div className="font-medium text-text">{entry.display_name}</div>
                    <div className="text-xs text-text-muted font-mono">{entry.slug}</div>
                  </td>
                  <td className="px-4 py-3 hidden sm:table-cell">
                    <span className="text-xs text-text-muted capitalize">{entry.model_kind.replace('_', ' ')}</span>
                  </td>
                  <td className="px-4 py-3 text-right">
                    <ScoreBadge score={entry.score} />
                  </td>
                  <td className="px-4 py-3 text-right hidden md:table-cell">
                    <div className="flex items-center justify-end gap-1">
                      <DeltaIcon value={entry.crps != null ? -entry.crps : undefined} />
                      <span className="font-mono text-xs text-text">
                        {entry.crps?.toFixed(4) ?? '—'}
                      </span>
                    </div>
                  </td>
                  <td className="px-4 py-3 text-right hidden md:table-cell">
                    <span className="font-mono text-xs text-text">
                      {entry.coverage_90 != null ? `${(entry.coverage_90 * 100).toFixed(1)}%` : '—'}
                    </span>
                  </td>
                  <td className="px-4 py-3 text-right hidden lg:table-cell">
                    <span className="font-mono text-xs text-text">
                      {entry.sharpe?.toFixed(2) ?? '—'}
                    </span>
                  </td>
                  <td className="px-4 py-3">
                    <button
                      onClick={(e) => {
                        e.stopPropagation()
                        navigate(`/models/${entry.model_id}`)
                      }}
                      className="text-text-muted hover:text-accent transition-colors"
                      title="Open model"
                    >
                      <ExternalLink className="h-3.5 w-3.5" />
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}

      {/* Expanded report panel */}
      {selectedId && (
        <EvalReportPanel modelId={selectedId} onClose={() => setSelectedId(null)} />
      )}
    </div>
  )
}

function EvalReportPanel({ modelId, onClose }: { modelId: string; onClose: () => void }) {
  const { data: quality } = useQuery({
    queryKey: ['model-quality', modelId],
    queryFn: () =>
      client.get<{ series: unknown[]; alerts: unknown[] }>(`/api/models/${modelId}/quality`).then((r) => r.data),
  })

  return (
    <div className="mt-4 rounded-xl border border-accent/30 bg-surface p-5 relative">
      <button
        onClick={onClose}
        className="absolute top-4 right-4 text-text-muted hover:text-text text-xs"
      >
        Close ✕
      </button>
      <h3 className="text-sm font-medium text-text mb-3">Quality Report</h3>
      {quality ? (
        <div className="space-y-2">
          <p className="text-xs text-text-muted">
            {quality.series.length} data points collected.{' '}
            {quality.alerts.length > 0
              ? `⚠ ${quality.alerts.length} active alert(s).`
              : '✓ No active alerts.'}
          </p>
          <div className="flex gap-2">
            <button
              onClick={() => window.open(`/models/${modelId}?tab=forecast`, '_blank')}
              className="inline-flex items-center gap-1.5 text-xs text-accent hover:underline"
            >
              <ExternalLink className="h-3 w-3" />
              Open Forecast Quality tab
            </button>
          </div>
        </div>
      ) : (
        <div className="flex items-center gap-2 text-sm text-text-muted">
          <Loader2 className="h-3.5 w-3.5 animate-spin" />
          Loading…
        </div>
      )}
    </div>
  )
}
