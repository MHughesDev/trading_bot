// All functions return arrays the same length as the input.
// Positions without enough data are NaN.

export function calcSMA(closes: number[], period: number): number[] {
  const out = new Float64Array(closes.length).fill(NaN)
  let sum = 0
  for (let i = 0; i < closes.length; i++) {
    sum += closes[i]
    if (i >= period) sum -= closes[i - period]
    if (i >= period - 1) out[i] = sum / period
  }
  return Array.from(out)
}

export function calcEMA(closes: number[], period: number): number[] {
  const out = new Float64Array(closes.length).fill(NaN)
  if (closes.length < period) return Array.from(out)
  const k = 2 / (period + 1)
  let prev = 0
  for (let i = 0; i < period; i++) prev += closes[i]
  prev /= period
  out[period - 1] = prev
  for (let i = period; i < closes.length; i++) {
    prev = closes[i] * k + prev * (1 - k)
    out[i] = prev
  }
  return Array.from(out)
}

export interface BBResult {
  upper: number[]
  middle: number[]
  lower: number[]
}
export function calcBB(closes: number[], period = 20, mult = 2): BBResult {
  const middle = calcSMA(closes, period)
  const upper = middle.map((m, i) => {
    if (isNaN(m)) return NaN
    const slice = closes.slice(i - period + 1, i + 1)
    const variance = slice.reduce((s, v) => s + (v - m) ** 2, 0) / period
    return m + mult * Math.sqrt(variance)
  })
  const lower = upper.map((u, i) => (isNaN(u) ? NaN : 2 * middle[i] - u))
  return { upper, middle, lower }
}

export function calcRSI(closes: number[], period = 14): number[] {
  const out = new Float64Array(closes.length).fill(NaN)
  if (closes.length < period + 1) return Array.from(out)
  let avgGain = 0, avgLoss = 0
  for (let i = 1; i <= period; i++) {
    const d = closes[i] - closes[i - 1]
    if (d > 0) avgGain += d; else avgLoss -= d
  }
  avgGain /= period; avgLoss /= period
  out[period] = 100 - 100 / (1 + avgGain / (avgLoss || 1e-10))
  for (let i = period + 1; i < closes.length; i++) {
    const d = closes[i] - closes[i - 1]
    avgGain = (avgGain * (period - 1) + Math.max(d, 0)) / period
    avgLoss = (avgLoss * (period - 1) + Math.max(-d, 0)) / period
    out[i] = 100 - 100 / (1 + avgGain / (avgLoss || 1e-10))
  }
  return Array.from(out)
}

export interface MACDResult {
  macdLine: number[]
  signalLine: number[]
  histogram: number[]
}
export function calcMACD(
  closes: number[],
  fast = 12,
  slow = 26,
  signalPeriod = 9,
): MACDResult {
  const fastE = calcEMA(closes, fast)
  const slowE = calcEMA(closes, slow)
  const macdLine = closes.map((_, i) =>
    isNaN(fastE[i]) || isNaN(slowE[i]) ? NaN : fastE[i] - slowE[i],
  )

  // EMA of MACD — only feed valid (non-NaN) values
  const validIdx: number[] = []
  const validVals: number[] = []
  for (let i = 0; i < macdLine.length; i++) {
    if (!isNaN(macdLine[i])) { validIdx.push(i); validVals.push(macdLine[i]) }
  }
  const sigVals = calcEMA(validVals, signalPeriod)
  const signalLine = new Float64Array(closes.length).fill(NaN)
  validIdx.forEach((idx, j) => { if (!isNaN(sigVals[j])) signalLine[idx] = sigVals[j] })

  const histogram = macdLine.map((m, i) =>
    isNaN(m) || isNaN(signalLine[i]) ? NaN : m - signalLine[i],
  )
  return { macdLine, signalLine: Array.from(signalLine), histogram }
}
