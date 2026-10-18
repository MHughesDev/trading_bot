// One place that turns a settled `Read<T>` into either its data or the honest
// sentence for why there is none (see `api/ops.ts`).
//
// The four failure sentences are deliberately different from each other and
// from an empty list. A halt list that failed to load must never render as "no
// halts"; this component is how a page is made unable to do that by accident:
// `children` is only called with data the route actually returned.
import type { ReactNode } from 'react'
import type { UseQueryResult } from '@tanstack/react-query'
import { EmptyState, ErrorState, PanelLoading } from '@/components/primitives/States'
import type { Read } from '@/api/ops'

export function ReadGate<T>({
  query,
  route,
  what,
  children,
}: {
  query: Pick<UseQueryResult<Read<T>>, 'data' | 'isPending' | 'isError' | 'refetch'>
  /** The path, so a failure names the thing that did not answer. */
  route: string
  /** What the panel is, in lower case: "the halt list". */
  what: string
  children: (data: T) => ReactNode
}) {
  if (query.isPending) return <PanelLoading />
  const r = query.data
  if (query.isError || !r) {
    return (
      <ErrorState
        message={`Could not read ${what}`}
        detail={`${route} did not complete. ${what} is unknown, not empty.`}
        onRetry={() => void query.refetch()}
      />
    )
  }
  switch (r.state) {
    case 'ok':
      return <>{children(r.data)}</>
    case 'not_served':
      return (
        <EmptyState
          message={`${route} is not served by this build`}
          detail={`${what} cannot be shown. That is a missing route, not a clean result.`}
        />
      )
    case 'forbidden':
      return (
        <EmptyState
          message={`This session may not read ${what}`}
          detail={r.detail}
        />
      )
    case 'error':
      return (
        <ErrorState
          message={`Could not read ${what}`}
          detail={`${route} answered ${r.status ?? 'nothing'}: ${r.message}. ${what} is unknown, not empty.`}
          onRetry={() => void query.refetch()}
        />
      )
  }
}
