// The one subscriber to `GET /api/monitor/stream` (Set S S-7.10).
//
// Mounted once in the shell, beside `KillSwitchBanner`, for the same structural
// reason: "the desk reflects a trip within one event" is a property of the
// frame around every route, not of a page that has to remember to subscribe.
//
// It does three things with each frame and nothing else:
//
//   1. writes it into the shared live state (`MONITOR_LIVE_KEY`), which the
//      Monitor page and anything else reads;
//   2. announces it — the polite live region always, an OS notification only
//      through `notifyDecision`, which can refuse it four ways (opted out, not
//      permitted, page in front of the person, quiet hours). A drill never
//      posts a notification: see `eventAnnouncement`;
//   3. asks the queries that show monitor state to re-read, because the event
//      is the news that they changed.
//
// It renders nothing. A subscription with a visible control would be a
// subscription somebody can switch off, and this is the channel the operator is
// told about a trip on.
import { useCallback, useEffect, useRef } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import { MONITOR_STREAM } from '@/api/ops'
import { useDeskStream } from '@/hooks/useDeskStream'
import type { WireEvent } from '@/lib/desk/events'
import { announce } from '@/lib/announce'
import { deliverNotification, notifyState, pageVisibility } from '@/lib/notify'
import { usePrefs } from '@/store/prefs'
import {
  MONITOR_LIVE_KEY,
  applyFrame,
  emptyLiveState,
  eventAnnouncement,
  parseMonitorFrame,
  type MonitorLiveState,
} from '@/lib/ops/monitor'

export function MonitorNotifier() {
  const qc = useQueryClient()
  const announced = useRef<Set<string>>(new Set())

  const onEvent = useCallback(
    (e: WireEvent) => {
      // The hook parsed the JSON for its own event type; the frame is whatever
      // the monitor sent, and `parseMonitorFrame` is where that is checked.
      const frame = parseMonitorFrame(e as unknown)
      if (frame.kind === 'unknown') return
      qc.setQueryData<MonitorLiveState>(MONITOR_LIVE_KEY, (prev) =>
        applyFrame(prev ?? emptyLiveState(), frame),
      )
      if (frame.kind !== 'monitor_event') return

      // A state change: the tables that show it are stale now.
      void qc.invalidateQueries({ queryKey: ['desk', 'monitor'] })
      void qc.invalidateQueries({ queryKey: ['desk', 'kill-switch'] })
      void qc.invalidateQueries({ queryKey: ['monitor', 'ops'] })
      void qc.invalidateQueries({ queryKey: ['alerts', 'inbox'] })

      const a = eventAnnouncement(frame.event)
      if (!a || announced.current.has(a.key)) return
      announced.current.add(a.key)
      announce(a.text)
      if (!a.notify) return
      const p = usePrefs.getState()
      deliverNotification(a.text, a.detail, {
        enabled: p.browserNotifications,
        state: notifyState(),
        quietHours: { enabled: p.quietHoursEnabled, start: p.quietHoursStart, end: p.quietHoursEnd },
        now: new Date(),
        visibility: pageVisibility(),
      })
    },
    [qc],
  )

  const stream = useDeskStream({ url: MONITOR_STREAM, onEvent })

  // The connection's own state, kept beside the frames so the page can say
  // whether silence means "nothing happened" or "nothing could be heard".
  useEffect(() => {
    qc.setQueryData<MonitorLiveState>(MONITOR_LIVE_KEY, (prev) => ({
      ...(prev ?? emptyLiveState()),
      stream: {
        state: stream.state,
        everReceived: stream.everReceived,
        staleForSecs: stream.staleForSecs,
        error: stream.error,
      },
    }))
  }, [qc, stream.state, stream.everReceived, stream.staleForSecs, stream.error])

  return null
}
