/**
 * The browser-notification opt-in (AGENT-012 §13; Set S task S-6.22's second
 * clause — "notifications (in-app, browser opt-in, quiet hours)").
 *
 * The defect this panel is written against is the one that has dominated this
 * programme: a control whose callee cannot succeed.
 * `Notification.requestPermission()` on an origin the user has blocked resolves
 * immediately with `denied` and shows them nothing at all. An **Enable
 * notifications** button wired to it therefore passes every test, reaches a real
 * function, and does nothing — and the person who clicks it concludes the
 * product is broken rather than that their browser is blocking it. A *disabled*
 * button with no explanation is no better: it is the same non-answer with a grey
 * background.
 *
 * So each of the five states renders a sentence that is true and says what to do
 * next, and the control appears only where it can work.
 *
 * The second half is the test button, which exists because the delivery path is
 * invisible by construction — notifications fire when the tab is in the
 * background. An opt-in whose effect cannot be observed from the page that
 * offers it is indistinguishable from a broken one, so there is one control that
 * bypasses the visibility gate and **reports the refusal it got** rather than
 * silently doing nothing.
 *
 * WHICH OF THESE WOULD HAVE FAILED BEFORE: all of them.
 * `components/settings/BrowserNotifications.tsx` is new; before it, the
 * Notifications section said "In-app only for now" and the platform had no
 * browser-notification opt-in and no quiet hours at all — which is what kept
 * S-6.22 `PARTIAL`.
 */
import { render, screen, fireEvent } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'

import { BrowserNotifications } from '../BrowserNotifications'
import { PREF_DEFAULTS, usePrefs } from '@/store/prefs'

function browser(permission: string | null, secure = true) {
  if (permission === null) {
    Reflect.deleteProperty(window, 'Notification')
  } else {
    const posted: string[] = []
    class N {
      static permission = permission
      static requestPermission = async () => permission
      constructor(title: string) {
        posted.push(title)
      }
    }
    Object.defineProperty(window, 'Notification', { value: N, configurable: true, writable: true })
    Object.defineProperty(window, 'isSecureContext', { value: secure, configurable: true, writable: true })
    return posted
  }
  Object.defineProperty(window, 'isSecureContext', { value: secure, configurable: true, writable: true })
  return [] as string[]
}

beforeEach(() => {
  usePrefs.setState({ ...PREF_DEFAULTS })
})

afterEach(() => {
  Reflect.deleteProperty(window, 'Notification')
})

describe('a browser that has refused gets a sentence, not a button', () => {
  it('offers no control at all, because none of them could work', () => {
    browser('denied')
    render(<BrowserNotifications />)
    expect(screen.queryByRole('button', { name: /ask this browser/i })).toBeNull()
    expect(screen.queryByRole('switch', { name: /Post a notification/i })).toBeNull()
  })

  it('names the browser as the thing refusing, and where the user can lift it', () => {
    browser('denied')
    render(<BrowserNotifications />)
    const state = screen.getByTestId('notify-state')
    expect(state).toHaveTextContent(/refused notifications for this site/i)
    expect(state).toHaveTextContent(/Nothing here can ask again/i)
    expect(state).toHaveTextContent(/site-settings/i)
  })
})

describe('a browser with no Notification API says so rather than offering to ask it', () => {
  it('draws neither the ask button nor the switch', () => {
    browser(null)
    render(<BrowserNotifications />)
    expect(screen.getByTestId('notify-state')).toHaveTextContent(/no Notification API/i)
    expect(screen.queryByRole('button', { name: /ask this browser/i })).toBeNull()
    expect(screen.queryByRole('switch', { name: /Post a notification/i })).toBeNull()
  })

  it('still promises the live region, which needs no permission', () => {
    browser(null)
    render(<BrowserNotifications />)
    expect(screen.getByTestId('notify-state')).toHaveTextContent(/screen readers/i)
  })
})

describe('a plain-HTTP page is not a browser that has never been asked', () => {
  it('reports the insecure context rather than offering a prompt that always fails', () => {
    // The platform is reached over `http://<lan-ip>:7080` from a second machine
    // and the constructor is present there. Read as `default` it would draw a
    // button whose only possible outcome is failure.
    browser('default', false)
    render(<BrowserNotifications />)
    expect(screen.getByTestId('notify-state')).toHaveTextContent(/not a secure context/i)
    expect(screen.queryByRole('button', { name: /ask this browser/i })).toBeNull()
  })
})

describe('the opt-in is an opt-in', () => {
  it('defaults to off', () => {
    expect(PREF_DEFAULTS.browserNotifications).toBe(false)
  })

  it('offers the ask button only where the browser can still be asked', () => {
    browser('default')
    render(<BrowserNotifications />)
    expect(screen.getByRole('button', { name: /ask this browser/i })).toBeInTheDocument()
  })

  it('offers the switch once permission exists', () => {
    browser('granted')
    render(<BrowserNotifications />)
    expect(screen.getByRole('switch', { name: /Post a notification/i })).toBeInTheDocument()
  })
})

describe('quiet hours that cannot be read are not quiet hours at midnight', () => {
  it('flags the broken field and says notifications are being held', () => {
    browser('granted')
    usePrefs.setState({ quietHoursEnabled: true, quietHoursStart: '', quietHoursEnd: '07:00' })
    render(<BrowserNotifications />)
    const warn = screen.getByTestId('quiet-hours-unreadable')
    expect(warn).toHaveAttribute('role', 'alert')
    expect(warn).toHaveTextContent(/not readable/i)
    expect(warn).toHaveTextContent(/is not midnight/i)
  })

  it('says a zero-length window silences nothing rather than everything', () => {
    browser('granted')
    usePrefs.setState({ quietHoursEnabled: true, quietHoursStart: '22:00', quietHoursEnd: '22:00' })
    render(<BrowserNotifications />)
    expect(screen.getByTestId('quiet-hours-empty')).toHaveTextContent(/silences nothing/i)
  })

  it('shows neither warning on a window that reads', () => {
    browser('granted')
    usePrefs.setState({ quietHoursEnabled: true, quietHoursStart: '22:00', quietHoursEnd: '07:00' })
    render(<BrowserNotifications />)
    expect(screen.queryByTestId('quiet-hours-unreadable')).toBeNull()
    expect(screen.queryByTestId('quiet-hours-empty')).toBeNull()
  })
})

describe('the test notification reports which of the four refusals it got', () => {
  function sendTest() {
    fireEvent.click(screen.getByRole('button', { name: /send a test notification/i }))
    return screen.getByTestId('notify-test-result').textContent ?? ''
  }

  it('posts one when everything is in order, and says it did', () => {
    const posted = browser('granted')
    usePrefs.setState({ browserNotifications: true })
    render(<BrowserNotifications />)
    expect(sendTest()).toMatch(/Posted/)
    expect(posted).toEqual(['Test notification'])
  })

  it('blames the opt-out when the switch is off', () => {
    const posted = browser('granted')
    render(<BrowserNotifications />)
    expect(sendTest()).toMatch(/Browser notifications are off/)
    expect(posted).toEqual([])
  })

  it('blames quiet hours when it is inside the window, rather than going silent', () => {
    // The refusal a person would otherwise diagnose as a broken feature.
    const posted = browser('granted')
    usePrefs.setState({
      browserNotifications: true,
      quietHoursEnabled: true,
      quietHoursStart: '00:00',
      quietHoursEnd: '23:59',
    })
    render(<BrowserNotifications />)
    expect(sendTest()).toMatch(/inside your quiet hours/)
    expect(posted).toEqual([])
  })

  it('blames the unreadable window by name, on the panel that owns the fields', () => {
    const posted = browser('granted')
    usePrefs.setState({
      browserNotifications: true,
      quietHoursEnabled: true,
      quietHoursStart: 'oops',
      quietHoursEnd: '07:00',
    })
    render(<BrowserNotifications />)
    expect(sendTest()).toMatch(/not a readable HH:MM/)
    expect(posted).toEqual([])
  })
})
