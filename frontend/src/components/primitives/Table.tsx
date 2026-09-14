import { useMemo, useState, type ReactNode } from 'react'
import { cn } from '@/lib/utils'
import { EmptyState } from './States'

/* =============================================================================
   Spec §3.11 — table / data grid. The most important component in the product.

   - header 30px, --label-* styling, sticky, hairline bottom
   - first column left-aligned (identity); EVERY other column right-aligned (N7)
   - numeric cells carry .n → --font-numeric + tnum
   - sort via header click, with aria-sort
   - empty state replaces the BODY, never the header
   - loading renders skeleton rows at the same height, never a spinner
   ============================================================================= */

export interface Column<R> {
  key: string
  header: ReactNode
  /** Cell renderer. */
  cell: (row: R, index: number) => ReactNode
  /** Tabular numerals + right alignment. Defaults to true for non-first columns. */
  numeric?: boolean
  /** Force left alignment (identity-like columns beyond the first). */
  align?: 'left' | 'right'
  /** Comparable value for sorting. Omit to make the column unsortable. */
  sortValue?: (row: R) => number | string
  width?: number | string
  /** Hide at narrow widths. */
  hideBelow?: number
  headerTitle?: string
}

export interface TableProps<R> {
  columns: Column<R>[]
  rows: R[]
  rowKey: (row: R, index: number) => string
  /** Total row rendered in <tfoot>. Must reconcile with the body (§4.1 MUST). */
  footer?: ReactNode
  caption?: string
  /** Currently selected row key. */
  selectedKey?: string | null
  onRowClick?: (row: R, index: number) => void
  loading?: boolean
  /** Rows to skeleton while loading. */
  skeletonRows?: number
  empty?: ReactNode
  dense?: boolean
  className?: string
  initialSort?: { key: string; dir: 'asc' | 'desc' }
  /** Disable internal sorting when the caller sorts server-side. */
  sortable?: boolean
}

export function Table<R>({
  columns,
  rows,
  rowKey,
  footer,
  caption,
  selectedKey,
  onRowClick,
  loading,
  skeletonRows = 6,
  empty,
  dense,
  className,
  initialSort,
  sortable = true,
}: TableProps<R>) {
  const [sort, setSort] = useState<{ key: string; dir: 'asc' | 'desc' } | null>(initialSort ?? null)

  const sorted = useMemo(() => {
    if (!sortable || !sort) return rows
    const col = columns.find((c) => c.key === sort.key)
    if (!col?.sortValue) return rows
    const dir = sort.dir === 'asc' ? 1 : -1
    return [...rows].sort((a, b) => {
      const av = col.sortValue!(a)
      const bv = col.sortValue!(b)
      if (typeof av === 'number' && typeof bv === 'number') return (av - bv) * dir
      return String(av).localeCompare(String(bv)) * dir
    })
  }, [rows, sort, columns, sortable])

  function toggleSort(col: Column<R>) {
    if (!sortable || !col.sortValue) return
    setSort((s) =>
      s?.key === col.key ? { key: col.key, dir: s.dir === 'asc' ? 'desc' : 'asc' } : { key: col.key, dir: 'desc' },
    )
  }

  const showEmpty = !loading && sorted.length === 0

  return (
    <table className={cn('tbl', dense && 'dense', className)}>
      {caption && <caption className="sr-only">{caption}</caption>}
      <thead>
        <tr>
          {columns.map((c, i) => {
            const isNumeric = c.numeric ?? i > 0
            const align = c.align ?? (isNumeric ? 'right' : i === 0 ? 'left' : 'right')
            const active = sort?.key === c.key
            return (
              <th
                key={c.key}
                scope="col"
                title={c.headerTitle}
                style={c.width ? { width: c.width } : undefined}
                aria-sort={active ? (sort!.dir === 'asc' ? 'ascending' : 'descending') : undefined}
                className={cn(align === 'left' && 'l', sortable && c.sortValue && 'sortable')}
                onClick={() => toggleSort(c)}
              >
                {c.header}
                {active && <span className="caret" aria-hidden>{sort!.dir === 'asc' ? '▲' : '▼'}</span>}
              </th>
            )
          })}
        </tr>
      </thead>

      <tbody>
        {loading &&
          Array.from({ length: skeletonRows }).map((_, r) => (
            <tr key={`sk-${r}`}>
              {columns.map((c, i) => (
                <td key={c.key} className={cn(i === 0 && 'l')}>
                  <span className="skel" style={{ display: 'inline-block', width: i === 0 ? '60%' : '46%' }} />
                </td>
              ))}
            </tr>
          ))}

        {showEmpty && (
          <tr>
            <td colSpan={columns.length} className="l" style={{ height: 'auto', padding: 0 }}>
              {empty ?? <EmptyState message="Nothing here yet." />}
            </td>
          </tr>
        )}

        {!loading &&
          sorted.map((row, index) => {
            const key = rowKey(row, index)
            return (
              <tr
                key={key}
                className={cn('hoverable', onRowClick && 'clickable', selectedKey === key && 'on')}
                onClick={onRowClick ? () => onRowClick(row, index) : undefined}
                tabIndex={onRowClick ? 0 : undefined}
                onKeyDown={
                  onRowClick
                    ? (e) => {
                        if (e.key === 'Enter' || e.key === ' ') {
                          e.preventDefault()
                          onRowClick(row, index)
                        }
                      }
                    : undefined
                }
              >
                {columns.map((c, i) => {
                  const isNumeric = c.numeric ?? i > 0
                  const align = c.align ?? (isNumeric ? 'right' : i === 0 ? 'left' : 'right')
                  return (
                    <td key={c.key} className={cn(isNumeric && 'n', align === 'left' && 'l')}>
                      {c.cell(row, index)}
                    </td>
                  )
                })}
              </tr>
            )
          })}
      </tbody>

      {footer && !loading && sorted.length > 0 && <tfoot>{footer}</tfoot>}
    </table>
  )
}

/** Identity cell: swatch? + ticker over name (§3.11). */
export function IdentityCell({
  swatch,
  ticker,
  name,
}: {
  swatch?: ReactNode
  ticker: ReactNode
  name?: ReactNode
}) {
  return (
    <span className="sym">
      {swatch}
      <span className="sym-stack">
        <span className="tick truncate-1">{ticker}</span>
        {name !== undefined && name !== null && <span className="name truncate-1">{name}</span>}
      </span>
    </span>
  )
}

/** A scrolling shell for a table inside a flush panel body. */
export function TableScroll({ children, className }: { children: ReactNode; className?: string }) {
  return <div className={cn('tblwrap', className)}>{children}</div>
}
