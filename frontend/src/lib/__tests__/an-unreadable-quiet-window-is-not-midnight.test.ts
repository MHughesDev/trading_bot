/**
 * Quiet hours, and the four separate reasons a notification does not appear
 * (AGENT-012 §13; Set S task S-6.22).
 *
 * The rule this file exists for is in its name. A quiet-hours window is stored
 * as two `HH:MM` strings, and the obvious ways to read one — `Number('')` is
 * `0`, `parseInt('7')` is `7`, `new Date('bad').getHours()` is `NaN` and
 * `NaN * 60` compares false against everything — all turn a window nobody can
 * read into a window that silences nothing, or into midnight. Both render as a
 * setting that is working. This platform has shipped that mistake twice with
 * numbers; a setting is the same mistake with a string.
 *
 * So `minutesOfDay` returns `null` and never a number it had to invent, and
 * `notifyDecision` refuses to deliver on an unreadable window rather than
 * guessing a default — with its own reason code, because the alternative is a
 * person whose notifications stopped and no sentence anywhere saying why.
 *
 * WHICH OF THESE WOULD HAVE FAILED BEFORE: all of them. `lib/notify.ts` did not
 * exist; S-6.22's browser-notification clause was unbuilt, and the plan file
 * recorded it as unbuilt.
 */
import { describe, expect, it } from 'vitest'

import {
  inQuietHours,
  minutesOfDay,
  notifyDecision,
  notifyState,
  type NotifyEnv,
  type NotifyOptions,
} from '@/lib/notify'

describe('a window that cannot be read is not a window at midnight', () => {
  it('refuses the empty string rather than reading it as 00:00', () => {
    expect(minutesOfDay('')).toBeNull()
  })

  it('refuses every shape that is not HH:MM', () => {
    for (const bad of ['7:00', '24:00', '22:60', '2200', 'nope', '22:0', null, undefined, 7]) {
      expect(minutesOfDay(bad)).toBeNull()
    }
  })

  it('still reads a real midnight as the number zero', () => {
    // The converse matters as much: `00:00` is a measurement and must not be
    // swept up by a truthiness check on the way out.
    expect(minutesOfDay('00:00')).toBe(0)
    expect(minutesOfDay('23:59')).toBe(23 * 60 + 59)
    expect(minutesOfDay(' 07:30 ')).toBe(450)
  })
})

describe('a quiet window may cross midnight, which is the only kind anyone sets', () => {
  const start = 22 * 60
  const end = 7 * 60

  it('is quiet at 02:00 inside a 22:00–07:00 window', () => {
    expect(inQuietHours(2 * 60, start, end)).toBe(true)
    expect(inQuietHours(23 * 60, start, end)).toBe(true)
  })

  it('is not quiet at midday', () => {
    expect(inQuietHours(12 * 60, start, end)).toBe(false)
  })

  it('treats the boundaries as half-open, so 07:00 is awake', () => {
    expect(inQuietHours(start, start, end)).toBe(true)
    expect(inQuietHours(end, start, end)).toBe(false)
  })

  it('handles a window that does not cross midnight', () => {
    expect(inQuietHours(13 * 60, 12 * 60, 14 * 60)).toBe(true)
    expect(inQuietHours(15 * 60, 12 * 60, 14 * 60)).toBe(false)
  })

  it('reads a zero-length window as silencing nothing, not as a whole day', () => {
    // The mis-set field has to be recoverable. Reading `22:00–22:00` as "always
    // quiet" would turn one bad keystroke into permanent silence with no error.
    expect(inQuietHours(2 * 60, start, start)).toBe(false)
    expect(inQuietHours(start, start, start)).toBe(false)
  })
})

function opts(over: Partial<NotifyOptions> = {}): NotifyOptions {
  return {
    enabled: true,
    state: 'granted',
    quietHours: { enabled: false, start: '22:00', end: '07:00' },
    now: new Date(2026, 8, 22, 12, 0),
    visibility: 'hidden',
    ...over,
  }
}

describe('every reason a notification did not appear is a different reason', () => {
  it('delivers when nothing is in the way', () => {
    expect(notifyDecision(opts())).toEqual({ deliver: true })
  })

  it('names an opt-out as an opt-out', () => {
    expect(notifyDecision(opts({ enabled: false }))).toEqual({ deliver: false, why: 'opted_out' })
  })

  it('names a browser refusal separately from an opt-out', () => {
    for (const state of ['denied', 'default', 'unsupported', 'insecure'] as const) {
      expect(notifyDecision(opts({ state }))).toEqual({ deliver: false, why: 'not_permitted' })
    }
  })

  it('does not interrupt a page the person is already looking at', () => {
    expect(notifyDecision(opts({ visibility: 'visible' }))).toEqual({
      deliver: false,
      why: 'page_visible',
    })
  })

  it('lets a deliberate test through with no document to ask', () => {
    expect(notifyDecision(opts({ visibility: null }))).toEqual({ deliver: true })
  })

  it('holds one inside the quiet window', () => {
    expect(
      notifyDecision(
        opts({
          quietHours: { enabled: true, start: '22:00', end: '07:00' },
          now: new Date(2026, 8, 22, 2, 30),
        }),
      ),
    ).toEqual({ deliver: false, why: 'quiet_hours' })
  })

  it('posts one outside it', () => {
    expect(
      notifyDecision(
        opts({
          quietHours: { enabled: true, start: '22:00', end: '07:00' },
          now: new Date(2026, 8, 22, 9, 0),
        }),
      ),
    ).toEqual({ deliver: true })
  })

  it('refuses an unreadable window with its own reason, rather than guessing one', () => {
    const d = notifyDecision(
      opts({
        quietHours: { enabled: true, start: '', end: '07:00' },
        now: new Date(2026, 8, 22, 12, 0),
      }),
    )
    // Not `deliver: true` (an unreadable window silently ignored) and not
    // `quiet_hours` (an unreadable window silently obeyed). A third answer,
    // because it is a third fact.
    expect(d).toEqual({ deliver: false, why: 'quiet_hours_unreadable' })
  })

  it('ignores an unreadable window when quiet hours are off', () => {
    expect(
      notifyDecision(opts({ quietHours: { enabled: false, start: '', end: '' } })),
    ).toEqual({ deliver: true })
  })
})

describe('what the browser will do is read, never assumed', () => {
  const N = (permission: string) =>
    ({ permission, requestPermission: async () => permission }) as unknown as NotifyEnv['Notification']

  it('reports a browser with no Notification API as unsupported', () => {
    expect(notifyState({})).toBe('unsupported')
    expect(notifyState({ Notification: undefined })).toBe('unsupported')
  })

  it('reports a plain-HTTP page as insecure rather than as askable', () => {
    // The platform is reached over `http://<lan-ip>:7080` from a second
    // machine. The constructor exists there and every request fails, which
    // would otherwise render as `default` and offer a button that cannot work.
    expect(notifyState({ Notification: N('default'), isSecureContext: false })).toBe('insecure')
  })

  it('separates granted, denied and never-asked', () => {
    expect(notifyState({ Notification: N('granted'), isSecureContext: true })).toBe('granted')
    expect(notifyState({ Notification: N('denied'), isSecureContext: true })).toBe('denied')
    expect(notifyState({ Notification: N('default'), isSecureContext: true })).toBe('default')
  })
})
