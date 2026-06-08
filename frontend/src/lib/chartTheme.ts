function cssVar(name: string): string {
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim()
}

export function chartColors() {
  return {
    text: cssVar('--tb-text-muted'),
    grid: cssVar('--tb-border'),
    border: cssVar('--tb-border-2'),
    accent: cssVar('--tb-accent'),
    pnlUp: cssVar('--tb-pnl-up'),
    pnlDown: cssVar('--tb-pnl-down'),
  }
}
