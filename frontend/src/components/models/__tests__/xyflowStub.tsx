import type { ReactNode } from 'react'

/** jsdom has no ResizeObserver, which the real canvas needs; the chain is also listed as text. */
export function ReactFlow({ nodes, children }: { nodes: Array<{ id: string; data: { label: string } }>; children?: ReactNode }) {
  return (
    <div data-testid="flow">
      {nodes.map((n) => (
        <span key={n.id} data-testid="flow-node">
          {n.data.label}
        </span>
      ))}
      {children}
    </div>
  )
}
export const Background = () => null
export const Controls = () => null
export const MiniMap = () => null
