// INV-2: the distribution viewer exposes no best-member affordance. No control
// surfaces a metric-ranked member ("best", "sort by", "open peak"), and the
// worst-5% is rendered at least as prominently as the median.
import { describe, it, expect } from 'vitest'
import { render, screen } from '@testing-library/react'
import { DistributionViewer } from './DistributionViewer'
import type { StudyView } from '@/api/experiments'

const study: StudyView = {
  study_id: 'sweep-1',
  kind: 'parameter_sweep',
  metric: 'total_return',
  question: 'how does performance vary across the grid?',
  trial_delta: 12,
  sealed: true,
  distribution: {
    metric: 'total_return',
    dist: [0.02, 0.03, 0.05, 0.04, 0.06, 0.07, 0.08, 0.05],
    median: 0.05,
    iqr: [0.03, 0.07],
    worst_5pct: 0.02,
    spread: 0.018,
  },
  verdict: { summary: 'ok', positive_median: true, survivable_worst5: true, plateau: true },
  members: ['sha256:aaa', 'sha256:bbb', 'sha256:ccc'],
  selection_rule: 'median_stable_centroid',
  carried_forward: true,
  unsafe: false,
}

describe('DistributionViewer (INV-2)', () => {
  it('exposes no best-member / sort / open-peak control', () => {
    const { container } = render(<DistributionViewer study={study} />)
    const text = container.textContent ?? ''
    expect(text.toLowerCase()).not.toMatch(/best member|best run|sort by|rank|open peak|top performer/)
    // No actionable control (button/link) over members exists at all.
    expect(screen.queryByRole('button')).not.toBeInTheDocument()
    expect(screen.queryByRole('link')).not.toBeInTheDocument()
  })

  it('renders worst-5% at least as prominently as the median', () => {
    render(<DistributionViewer study={study} />)
    // "worst-5%"/"median" appear both as a headline stat and as a histogram
    // marker; the headline stat is the first occurrence in DOM order.
    const worst = screen.getAllByText('worst-5%')[0]
    const median = screen.getAllByText('median')[0]
    // Both are top-level distribution stats with the same emphatic styling.
    const worstStat = worst.parentElement!
    const medianStat = median.parentElement!
    const valueClass = (el: HTMLElement) => el.querySelector('div:last-child')?.className ?? ''
    expect(valueClass(worstStat)).toEqual(valueClass(medianStat))
    expect(valueClass(worstStat)).toContain('text-lg')
  })

  it('labels the carry-forward with its pre-declared selection rule (not an argmax)', () => {
    render(<DistributionViewer study={study} />)
    expect(screen.getByText(/median-stable centroid/i)).toBeInTheDocument()
  })

  it('shows member ids as provenance only, in insertion order', () => {
    render(<DistributionViewer study={study} />)
    expect(screen.getByText(/member run ids \(provenance, insertion order\)/i)).toBeInTheDocument()
  })
})
