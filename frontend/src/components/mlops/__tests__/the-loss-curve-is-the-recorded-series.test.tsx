/**
 * H-5.5 / H-5.4: the Training Console draws the loss curve the registry
 * recorded, and the cockpit's feed carries the model's own event log.
 *
 * A loss the trainer did not measure is `null` on the wire. It must leave a gap
 * (a run with no validation split has no validation line) and never be drawn as
 * a perfect zero.
 */
import { render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import { buildActivity, curveSeries, eventText } from '../studio.helpers'
import { LossCurve } from '../TrainLive'

describe('the loss curve', () => {
  it('draws two lines when both losses were reported', () => {
    render(
      <LossCurve
        points={[
          { epoch: 1, train_loss: 0.9, val_loss: 1.0 },
          { epoch: 2, train_loss: 0.5, val_loss: 0.8 },
        ]}
      />,
    )
    expect(screen.getByTestId('loss-curve').querySelectorAll('path')).toHaveLength(2)
    expect(screen.getByTestId('loss-curve').textContent).toContain('validation 0.8000 (dashed)')
  })

  it('draws one line, and says why, for a run with no validation split', () => {
    render(<LossCurve points={[{ epoch: 1, train_loss: 0.9, val_loss: null }]} />)
    expect(screen.getByTestId('loss-curve').querySelectorAll('path')).toHaveLength(1)
    expect(screen.getByTestId('loss-curve').textContent).toContain('no validation loss reported')
  })

  it('a null loss is a gap, not a zero', () => {
    const s = curveSeries([
      { epoch: 1, train_loss: 0.9, val_loss: null },
      { epoch: 2, train_loss: 0.5, val_loss: 0.8 },
    ])
    expect(s.val).toEqual([[2, 0.8]])
    expect(s.validated).toBe(1)
  })

  it('says nothing has reported rather than drawing an empty chart', () => {
    render(<LossCurve points={[]} />)
    expect(screen.getByTestId('loss-curve-empty')).toBeInTheDocument()
    expect(screen.queryByTestId('loss-curve')).toBeNull()
  })

  it('says the read failed rather than that nothing was reported', () => {
    render(<LossCurve points={undefined} failed />)
    expect(screen.getByTestId('loss-curve-unread')).toBeInTheDocument()
  })
})

describe('the activity feed carries the event log', () => {
  it('words a rename and a promotion, and joins them newest first', () => {
    const feed = buildActivity({
      runs: [],
      versions: [],
      deployments: [],
      events: [
        { kind: 'renamed', payload: { from: 'a', to: 'b' }, created_at: '2026-10-02T10:00:00Z' },
        { kind: 'promoted', payload: { version: 3, alias: 'production' }, created_at: '2026-10-03T10:00:00Z' },
      ],
    })
    expect(feed.map((f) => f.text)).toEqual(['v3 promoted to production', 'Renamed from "a" to "b"'])
  })

  it('shows a kind it has no words for as itself, and never an inference trace', () => {
    expect(eventText({ kind: 'dataset_attached' })).toBe('dataset attached')
    const feed = buildActivity({
      runs: [],
      versions: [],
      deployments: [],
      events: [{ kind: 'inference_trace', payload: {}, created_at: '2026-10-03T10:00:00Z' }],
    })
    expect(feed).toEqual([])
  })
})
