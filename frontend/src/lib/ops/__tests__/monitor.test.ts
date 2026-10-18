/**
 * S-7.10: the monitor's push channel, as the page reads it.
 *
 * Three frames share one stream and the SSE `event:` name does not reach the
 * client, so the frame's kind is read off its shape. These assert that the three
 * are told apart, that an unrecognised frame is dropped rather than guessed into
 * a snapshot, and — the part that is a safety property rather than tidiness —
 * that a drill never posts an operating-system notification, that a blip never
 * announces, and that a verdict this build has no word for is shown as itself
 * instead of folded into "fresh".
 */
import { describe, expect, it } from 'vitest'
import {
  applyFrame,
  ageWords,
  emptyLiveState,
  eventAnnouncement,
  eventSeverity,
  feedVerdict,
  mergeEvents,
  parseMonitorFrame,
  streamWords,
  timeframeWords,
} from '@/lib/ops/monitor'

const event = (over: Record<string, unknown> = {}) => ({
  event_id: 'e1',
  occurred_at: '2026-10-03T10:00:00Z',
  scope: 'global',
  family: 'lease',
  kind: 'tripped',
  metric: null,
  reason: 'heartbeat lost',
  actor: 'monitor',
  is_drill: false,
  ...over,
})

describe('the three frames are told apart by shape', () => {
  it('reads a monitor_event', () => {
    const f = parseMonitorFrame(event())
    expect(f.kind).toBe('monitor_event')
    expect(f.kind === 'monitor_event' && f.event.reason).toBe('heartbeat lost')
  })

  it('reads a kill_switch snapshot, with an unreadable switch carried as an error', () => {
    const f = parseMonitorFrame({ lease: 'held', orders_permitted: true, halted_scopes: ['global'] })
    expect(f).toMatchObject({ kind: 'kill_switch', snapshot: { halted_scopes: ['global'], error: null } })
    const bad = parseMonitorFrame({ lease: 'unreadable', orders_permitted: false, error: 'pool closed' })
    // Unreadable is not "nothing halted": the error survives into the state.
    expect(bad).toMatchObject({ kind: 'kill_switch', snapshot: { orders_permitted: false, error: 'pool closed' } })
  })

  it('reads a stream_error and drops anything else', () => {
    expect(parseMonitorFrame({ message: 'listener closed' })).toEqual({ kind: 'stream_error', message: 'listener closed' })
    for (const junk of [null, 7, 'x', {}, { foo: 1 }]) {
      expect(parseMonitorFrame(junk).kind).toBe('unknown')
    }
  })
})

describe('the shared live state', () => {
  it('keeps one row per event id, newest first, even when a frame is replayed', () => {
    let s = emptyLiveState()
    const a = parseMonitorFrame(event({ event_id: 'a', occurred_at: '2026-10-03T10:00:00Z' }))
    const b = parseMonitorFrame(event({ event_id: 'b', occurred_at: '2026-10-03T10:05:00Z', kind: 'breach' }))
    s = applyFrame(s, a)
    s = applyFrame(s, b)
    s = applyFrame(s, a)
    expect(s.events.map((e) => e.event_id)).toEqual(['b', 'a'])
  })

  it('a snapshot replaces the last snapshot and clears a stream error', () => {
    let s = applyFrame(emptyLiveState(), parseMonitorFrame({ message: 'boom' }))
    expect(s.streamError).toBe('boom')
    s = applyFrame(s, parseMonitorFrame({ lease: 'held', orders_permitted: true, halted_scopes: [] }))
    expect(s.streamError).toBeNull()
    expect(s.snapshot?.halted_scopes).toEqual([])
  })

  it('caps the list', () => {
    let list: { event_id: string; occurred_at: string }[] = []
    for (let i = 0; i < 130; i++) {
      list = mergeEvents(list, { event_id: `e${i}`, occurred_at: `2026-10-03T10:${String(i % 60).padStart(2, '0')}:00Z` })
    }
    expect(list.length).toBe(100)
  })
})

describe('what announces, and what only notifies', () => {
  it('announces a trip as critical and allows a notification', () => {
    const f = parseMonitorFrame(event())
    const a = f.kind === 'monitor_event' ? eventAnnouncement(f.event) : null
    expect(a).toMatchObject({ severity: 'critical', notify: true })
    expect(a?.text).toMatch(/Kill switch tripped: platform-wide/)
  })

  it('announces a drill marked DRILL and never notifies', () => {
    const f = parseMonitorFrame(event({ kind: 'drill', is_drill: true }))
    const a = f.kind === 'monitor_event' ? eventAnnouncement(f.event) : null
    expect(a?.text).toMatch(/^DRILL:/)
    expect(a?.notify).toBe(false)
  })

  it('a drill that looks like a trip is still a drill', () => {
    const f = parseMonitorFrame(event({ kind: 'tripped', is_drill: true, scope: 'drill:weekly' }))
    const a = f.kind === 'monitor_event' ? eventAnnouncement(f.event) : null
    expect(a?.text).toMatch(/^DRILL:/)
    expect(a?.notify).toBe(false)
  })

  it('says nothing for a blip', () => {
    const f = parseMonitorFrame(event({ kind: 'blip' }))
    expect(f.kind === 'monitor_event' && eventAnnouncement(f.event)).toBeNull()
  })

  it('ranks halts and trips above breaches above the rest', () => {
    expect(eventSeverity('halted')).toBe('critical')
    expect(eventSeverity('breach')).toBe('warning')
    expect(eventSeverity('resumed')).toBe('info')
  })
})

describe('verdicts and ages never flatter', () => {
  it('shows an unrecognised feed verdict as itself, in a warning tone', () => {
    const v = feedVerdict('quarantined')
    expect(v.word).toBe('quarantined')
    expect(v.tone).toBe('warn')
  })

  it('marks unmeasurable and a silent producer as not fresh', () => {
    expect(feedVerdict('unmeasurable').tone).not.toBe('pos')
    expect(feedVerdict('producer_silent').tone).toBe('neg')
    expect(feedVerdict('fresh').tone).toBe('pos')
  })

  it('an age nobody measured is not zero seconds', () => {
    expect(ageWords(null)).toBe('not recorded')
    expect(ageWords(undefined)).toBe('not recorded')
    expect(ageWords(0)).toBe('0 s')
    expect(ageWords(7200)).toBe('2 h')
    expect(ageWords(86400 * 5)).toBe('5 days')
  })

  it('a stream that never connected has no age to print', () => {
    expect(streamWords('stale', false, null, null)).toMatch(/has not connected yet/)
    expect(streamWords('stale', true, 12, null)).toMatch(/dropped 12 s ago/)
    expect(streamWords('live', false, null, null)).toMatch(/waiting for the first frame/)
    expect(streamWords('closed', false, null, 'not signed in')).toMatch(/not signed in/)
  })

  it('labels bar timeframes', () => {
    expect([60, 300, 3600, 86400, 45].map(timeframeWords)).toEqual(['1m', '5m', '1h', '1d', '45s'])
  })
})

describe('an event carries the number that tripped it and the limit (S-7.10)', () => {
  it('reads observed_value and limit_value off the frame, null when absent', () => {
    const f = parseMonitorFrame(event({ metric: 'drawdown', observed_value: 0.42, limit_value: 0.3 }))
    expect(f.kind === 'monitor_event' && [f.event.observed_value, f.event.limit_value]).toEqual([0.42, 0.3])
    const bare = parseMonitorFrame(event())
    // Not recorded is null, never 0: a 0 would read as a limit nobody set.
    expect(bare.kind === 'monitor_event' && [bare.event.observed_value, bare.event.limit_value]).toEqual([null, null])
  })

  it('puts both numbers in the announcement detail', () => {
    const f = parseMonitorFrame(event({ metric: 'drawdown', observed_value: 0.42, limit_value: 0.3 }))
    const a = f.kind === 'monitor_event' ? eventAnnouncement(f.event) : null
    expect(a?.detail).toContain('0.42 against a limit of 0.3')
  })
})
