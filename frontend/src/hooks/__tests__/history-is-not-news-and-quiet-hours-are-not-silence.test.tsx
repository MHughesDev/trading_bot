/**
 * The live region, end to end (AGENT-012 §15; Set S task S-6.22).
 *
 * Before this, one `aria-live` region existed on the whole platform — the
 * kill-switch banner's — and the desk announced nothing at all. A person using a
 * screen reader watched a run by reading the card stream and had no way to learn
 * that a new approval had appeared behind where they were reading.
 *
 * Three claims are tested here and each one is a thing the obvious
 * implementation gets wrong:
 *
 *  1. **History is not news.** The naive hook announces every approval it finds
 *     on mount, so opening a conversation with nine pending approvals shouts
 *     nine times, in a channel the person cannot skim or scroll back through.
 *  2. **A run that stopped answering did not finish.** See
 *     `lib/desk/__tests__/a-run-that-stopped-answering-did-not-finish.test.ts`;
 *     asserted again here through the hook, because the call site is where the
 *     `=== true` collapse would be reintroduced.
 *  3. **Quiet hours silence the browser, never the screen reader.** They are a
 *     setting about being interrupted when you are *away*. Applying them to the
 *     live region would mean a blind user's own notification preference muted
 *     the only channel telling them what the agent is doing, between 22:00 and
 *     07:00, on a research desk built for long unattended runs.
 *
 * WHICH OF THESE WOULD HAVE FAILED BEFORE: all of them — `useDeskAnnouncer`,
 * `LiveRegion` and `lib/announce.ts` are new, and `grep -rn 'aria-live' src`
 * before this change returned the kill-switch banner, the toast container, the
 * environment region, two primitives and the strategy validation console, and
 * nothing that ever said the word "approval".
 */
import { render, screen, act } from '@testing-library/react'
import type { ReactElement } from 'react'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'

import type { DeskCard } from '@/api/desk'
import { LiveRegion } from '@/components/LiveRegion'
import { useDeskAnnouncer } from '../useDeskAnnouncer'
import { resetAnnouncer } from '@/lib/announce'
import { PREF_DEFAULTS, usePrefs } from '@/store/prefs'

function card(seq: number, kind: DeskCard['kind'], headline: string): DeskCard {
  return { seq, step: 1, kind, at: '2026-09-22T10:00:00Z', headline }
}

function Harness(props: {
  conversationId: string
  cards: DeskCard[]
  running: boolean | null
  title: string | null
}) {
  useDeskAnnouncer(props)
  return null
}

function mount(props: Parameters<typeof Harness>[0]) {
  return render(
    <>
      <LiveRegion />
      <Harness {...props} />
    </>,
  )
}

function regionText() {
  return screen.getByTestId('live-region').textContent ?? ''
}

beforeEach(() => {
  resetAnnouncer()
  usePrefs.setState({ ...PREF_DEFAULTS })
})

afterEach(() => {
  resetAnnouncer()
})

describe('opening a conversation is not news', () => {
  it('announces nothing for approvals that were already there', () => {
    mount({
      conversationId: 'c1',
      cards: [
        card(1, 'approval', 'spend a holdout look'),
        card(2, 'approval', 'promote to paper'),
        card(3, 'question', 'which venue?'),
      ],
      running: true,
      title: 'carry',
    })
    expect(regionText()).toBe('')
  })
})

describe('a new approval and a new question are announced once each', () => {
  it('announces the card that arrived and not the ones that were there', () => {
    const { rerender } = mount({ conversationId: 'c1', cards: [card(1, 'approval', 'old one')], running: true, title: 't' })
    expect(regionText()).toBe('')

    act(() => {
      rerender(
        <>
          <LiveRegion />
          <Harness
            conversationId="c1"
            cards={[card(1, 'approval', 'old one'), card(2, 'approval', 'spend a holdout look')]}
            running={true}
            title="t"
          />
        </>,
      )
    })
    expect(regionText()).toContain('Waiting on you: spend a holdout look')
    expect(regionText()).not.toContain('old one')
  })

  it("announces a question in the agent's own words", () => {
    const { rerender } = mount({ conversationId: 'c1', cards: [], running: true, title: 't' })
    act(() => {
      rerender(
        <>
          <LiveRegion />
          <Harness
            conversationId="c1"
            cards={[card(9, 'question', 'which venue should this assume?')]}
            running={true}
            title="t"
          />
        </>,
      )
    })
    expect(regionText()).toContain('The agent asked: which venue should this assume?')
  })

  it('does not announce the same card twice when the list is rebuilt', () => {
    const cards = [card(1, 'approval', 'a')]
    const { rerender } = mount({ conversationId: 'c1', cards: [], running: true, title: 't' })
    const again = (cs: DeskCard[]) =>
      act(() => {
        rerender(
          <>
            <LiveRegion />
            <Harness conversationId="c1" cards={cs} running={true} title="t" />
          </>,
        )
      })
    again(cards)
    // `mergeCards` returns a new array on every stream frame, so the hook sees a
    // fresh identity with the same contents constantly.
    again([...cards])
    again([...cards])
    expect(regionText().match(/Waiting on you: a/g)).toHaveLength(1)
  })
})

describe('a run that stopped answering did not finish', () => {
  const swap = (rerender: (ui: ReactElement) => void, running: boolean | null) =>
    act(() => {
      rerender(
        <>
          <LiveRegion />
          <Harness conversationId="c1" cards={[]} running={running} title="funding carry" />
        </>,
      )
    })

  it('announces a genuine finish', () => {
    const { rerender } = mount({ conversationId: 'c1', cards: [], running: true, title: 'funding carry' })
    swap(rerender, false)
    expect(regionText()).toContain('Run finished: funding carry')
  })

  it('says nothing when the conversation became unreadable', () => {
    const { rerender } = mount({ conversationId: 'c1', cards: [], running: true, title: 'funding carry' })
    swap(rerender, null)
    expect(regionText()).toBe('')
  })

  it('says nothing when a different conversation is opened on an idle run', () => {
    const { rerender } = mount({ conversationId: 'c1', cards: [], running: true, title: 'a' })
    act(() => {
      rerender(
        <>
          <LiveRegion />
          <Harness conversationId="c2" cards={[]} running={false} title="b" />
        </>,
      )
    })
    expect(regionText()).toBe('')
  })
})

describe('quiet hours silence the browser, never the screen reader', () => {
  /** A granted browser that records what it was asked to post. */
  function stubNotification() {
    const posted: string[] = []
    class N {
      static permission = 'granted'
      static requestPermission = async () => 'granted'
      constructor(title: string) {
        posted.push(title)
      }
    }
    Object.defineProperty(window, 'Notification', { value: N, configurable: true, writable: true })
    Object.defineProperty(window, 'isSecureContext', { value: true, configurable: true, writable: true })
    return posted
  }

  function arrive() {
    const { rerender } = mount({ conversationId: 'c1', cards: [], running: true, title: 't' })
    act(() => {
      rerender(
        <>
          <LiveRegion />
          <Harness conversationId="c1" cards={[card(1, 'approval', 'spend a holdout look')]} running={true} title="t" />
        </>,
      )
    })
  }

  afterEach(() => {
    Reflect.deleteProperty(window, 'Notification')
  })

  it('announces inside the quiet window and posts nothing', () => {
    const posted = stubNotification()
    usePrefs.setState({
      browserNotifications: true,
      quietHoursEnabled: true,
      quietHoursStart: '00:00',
      quietHoursEnd: '23:59',
    })
    arrive()
    expect(regionText()).toContain('Waiting on you: spend a holdout look')
    expect(posted).toEqual([])
  })

  it('posts nothing at all while the page is in front of the person', () => {
    // jsdom reports `visible`, which is the situation this gate exists for: the
    // card is on screen and the live region has already said it.
    const posted = stubNotification()
    usePrefs.setState({ browserNotifications: true, quietHoursEnabled: false })
    arrive()
    expect(regionText()).toContain('Waiting on you: spend a holdout look')
    expect(posted).toEqual([])
  })

  it('posts one when the tab is hidden and nothing is holding it', () => {
    const posted = stubNotification()
    Object.defineProperty(document, 'visibilityState', { value: 'hidden', configurable: true })
    usePrefs.setState({ browserNotifications: true, quietHoursEnabled: false })
    arrive()
    expect(posted).toEqual(['Waiting on you: spend a holdout look'])
    Object.defineProperty(document, 'visibilityState', { value: 'visible', configurable: true })
  })

  it('posts nothing when the opt-in is off, which is the default', () => {
    const posted = stubNotification()
    Object.defineProperty(document, 'visibilityState', { value: 'hidden', configurable: true })
    expect(PREF_DEFAULTS.browserNotifications).toBe(false)
    arrive()
    expect(regionText()).toContain('Waiting on you: spend a holdout look')
    expect(posted).toEqual([])
    Object.defineProperty(document, 'visibilityState', { value: 'visible', configurable: true })
  })
})

describe('the region exists before there is anything to say', () => {
  it('is in the document on the first render, empty', () => {
    // A live region inserted at the same moment as its text is announced by no
    // screen reader. This is why it is mounted in the app shell rather than
    // rendered beside the announcement.
    render(<LiveRegion />)
    const el = screen.getByTestId('live-region')
    expect(el).toBeInTheDocument()
    expect(el).toHaveAttribute('aria-live', 'polite')
    expect(el.textContent).toBe('')
  })
})
