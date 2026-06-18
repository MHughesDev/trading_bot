import type { DragEvent } from 'react'

interface PaletteItem {
  type: string
  label: string
  icon: string
  data: Record<string, unknown>
}

interface Category {
  label: string
  color: string
  items: PaletteItem[]
}

const CATEGORIES: Category[] = [
  {
    label: 'Indicators', color: '#7C3AED',
    items: [
      { type: 'indicator', label: 'EMA', icon: '〜', data: { indicatorId: 'ema_1', kind: 'ema', period: 14 } },
      { type: 'indicator', label: 'SMA', icon: '〜', data: { indicatorId: 'sma_1', kind: 'sma', period: 14 } },
      { type: 'indicator', label: 'RSI', icon: '〜', data: { indicatorId: 'rsi_1', kind: 'rsi', period: 14 } },
      { type: 'indicator', label: 'ATR', icon: '〜', data: { indicatorId: 'atr_1', kind: 'atr', period: 14 } },
    ],
  },
  {
    label: 'Conditions', color: '#B45309',
    items: [
      { type: 'condition', label: 'Crosses Above', icon: '↗', data: { conditionType: 'cross_above', rightMode: 'indicator', rightValue: 0 } },
      { type: 'condition', label: 'Crosses Below', icon: '↘', data: { conditionType: 'cross_below', rightMode: 'indicator', rightValue: 0 } },
      { type: 'condition', label: 'Greater Than', icon: '>', data: { conditionType: 'greater_than', rightMode: 'value', rightValue: 50 } },
      { type: 'condition', label: 'Less Than', icon: '<', data: { conditionType: 'less_than', rightMode: 'value', rightValue: 50 } },
      { type: 'condition', label: 'Is Rising', icon: '↑', data: { conditionType: 'rising', rightMode: 'value', rightValue: 0 } },
      { type: 'condition', label: 'Is Falling', icon: '↓', data: { conditionType: 'falling', rightMode: 'value', rightValue: 0 } },
    ],
  },
  {
    label: 'AI', color: '#0E7490',
    items: [
      { type: 'ai_forecast', label: 'AI Model', icon: '◎', data: { model: 'price_forecaster', direction: 'bullish', minConfidence: 0.6, alias: 'production' } },
    ],
  },
  {
    label: 'Logic', color: '#1E40AF',
    items: [
      { type: 'logic', label: 'AND Gate', icon: '∧', data: { op: 'and', inputCount: 2 } },
      { type: 'logic', label: 'OR Gate', icon: '∨', data: { op: 'or', inputCount: 2 } },
    ],
  },
  {
    label: 'Trade', color: '#15803D',
    items: [
      { type: 'action', label: 'Buy / Sell', icon: '⊕', data: { side: 'buy' } },
      { type: 'size', label: 'Position Size', icon: '$', data: { sizeType: 'percent_of_equity', value: 0.02 } },
      { type: 'exit', label: 'Stop Loss', icon: '⊗', data: { exitType: 'stop_loss', value: 0.015 } },
      { type: 'exit', label: 'Take Profit', icon: '⊗', data: { exitType: 'take_profit', value: 0.04 } },
      { type: 'exit', label: 'Trailing Stop', icon: '⊗', data: { exitType: 'trailing_stop', value: 0.02 } },
    ],
  },
]

function onDragStart(e: DragEvent<HTMLDivElement>, type: string, data: Record<string, unknown>) {
  e.dataTransfer.setData('application/reactflow', JSON.stringify({ type, data }))
  e.dataTransfer.effectAllowed = 'move'
}

export function Palette() {
  return (
    <div style={{
      width: 192, flexShrink: 0, height: '100%', overflowY: 'auto', overflowX: 'hidden',
      background: 'var(--tb-surface)', borderRight: '1px solid var(--tb-border)', padding: '8px 0 16px',
      scrollbarWidth: 'thin', scrollbarColor: 'var(--tb-border-2) transparent',
    }}>
      <div style={{ padding: '4px 12px 8px', fontSize: 10, color: 'var(--tb-text-dim)', fontWeight: 600, letterSpacing: '0.1em', textTransform: 'uppercase' }}>
        Drag to canvas
      </div>

      {CATEGORIES.map(cat => (
        <div key={cat.label} style={{ marginBottom: 10 }}>
          <div style={{ padding: '3px 12px', fontSize: 9, fontWeight: 700, letterSpacing: '0.12em', textTransform: 'uppercase', color: cat.color }}>
            {cat.label}
          </div>
          {cat.items.map(item => (
            <div
              key={`${item.type}-${item.label}`}
              draggable
              onDragStart={e => onDragStart(e, item.type, { ...item.data })}
              style={{
                margin: '2px 8px', padding: '6px 10px', background: 'var(--tb-background)',
                border: '1px solid var(--tb-border)', borderLeft: `3px solid ${cat.color}`,
                borderRadius: 6, cursor: 'grab', fontSize: 11, color: 'var(--tb-text-muted)',
                display: 'flex', alignItems: 'center', gap: 7, userSelect: 'none',
                transition: 'background 0.1s, color 0.1s',
              }}
              onMouseEnter={e => { e.currentTarget.style.background = 'var(--tb-border)'; e.currentTarget.style.color = 'var(--tb-text)' }}
              onMouseLeave={e => { e.currentTarget.style.background = 'var(--tb-background)'; e.currentTarget.style.color = 'var(--tb-text-muted)' }}
            >
              <span style={{ fontSize: 13, width: 16, textAlign: 'center', flexShrink: 0, color: cat.color }}>{item.icon}</span>
              {item.label}
            </div>
          ))}
        </div>
      ))}

      <div style={{ margin: '12px 8px 0', padding: 10, background: 'var(--tb-background)', border: '1px solid var(--tb-border)', borderRadius: 6, fontSize: 10, color: 'var(--tb-text-dim)', lineHeight: 1.5 }}>
        <div style={{ color: 'var(--tb-text-dim)', fontWeight: 600, marginBottom: 4 }}>Tips</div>
        Drag nodes onto the canvas, then connect handles. Press{' '}
        <kbd style={{ background: 'var(--tb-border)', padding: '0 4px', borderRadius: 3, fontFamily: 'monospace' }}>Del</kbd>{' '}
        to remove selected nodes or edges.
      </div>
    </div>
  )
}
