/**
 * "Not measured" and "0%" are different answers, and only one is a measurement.
 *
 * `BarrenRate::rate` returns `null` over an empty sample precisely so a renderer
 * cannot show a platform that has produced no knowledge at all as one that never
 * wastes a trial. That guarantee is only worth what the renderer does with it,
 * which is what this pins.
 */
import { describe, expect, it } from 'vitest'

import { formatRate } from '../Operations'

describe('formatRate', () => {
  it('renders a measured rate as a percentage', () => {
    expect(formatRate(0.6, 10)).toBe('60%')
    expect(formatRate(0, 10)).toBe('0%')
  })

  it('renders an absent rate as "not measured", never as zero', () => {
    // The failure this exists for: a platform that has run nothing showing as a
    // platform that never wastes a trial.
    expect(formatRate(null, 0)).toBe('not measured')
  })

  it('refuses a rate that arrives with no denominator', () => {
    // A rate without runs behind it is not a rate, whatever the server sent.
    // Trusting the number here would let a malformed row read as a measurement.
    expect(formatRate(0.5, 0)).toBe('not measured')
  })

  it('rounds rather than truncating, so a small rate is not shown as none', () => {
    expect(formatRate(0.006, 500)).toBe('1%')
    expect(formatRate(0.004, 500)).toBe('0%')
  })
})
