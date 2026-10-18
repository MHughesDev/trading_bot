// The continuous sessions desk home shows (AGENT-012 §2 amended; AGENT-014 §9;
// task S-11.13).
//
// Three served reads per session — the list row, the detail (state with its
// diagnosis, the day budget) and the agenda (next item, depth) — because no one
// route carries all of them. Bounded sessions and finished ones are not read:
// a bounded session is one Research run, already a row in *Running now* through
// `/api/desk/running`, and a stopped one is neither running nor waiting.
import { useQueries, useQuery } from '@tanstack/react-query'
import { deskApi, type AgendaItem, type DeskSession, type SessionDetail } from '@/api/desk'

export interface ContinuousRow {
  session: DeskSession
  /** `undefined` while loading, `null` when the route did not answer. */
  detail: SessionDetail | null | undefined
  agenda: AgendaItem[] | null | undefined
}

export function useContinuousSessions(): {
  rows: ContinuousRow[]
  /** `null` when the session list itself did not answer. */
  listed: DeskSession[] | null
  loading: boolean
  failed: boolean
  refetch: () => void
} {
  const list = useQuery({
    queryKey: ['desk', 'sessions'],
    queryFn: () => deskApi.sessions(),
    refetchInterval: 30_000,
  })
  const live = (list.data?.sessions ?? []).filter(
    (s) => s.continuous && s.state !== 'complete' && s.state !== 'stopped',
  )
  const details = useQueries({
    queries: live.map((s) => ({
      queryKey: ['desk', 'session', s.session_id],
      queryFn: () => deskApi.session(s.session_id),
      refetchInterval: 30_000,
    })),
  })
  const agendas = useQueries({
    queries: live.map((s) => ({
      queryKey: ['desk', 'agenda', s.session_id],
      queryFn: () => deskApi.agenda(s.session_id),
      refetchInterval: 30_000,
    })),
  })
  const rows = live.map((session, i) => ({
    session,
    detail: details[i]?.isError ? null : details[i]?.data,
    agenda: agendas[i]?.isError ? null : agendas[i]?.data === null ? null : agendas[i]?.data?.items,
  }))
  return {
    rows,
    listed: list.isError || list.data === null ? null : (list.data?.sessions ?? null),
    loading: list.isPending,
    failed: list.isError || list.data === null,
    refetch: () => {
      void list.refetch()
      for (const d of details) void d.refetch()
      for (const a of agendas) void a.refetch()
    },
  }
}
