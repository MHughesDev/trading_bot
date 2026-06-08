// Decimal-safe display formatting.  Never use parseFloat/toFixed on money values.

export function formatPrice(value: string | number, decimals = 2): string {
  const s = typeof value === 'number' ? value.toFixed(decimals) : value
  const [int, frac = ''] = s.split('.')
  const paddedFrac = frac.padEnd(decimals, '0').slice(0, decimals)
  const formattedInt = int.replace(/\B(?=(\d{3})+(?!\d))/g, ',')
  return decimals > 0 ? `${formattedInt}.${paddedFrac}` : formattedInt
}

export function formatSize(value: string | number, decimals = 4): string {
  return formatPrice(value, decimals)
}

export function formatPnl(value: string | number): string {
  const s = formatPrice(value, 2)
  const n = typeof value === 'number' ? value : parseFloat(value)
  return n >= 0 ? `+${s}` : s
}

export function pnlClass(value: string | number): string {
  const n = typeof value === 'number' ? value : parseFloat(value)
  if (n > 0) return 'text-green-400'
  if (n < 0) return 'text-red-400'
  return 'text-text-dim'
}
