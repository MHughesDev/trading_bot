import { useMemo, useState } from 'react'
import type { NodeFamily } from '@/nodes'
import { FAMILIES } from '@/nodes/families'
import { Panel, PanelBody, PanelHeader } from '@/components/primitives/Panel'
import { SearchInput } from '@/components/primitives/Field'
import { NoResults } from '@/components/primitives/States'
import { Label } from '@/components/primitives/Num'
import { cn } from '@/lib/utils'
import { BLOCKS } from './BlockPalette.blocks'

/* Spec §4.3 — the blocks panel gains a SEARCH field and FAMILY GROUPING with a
   colour dot. The flat alphabetical list goes away. */

export interface BlockDef {
  label: string
  family: NodeFamily
  type: string
  data: Record<string, unknown>
  /** Extra words that should match this block in search. */
  keywords?: string
}

const FAMILY_ORDER: NodeFamily[] = ['data', 'indicator', 'signal', 'logic', 'ai', 'intent', 'risk']

export function BlockPalette({ className }: { className?: string }) {
  const [q, setQ] = useState('')

  const filtered = useMemo(() => {
    const t = q.trim().toLowerCase()
    if (!t) return BLOCKS
    return BLOCKS.filter((b) =>
      `${b.label} ${b.family} ${FAMILIES[b.family].label} ${b.keywords ?? ''}`.toLowerCase().includes(t),
    )
  }, [q])

  const grouped = FAMILY_ORDER.map((f) => ({
    family: f,
    blocks: filtered.filter((b) => b.family === f),
  })).filter((g) => g.blocks.length > 0)

  function onDragStart(e: React.DragEvent, block: BlockDef) {
    e.dataTransfer.setData('application/reactflow', JSON.stringify({ type: block.type, data: block.data }))
    e.dataTransfer.effectAllowed = 'move'
  }

  return (
    <Panel className={cn('strat-palette', className)}>
      <PanelHeader title="Blocks" actions={<Label>Drag to canvas</Label>} />
      <div style={{ padding: 'var(--s-3) var(--s-3) 0' }}>
        <SearchInput value={q} onValueChange={setQ} placeholder="Search blocks" aria-label="Search blocks" />
      </div>
      <PanelBody flush style={{ overflow: 'auto', paddingBottom: 'var(--s-3)' }}>
        {grouped.length === 0 ? (
          <NoResults query={q} onClear={() => setQ('')} />
        ) : (
          grouped.map(({ family, blocks }) => {
            const info = FAMILIES[family]
            const Icon = info.icon
            return (
              <div key={family}>
                <div className="pal-grp" title={info.blurb}>
                  <i className="swatch" style={{ background: info.token }} aria-hidden />
                  <span className="lbl">{info.label}</span>
                </div>
                {blocks.map((b) => (
                  <button
                    key={`${b.family}-${b.label}`}
                    type="button"
                    className="pal-item"
                    draggable
                    onDragStart={(e) => onDragStart(e, b)}
                    title={`${b.label} — ${info.blurb}`}
                  >
                    <span className="g" style={{ color: info.token }}>
                      <Icon size={11} aria-hidden />
                    </span>
                    <span className="truncate-1">{b.label}</span>
                  </button>
                ))}
              </div>
            )
          })
        )}
      </PanelBody>
    </Panel>
  )
}
