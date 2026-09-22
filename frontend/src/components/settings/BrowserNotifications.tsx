// Browser-notification opt-in and quiet hours (AGENT-012 §13, §15; Set S task
// S-6.22's second clause).
//
// # What this panel promises, and the two things it refuses to promise
//
// It promises an OS-level notification for the three things the desk announces:
// a new approval, a new question, a run that finished. It refuses to imply
// **push** — there is no service worker on this origin and no push subscription
// anywhere on the platform, so a closed tab gets nothing, and the panel says
// that rather than leaving the reader to assume the usual meaning of the words.
// It also refuses to imply that a notification arrives while you are looking at
// the page: the announcement goes to the live region instead, which is the
// correct channel for someone who is already here.
//
// # Why there is no "enable" button on a denied browser
//
// `Notification.requestPermission()` on a `denied` origin resolves immediately
// with `denied` and shows the user nothing. A button wired to it is a control
// whose callee cannot succeed — it passes every test and does nothing, and the
// person who clicks it concludes the product is broken rather than that their
// browser is blocking it. So the `denied` state renders a **named absence**: the
// sentence that says only the browser's own site settings can lift it.
//
// # Why the test button exists
//
// The delivery path is invisible by construction — it fires when the tab is in
// the background. An opt-in whose effect can never be observed from the page
// that offers it is indistinguishable from one that is broken. The test button
// is the one place that bypasses the visibility gate, and it reports the
// decision it got rather than silently doing nothing, so quiet hours and a
// malformed window are both visible where they are set.
import { useEffect, useState } from 'react'
import { Bell, BellOff } from 'lucide-react'

import { Button } from '@/components/primitives/Button'
import { Panel, PanelBody, PanelHeader } from '@/components/primitives/Panel'
import { Field, Input, SwitchRow } from '@/components/primitives/Field'
import { usePrefs } from '@/store/prefs'
import {
  REFUSAL_TEXT,
  deliverNotification,
  minutesOfDay,
  notifyState,
  requestNotifyPermission,
  type NotifyState,
} from '@/lib/notify'

/** One sentence per state, and every one of them says what to do next. */
const STATE_TEXT: Record<NotifyState, string> = {
  unsupported:
    'This browser has no Notification API, so nothing on this page can post one. Announcements still reach screen readers through the live region.',
  insecure:
    'This page is not a secure context — the Notification API only works over HTTPS or on localhost. Reach the platform over HTTPS, or over localhost on this machine.',
  denied:
    'This browser has refused notifications for this site. Nothing here can ask again: the block is a browser setting, and only you can lift it from the padlock or the site-settings panel in the address bar.',
  default: 'This browser has not been asked yet.',
  granted: 'This browser will post notifications for this site.',
}

export function BrowserNotifications() {
  const prefs = usePrefs()
  const [state, setState] = useState<NotifyState>(() => notifyState())
  const [asking, setAsking] = useState(false)
  const [testResult, setTestResult] = useState<string | null>(null)

  // Re-read on focus. Permission is changed in browser chrome, not here, so the
  // panel can be looking at a stale answer the moment the user comes back from
  // the site-settings popover — and a stale `denied` would keep showing the
  // sentence that says to go and change it.
  useEffect(() => {
    const refresh = () => setState(notifyState())
    window.addEventListener('focus', refresh)
    document.addEventListener('visibilitychange', refresh)
    return () => {
      window.removeEventListener('focus', refresh)
      document.removeEventListener('visibilitychange', refresh)
    }
  }, [])

  const startMins = minutesOfDay(prefs.quietHoursStart)
  const endMins = minutesOfDay(prefs.quietHoursEnd)
  const windowUnreadable = prefs.quietHoursEnabled && (startMins === null || endMins === null)
  const windowEmpty =
    prefs.quietHoursEnabled && startMins !== null && endMins !== null && startMins === endMins

  return (
    <Panel>
      <PanelHeader title="Browser notifications" />
      <PanelBody>
        <p className="sub" data-testid="notify-state">
          {STATE_TEXT[state]}
        </p>

        {state === 'default' && (
          <Button
            size="xs"
            variant="primary"
            icon={<Bell size={11} aria-hidden />}
            disabled={asking}
            onClick={() => {
              setAsking(true)
              void requestNotifyPermission().then((s) => {
                setState(s)
                setAsking(false)
                // Granting permission is not the same as asking for them: the
                // browser now *may*, and this switch is where the person says
                // it *should*. Turned on here because clicking "Allow" on the
                // prompt this button raised is an unambiguous yes.
                if (s === 'granted') prefs.set('browserNotifications', true)
              })
            }}
          >
            {asking ? 'Asking…' : 'Ask this browser for permission'}
          </Button>
        )}

        {/* Drawn only where it can work. A greyed-out switch under
            "unsupported", "insecure" or "denied" would be a control with no
            sentence attached — the shape this phase keeps deleting — and the
            three states above each already say what is true and what to do.
            Under `default` the Ask button is the control; a switch beside it
            would offer two ways to mean the same thing, one of which silently
            does nothing until permission exists. */}
        {state === 'granted' && (
          <SwitchRow
            label="Post a notification for desk events"
            description="A new approval, a new question, or a run finishing — while this tab is in the background."
            checked={prefs.browserNotifications}
            onCheckedChange={(v) => prefs.set('browserNotifications', v)}
          />
        )}

        <div className="hr" />

        <SwitchRow
          label="Quiet hours"
          description="Hold browser notifications inside a window. Screen-reader announcements are never held — you are on the page."
          checked={prefs.quietHoursEnabled}
          onCheckedChange={(v) => prefs.set('quietHoursEnabled', v)}
        />

        {prefs.quietHoursEnabled && (
          <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 'var(--s-3)' }}>
            <Field
              label="From"
              error={startMins === null ? 'Not a readable HH:MM' : null}
              hint="24-hour, on this device's clock"
            >
              <Input
                type="time"
                small
                value={prefs.quietHoursStart}
                aria-label="Quiet hours start"
                onChange={(e) => prefs.set('quietHoursStart', e.target.value)}
              />
            </Field>
            <Field
              label="To"
              error={endMins === null ? 'Not a readable HH:MM' : null}
              hint="A window may cross midnight"
            >
              <Input
                type="time"
                small
                value={prefs.quietHoursEnd}
                aria-label="Quiet hours end"
                onChange={(e) => prefs.set('quietHoursEnd', e.target.value)}
              />
            </Field>
          </div>
        )}

        {windowUnreadable && (
          <p className="callout warn" role="alert" data-testid="quiet-hours-unreadable">
            <BellOff size={13} aria-hidden style={{ flex: 'none', marginTop: 2 }} />
            <span>
              Quiet hours are on and this window is not readable, so <strong>nothing</strong> is
              being posted. An unreadable start time is not midnight and it is not ignored — fix
              both fields, or turn quiet hours off.
            </span>
          </p>
        )}

        {windowEmpty && (
          <p className="callout neutral" data-testid="quiet-hours-empty">
            <BellOff size={13} aria-hidden style={{ flex: 'none', marginTop: 2 }} />
            <span>
              This window starts and ends at the same minute, which silences nothing. It is read
              as a zero-length window rather than as a whole day.
            </span>
          </p>
        )}

        <div className="hr" />

        <div className="desk-actions">
          <Button
            size="xs"
            variant="ghost"
            onClick={() => {
              const d = deliverNotification('Test notification', 'From your trading platform', {
                enabled: prefs.browserNotifications,
                state: notifyState(),
                quietHours: {
                  enabled: prefs.quietHoursEnabled,
                  start: prefs.quietHoursStart,
                  end: prefs.quietHoursEnd,
                },
                now: new Date(),
                // The one call that does not check visibility: you asked to see
                // one, and you are by definition looking at the page.
                visibility: null,
              })
              setTestResult(d.deliver ? 'Posted. If you saw nothing, your OS is suppressing it.' : REFUSAL_TEXT[d.why])
            }}
          >
            Send a test notification
          </Button>
        </div>
        {testResult && (
          <p className="sub" role="status" data-testid="notify-test-result">
            {testResult}
          </p>
        )}

        <p className="callout neutral" style={{ marginTop: 'var(--s-4)' }}>
          <Bell size={13} aria-hidden style={{ flex: 'none', marginTop: 2 }} />
          <span>
            These come from an <strong>open tab</strong>. There is no service worker and no push
            subscription on this platform, so nothing is delivered once you close the page — a
            long research run that finishes overnight is in the “Since you were away” digest, not
            in your notification tray.
          </span>
        </p>
      </PanelBody>
    </Panel>
  )
}
