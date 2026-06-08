import { useEffect, useRef } from 'react'

export type NodeMenuState = { nodeId: string; x: number; y: number; disabled: boolean }

export function NodeContextMenu({ menu, onClose, onDuplicate, onToggleDisabled, onDisconnect, onDelete }: {
  menu: NodeMenuState
  onClose: () => void
  onDuplicate: (id: string) => void
  onToggleDisabled: (id: string) => void
  onDisconnect: (id: string) => void
  onDelete: (id: string) => void
}) {
  const ref = useRef<HTMLDivElement>(null)

  useEffect(() => {
    function onPointerDown(e: PointerEvent) {
      if (ref.current && !ref.current.contains(e.target as globalThis.Node)) onClose()
    }
    function onKey(e: KeyboardEvent) { if (e.key === 'Escape') onClose() }
    window.addEventListener('pointerdown', onPointerDown)
    window.addEventListener('keydown', onKey)
    window.addEventListener('scroll', onClose, true)
    window.addEventListener('blur', onClose)
    return () => {
      window.removeEventListener('pointerdown', onPointerDown)
      window.removeEventListener('keydown', onKey)
      window.removeEventListener('scroll', onClose, true)
      window.removeEventListener('blur', onClose)
    }
  }, [onClose])

  function run(action: (id: string) => void) {
    action(menu.nodeId)
    onClose()
  }

  return (
    <div ref={ref} className="tb-context-menu" style={{ left: menu.x, top: menu.y }}>
      <button className="tb-context-menu-item" onClick={() => run(onDuplicate)}>
        <span className="tb-context-menu-icon">⧉</span> Duplicate
      </button>
      <button className="tb-context-menu-item" onClick={() => run(onToggleDisabled)}>
        <span className="tb-context-menu-icon">{menu.disabled ? '▶' : '⏸'}</span> {menu.disabled ? 'Enable' : 'Disable'}
      </button>
      <button className="tb-context-menu-item" onClick={() => run(onDisconnect)}>
        <span className="tb-context-menu-icon">⌫</span> Disconnect edges
      </button>
      <div className="tb-context-menu-sep" />
      <button className="tb-context-menu-item danger" onClick={() => run(onDelete)}>
        <span className="tb-context-menu-icon">🗑</span> Delete
      </button>
    </div>
  )
}
