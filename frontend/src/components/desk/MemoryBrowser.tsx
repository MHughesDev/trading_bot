// The memory browser (AGENT-012 §7; task S-6.18).
//
// The rule that shapes it: **a conflicting pair renders side by side and neither
// is hidden by ranking.** A memory surface that shows only the winner of a
// disagreement is a surface that quietly decides the disagreement, and the
// person never learns there was one.
import { useMemo, useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { Badge } from '@/components/primitives/Badge'
import { Button } from '@/components/primitives/Button'
import { SearchInput } from '@/components/primitives/Field'
import { Tabs } from '@/components/primitives/Segmented'
import { EmptyState } from '@/components/primitives/States'
import { deskApi } from '@/api/desk'
import { ProvenanceBadge } from './Shared'

type Bucket = 'findings' | 'negative' | 'profiles' | 'questions' | 'preferences' | 'playbooks' | 'signals'

const BUCKETS: Array<{ value: Bucket; label: string }> = [
  { value: 'findings', label: 'Findings' },
  { value: 'negative', label: 'Negative results' },
  { value: 'profiles', label: 'Instrument profiles' },
  { value: 'questions', label: 'Open questions' },
  { value: 'preferences', label: 'Preferences' },
  { value: 'playbooks', label: 'Playbooks' },
  { value: 'signals', label: 'Signal registry' },
]

export interface MemoryRow {
  id: string
  subject: string
  claim: string
  confidence?: string
  n?: number | null
  window?: string
  valid_until?: string | null
  cited?: number
  /** Ids of rows this one disagrees with. */
  conflicts_with?: string[]
  provenance?: { run?: string; tool?: string; window?: string }
}

/**
 * Pairs every conflict exactly once.
 *
 * Both sides are returned together so the renderer cannot show one and rank the
 * other below the fold.
 */
export function conflictPairs(rows: MemoryRow[]): Array<[MemoryRow, MemoryRow]> {
  const byId = new Map(rows.map((r) => [r.id, r]))
  const seen = new Set<string>()
  const out: Array<[MemoryRow, MemoryRow]> = []
  for (const r of rows) {
    for (const other of r.conflicts_with ?? []) {
      const key = [r.id, other].sort().join('|')
      if (seen.has(key)) continue
      const o = byId.get(other)
      if (!o) continue
      seen.add(key)
      out.push([r, o])
    }
  }
  return out
}

function Row({ row, onRetry }: { row: MemoryRow; onRetry?: (r: MemoryRow) => void }) {
  return (
    <div className="desk-card">
      <div className="desk-card-hd">
        <Badge tone="outline">{row.subject}</Badge>
        {row.confidence && <Badge tone={row.confidence === 'refuted' ? 'neg' : 'neutral'}>{row.confidence}</Badge>}
        <ProvenanceBadge mark={null} />
      </div>
      <p>{row.claim}</p>
      <p className="sub">
        {row.n === null || row.n === undefined ? '(n unknown)' : `n=${row.n}`}
        {row.window ? ` · ${row.window}` : ''}
        {row.valid_until ? ` · valid until ${row.valid_until}` : ''}
        {row.cited !== undefined ? ` · cited ${row.cited}×` : ''}
      </p>
      <p className="sub">
        provenance: run {row.provenance?.run ?? 'unknown'} · tool {row.provenance?.tool ?? 'unknown'} · data{' '}
        {row.provenance?.window ?? 'unknown'}
      </p>
      {onRetry && (
        <Button size="xs" variant="ghost" onClick={() => onRetry(row)}>
          Retry now
        </Button>
      )}
    </div>
  )
}

export function MemoryBrowser({ onOpenConversation }: { onOpenConversation?: (seed: string) => void }) {
  const [bucket, setBucket] = useState<Bucket>('findings')
  const [q, setQ] = useState('')
  const [conflictsOnly, setConflictsOnly] = useState(false)

  const data = useQuery({
    queryKey: ['desk', 'memory', bucket],
    queryFn: () => deskApi.memory(bucket),
  })

  const rows = useMemo(() => (data.data?.rows ?? []) as unknown as MemoryRow[], [data.data])
  const filtered = rows.filter((r) => `${r.claim} ${r.subject}`.toLowerCase().includes(q.toLowerCase()))
  const pairs = useMemo(() => conflictPairs(rows), [rows])

  return (
    <div className="desk-scroll">
      <Tabs<Bucket> value={bucket} onChange={setBucket} tabs={BUCKETS} ariaLabel="Memory" />
      <div className="desk-actions">
        <SearchInput value={q} onValueChange={setQ} placeholder="Search memory" />
        <Button size="xs" variant={conflictsOnly ? 'primary' : 'ghost'} onClick={() => setConflictsOnly((c) => !c)}>
          Conflicts ({pairs.length})
        </Button>
      </div>

      {data.data === null && (
        <EmptyState
          message="The memory API is not built yet"
          detail="/api/memory/{bucket} lands with Set S phase 4 (AGENT-007). The browser is wired and fills the day it answers."
        />
      )}

      {conflictsOnly ? (
        pairs.length === 0 ? (
          <EmptyState message="No conflicts" detail="Nothing in this bucket disagrees with anything else." />
        ) : (
          pairs.map(([a, b]) => (
            <section
              key={`${a.id}|${b.id}`}
              style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 'var(--s-3)', marginTop: 'var(--s-3)' }}
              aria-label={`conflict between ${a.id} and ${b.id}`}
            >
              <Row row={a} />
              <Row row={b} />
              <div className="full">
                <Button
                  size="xs"
                  onClick={() => onOpenConversation?.(`diagnose the conflict between ${a.id} and ${b.id}`)}
                >
                  Resolve
                </Button>
              </div>
            </section>
          ))
        )
      ) : (
        filtered.map((r) => (
          <Row
            key={r.id}
            row={r}
            onRetry={
              bucket === 'negative' ? (row) => onOpenConversation?.(`retry: ${row.claim}`) : undefined
            }
          />
        ))
      )}
    </div>
  )
}
