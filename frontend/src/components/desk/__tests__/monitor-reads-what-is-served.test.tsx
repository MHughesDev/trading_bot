/**
 * The monitor claimed "armed" when nothing answered, and read three field names
 * the route does not send (AGENT-012 §11; Set S task S-6.21).
 *
 * Two defects, and the first is this platform's non-negotiable pointed at the
 * control that stops money moving:
 *
 * 1. `const tripped = kill.data?.kill_switch_active === true` — so a kill-switch
 *    read that did not answer rendered a **green "armed"** badge. An unknown
 *    safety state shown as the safe one. The same line printed `scope global`
 *    from `kill.data?.scope ?? 'global'`, and `/api/trading/status` has never
 *    sent a `scope` at all, so the widest possible claim was a client-side
 *    default.
 *
 * 2. The strategies table read `name`, `status` and `breaching_metric`;
 *    `/api/monitor/strategies` sends `strategy_id`, `state` and `scale_pct`.
 *    Every live strategy rendered as two blanks and a dash — a monitor with a
 *    table of empty rows, which reads as a monitor with nothing to report. The
 *    empty state underneath said the phase was unbuilt, which stopped being true
 *    when COMP-007 landed.
 *
 * `scale_pct` carries the rule in its own right: 0% means the monitor shut the
 * strategy down and an absent scale means nobody said, and they were one `?? 0`
 * apart.
 */
import { describe, expect, it } from 'vitest'
import { killSwitchBadge, monitorRow } from '@/components/desk/Operations.helpers'


describe('the kill-switch badge does not call an unknown state safe', () => {
  it('says the state is unknown when nothing answered', () => {
    for (const nothing of [null, undefined]) {
      const b = killSwitchBadge(nothing)
      expect(b.word).toBe('state unknown')
      // Not the green one. The tone is half of what a person reads here.
      expect(b.tone).toBe('warn')
    }
  })

  it('still says armed when the endpoint answered that it is', () => {
    const b = killSwitchBadge({ kill_switch_active: false })
    expect(b.word).toBe('armed')
    expect(b.tone).toBe('pos')
  })

  it('says tripped when it is tripped', () => {
    const b = killSwitchBadge({ kill_switch_active: true })
    expect(b.word).toBe('tripped')
    expect(b.tone).toBe('neg')
  })
})

describe('a monitor row is read under the names the route serves', () => {
  it('reads strategy_id and state, which is what comes back', () => {
    const row = monitorRow({ strategy_id: 'st_7', state: 'scaled_down', scale_pct: 25 })
    expect(row.strategy_id).toBe('st_7')
    expect(row.state).toBe('scaled_down')
    expect(row.scale_pct).toBe(25)
  })

  it('does not read the old names, which never arrive', () => {
    // The row the table used to render: every field under a name the route does
    // not send, and no error anywhere.
    const row = monitorRow({ name: 'momentum', status: 'live', breaching_metric: 'dd' })
    expect(row.state).toBeNull()
    expect(row.scale_pct).toBeNull()
    expect(row.strategy_id).toBe('unnamed')
  })

  it('keeps a scale of zero apart from a scale nobody recorded', () => {
    // 0% is the monitor having shut the strategy down. Absent is not that.
    expect(monitorRow({ strategy_id: 'st_1', scale_pct: 0 }).scale_pct).toBe(0)
    expect(monitorRow({ strategy_id: 'st_1' }).scale_pct).toBeNull()
    expect(monitorRow({ strategy_id: 'st_1', scale_pct: null }).scale_pct).toBeNull()
  })

  it('refuses a non-finite scale rather than printing NaN%', () => {
    expect(monitorRow({ strategy_id: 'st_1', scale_pct: Number.NaN }).scale_pct).toBeNull()
  })
})
