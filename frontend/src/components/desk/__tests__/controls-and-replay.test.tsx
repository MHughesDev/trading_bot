// The controls work mid-call (§19.3), the replay is byte-for-byte (§19.4), the
// stream does not duplicate on resume (S-6.2), and the digest covers exactly
// the gap (S-6.24).
import { render, screen, fireEvent } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import type { DigestLine, PromptSection, TrajectoryStep } from '@/api/desk'
import { assemblePrompt, groupByStep, promptIsFaithful } from '@/api/desk'
import { mergeCards, toCards, type WireEvent } from '@/lib/desk/events'
import { parseFrame } from '@/hooks/useDeskStream'
import { ControlsRow, useDeskKeymap, type DeskControls } from '../ControlsRow'
import { linesInGap, shouldShowDigest, MIN_GAP_MS } from '../AwayDigest'
import { diffLines } from '../RunDetail'

function harness(overrides: Partial<DeskControls> = {}) {
  const spies = {
    onPause: vi.fn(),
    onResume: vi.fn(),
    onStop: vi.fn(),
    onSteer: vi.fn(),
    onApprove: vi.fn(),
    onDeny: vi.fn(),
    onEscalate: vi.fn(),
    onBudget: vi.fn(),
    onToggleDetails: vi.fn(),
    onReplay: vi.fn(),
    onFocusComposer: vi.fn(),
    onShortcuts: vi.fn(),
  }
  const props: DeskControls = {
    paused: false,
    // Mid-call. Nothing below may be disabled by this.
    busy: true,
    disconnected: false,
    hasPendingApproval: true,
    detailsOpen: false,
    ...spies,
    ...overrides,
  }
  function Host() {
    useDeskKeymap(props)
    return <ControlsRow {...props} />
  }
  render(<Host />)
  return spies
}

describe('every control works from the keyboard while the agent is mid-call (§19.3)', () => {
  it('space pauses, a approves, x denies, s steers, d toggles details, r replays', () => {
    const s = harness()
    fireEvent.keyDown(document, { key: ' ' })
    expect(s.onPause).toHaveBeenCalled()
    fireEvent.keyDown(document, { key: 'a' })
    expect(s.onApprove).toHaveBeenCalled()
    fireEvent.keyDown(document, { key: 'x' })
    expect(s.onDeny).toHaveBeenCalled()
    fireEvent.keyDown(document, { key: 's' })
    expect(s.onSteer).toHaveBeenCalled()
    fireEvent.keyDown(document, { key: 'd' })
    expect(s.onToggleDetails).toHaveBeenCalled()
    fireEvent.keyDown(document, { key: 'r' })
    expect(s.onReplay).toHaveBeenCalled()
    fireEvent.keyDown(document, { key: '?' })
    expect(s.onShortcuts).toHaveBeenCalled()
  })

  it('space resumes when paused', () => {
    const s = harness({ paused: true })
    fireEvent.keyDown(document, { key: ' ' })
    expect(s.onResume).toHaveBeenCalled()
    expect(s.onPause).not.toHaveBeenCalled()
  })

  it('esc stops even from inside a text field', () => {
    const s = harness()
    const input = document.createElement('input')
    document.body.appendChild(input)
    fireEvent.keyDown(input, { key: 'Escape' })
    expect(s.onStop).toHaveBeenCalled()
    input.remove()
  })

  it('a letter typed into a field is text, not a shortcut', () => {
    const s = harness()
    const input = document.createElement('input')
    document.body.appendChild(input)
    fireEvent.keyDown(input, { key: 's' })
    expect(s.onSteer).not.toHaveBeenCalled()
    input.remove()
  })

  it('no control is disabled by the agent being busy', () => {
    harness()
    expect((screen.getByTestId('pause') as HTMLButtonElement).disabled).toBe(false)
    expect((screen.getByTestId('stop') as HTMLButtonElement).disabled).toBe(false)
    expect((screen.getByTestId('steer') as HTMLButtonElement).disabled).toBe(false)
  })

  it('steer is disabled only when the stream that would carry it is down', () => {
    harness({ disconnected: true })
    expect((screen.getByTestId('steer') as HTMLButtonElement).disabled).toBe(true)
    expect((screen.getByTestId('pause') as HTMLButtonElement).disabled).toBe(false)
  })
})

describe('the replay scrubber reproduces the prompt byte-for-byte (§19.4)', () => {
  const sections: PromptSection[] = [
    { layer: 'charter', text: 'CHARTER\n  indented\ttab\n', tokens: 8 },
    { layer: 'working_state', text: 'STATE\r\n<not markup>\n', tokens: 6 },
    { layer: 'schemas', text: '{"tool":"get_bars"}\n\n', tokens: 9 },
  ]
  const step: TrajectoryStep = {
    step: 3,
    at: new Date().toISOString(),
    sections,
    prompt: sections.map((s) => s.text).join(''),
    reply: 'ok',
    state: null,
  }

  it('reassembles the layers with no separator, trim or normalisation', () => {
    expect(assemblePrompt(sections)).toBe(step.prompt)
    const a = new TextEncoder().encode(assemblePrompt(sections))
    const b = new TextEncoder().encode(step.prompt)
    expect(Array.from(a)).toEqual(Array.from(b))
    expect(promptIsFaithful(step)).toBe(true)
  })

  it('reports rather than reconstructs when the layers do not reassemble', () => {
    const lying: TrajectoryStep = { ...step, prompt: step.prompt + 'extra the layers do not carry' }
    expect(promptIsFaithful(lying)).toBe(false)
  })

  it('diffs by line against the previous step', () => {
    const d = diffLines('a\nb\n', 'a\nc\n')
    expect(d.filter((l) => l.sign === '-').map((l) => l.text)).toEqual(['b'])
    expect(d.filter((l) => l.sign === '+').map((l) => l.text)).toEqual(['c'])
  })
})

describe('the stream resumes without duplicating (S-6.2)', () => {
  const at = new Date().toISOString()
  const wire: WireEvent[] = [
    { seq: 1, kind: 'user', content: { text: 'hello' }, created_at: at },
    { seq: 2, kind: 'plan', content: { goal: 'look at bars', steps: [] }, created_at: at },
    { seq: 3, kind: 'tool_call', content: { name: 'get_bars' }, created_at: at },
  ]

  it('parses an SSE frame into its id and data', () => {
    const f = parseFrame('id: 42\ndata: {"seq":1}\n')
    expect(f.id).toBe('42')
    expect(f.data).toBe('{"seq":1}')
  })

  it('a replayed frame updates rather than appends', () => {
    const first = toCards(wire)
    const replayed = toCards([wire[2], { seq: 4, kind: 'tool_result', content: { name: 'get_bars' }, created_at: at }])
    const merged = mergeCards(first, replayed)
    expect(merged.map((c) => c.seq)).toEqual([1, 2, 3, 4])
  })

  it('groups cards by step under one header', () => {
    const groups = groupByStep(toCards(wire))
    expect(groups.length).toBe(1)
    expect(groups[0].tools).toBe(1)
  })

  it('folds raw kinds behind the disclosure rather than into the stream', () => {
    const cards = toCards([
      ...wire,
      { seq: 9, kind: 'exposure', content: { tools: [] }, created_at: at },
      { seq: 10, kind: 'usage', content: {}, created_at: at },
    ])
    const groups = groupByStep(cards)
    expect(groups[0].raw.map((c) => c.headline)).toEqual(['exposure', 'usage'])
  })
})

describe('"Since you were away" covers exactly the gap (S-6.24)', () => {
  const now = Date.UTC(2026, 8, 15, 12, 0, 0)
  const lines: DigestLine[] = [
    { id: '1', at: new Date(now - 30 * 60_000).toISOString(), kind: 'concluded', text: 'refuted', href: null },
    { id: '2', at: new Date(now - 5 * 60 * 60_000).toISOString(), kind: 'finding', text: 'established', href: null },
    { id: '3', at: new Date(now - 40 * 60 * 60_000).toISOString(), kind: 'budget', text: 'spent $3', href: null },
  ]

  it('is absent for a gap under an hour', () => {
    expect(shouldShowDigest(now - 59 * 60_000, now)).toBe(false)
    expect(shouldShowDigest(now - MIN_GAP_MS, now)).toBe(true)
  })

  it('drops lines older than the last visit', () => {
    const since = now - 6 * 60 * 60_000
    expect(linesInGap(lines, since, now).map((l) => l.id)).toEqual(['1', '2'])
  })
})
