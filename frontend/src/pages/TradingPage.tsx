// Trading section — full-height horizontal infinite scroll of panels.
// Default workspace is seeded from the LayoutTemplateRegistry.

import { useState, useCallback } from 'react'
import { WorkspaceScroll } from '@/components/trading/WorkspaceScroll'
import { Panel } from '@/components/trading/Panel'
import { ScannerPanel } from '@/components/trading/ScannerPanel'
import { TerminalPanel, type TerminalAssetClass } from '@/components/trading/TerminalPanel'
import { ModeBadge } from '@/components/layout/ModeBadge'
import { layoutTemplates } from '@/config/layoutTemplates'
import type { PanelSpec } from '@/config/layoutTemplates'
import { Plus } from 'lucide-react'

function panelTitle(spec: PanelSpec): string {
  if (spec.kind === 'scanner') return 'Scanner'
  return spec.instrument ?? 'Terminal'
}

function panelWidth(spec: PanelSpec): number {
  return spec.kind === 'scanner' ? 360 : 480
}

export function TradingPage() {
  const [panels, setPanels] = useState<PanelSpec[]>(
    layoutTemplates.default.panels,
  )

  const removePanel = useCallback((id: string) => {
    setPanels((prev) => prev.filter((p) => p.id !== id))
  }, [])

  const addTerminal = useCallback(() => {
    const id = `terminal-${Date.now()}`
    setPanels((prev) => [
      ...prev,
      {
        id,
        kind: 'terminal',
        instrument: 'BTC-USD',
        venue: 'kraken',
        assetClass: 'crypto_spot_cex',
      },
    ])
  }, [])

  const addScanner = useCallback(() => {
    const id = `scanner-${Date.now()}`
    setPanels((prev) => [...prev, { id, kind: 'scanner' }])
  }, [])

  return (
    // Use h-full so the Trading section fills the remaining viewport height
    // (AppLayout main area has overflow-hidden — no vertical scroll for Trading).
    <div className="flex flex-col h-full overflow-hidden">
      {/* Workspace toolbar */}
      <div className="flex items-center gap-2 px-4 py-2 border-b border-border shrink-0 bg-surface">
        <span className="text-xs font-semibold text-text-muted uppercase tracking-wider mr-2">
          Trading
        </span>
        <button
          onClick={addScanner}
          className="flex items-center gap-1.5 rounded-lg px-2.5 py-1 text-xs text-text-muted hover:text-text hover:bg-border transition-colors border border-border"
        >
          <Plus className="h-3 w-3" />
          Scanner
        </button>
        <button
          onClick={addTerminal}
          className="flex items-center gap-1.5 rounded-lg px-2.5 py-1 text-xs text-text-muted hover:text-text hover:bg-border transition-colors border border-border"
        >
          <Plus className="h-3 w-3" />
          Terminal
        </button>
        <div className="ml-auto">
          <ModeBadge />
        </div>
      </div>

      {/* Horizontal panel scroll */}
      <WorkspaceScroll className="flex-1">
        {panels.map((spec) => (
          <Panel
            key={spec.id}
            title={panelTitle(spec)}
            width={panelWidth(spec)}
            onClose={() => removePanel(spec.id)}
          >
            {spec.kind === 'scanner' ? (
              <ScannerPanel />
            ) : (
              <TerminalPanel
                instrument={spec.instrument ?? 'BTC-USD'}
                assetClass={(spec.assetClass as TerminalAssetClass) ?? 'crypto_spot_cex'}
              />
            )}
          </Panel>
        ))}

        {/* Add panel affordance at the end */}
        <div className="flex flex-col items-center justify-center gap-4 w-40 shrink-0 border-l border-dashed border-border p-4">
          <button
            onClick={addScanner}
            className="flex flex-col items-center gap-2 rounded-xl border border-dashed border-border p-4 text-xs text-text-dim hover:text-text hover:border-border-2 transition-colors w-full"
          >
            <Plus className="h-4 w-4" />
            Scanner
          </button>
          <button
            onClick={addTerminal}
            className="flex flex-col items-center gap-2 rounded-xl border border-dashed border-border p-4 text-xs text-text-dim hover:text-text hover:border-border-2 transition-colors w-full"
          >
            <Plus className="h-4 w-4" />
            Terminal
          </button>
        </div>
      </WorkspaceScroll>
    </div>
  )
}
