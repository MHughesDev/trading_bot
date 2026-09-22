/**
 * A client method is read by the next author as evidence that a route exists
 * (Set S phase 6; the rule `deskApi` states where it deleted `pause`, `resume`
 * and `escalate`).
 *
 * This repository's dominant defect shape is complete, tested code with no
 * caller. Across an HTTP boundary it has a second, nastier form: a caller with
 * no callee, which no compiler can see and which renders as an empty panel —
 * indistinguishable from a panel with nothing in it.
 *
 * `researchApi.report(reportId)` was both at once. It called
 * `GET /api/reports/{id}`, which this platform does not mount — an
 * unauthenticated request answers **404** on :7080 where every mounted route
 * answers 401 — it returned `unknown`, so no call site could have disagreed
 * with whatever it produced, and nothing in `frontend/src` called it.
 *
 * It was found by requesting every `/api/…` literal in `frontend/src/api/*.ts`
 * against the running platform, which is the only method that has ever produced
 * a complete list here: three previous audits named three paths by reading, and
 * a sweep found twelve. This was the fourth path in that sweep that no audit
 * had named, and the only one with no caller at all.
 *
 * So this file holds two claims that point in opposite directions and are both
 * needed: the dead method stays gone, and the three live-but-unmounted reads
 * stay present, because those have panels that render an honest absence for
 * them and deleting a *read* with an honest absence would lose the explanation.
 * The distinction `deskApi` draws is the one applied here: a **control** has no
 * honest absence, and neither does a method nobody calls.
 */
import { describe, expect, it } from 'vitest'

import { researchApi } from '../research'
import { deskApi } from '../desk'
import { conversationsApi } from '../agent'

describe('a method nobody calls, in front of a route nothing serves, is deleted', () => {
  it('no longer offers `report` for the next caller to find', () => {
    expect('report' in researchApi).toBe(false)
  })

  it('keeps the reads whose panels render an honest absence for them', () => {
    // `/api/agent/runs/{id}/state`, `…/trajectory` and `…/lifecycle` answer 404
    // on this platform too. They stay, because the State tab, the replay
    // scrubber and the stage strip each say which absence it is — and a panel
    // that names a missing route is more useful than no panel.
    expect(typeof deskApi.state).toBe('function')
    expect(typeof deskApi.trajectory).toBe('function')
    expect(typeof deskApi.lifecycle).toBe('function')
  })

  it('offers no second client for a route that already has one', () => {
    // `deskApi.conversations` called `/api/agent/conversations` — a mounted
    // route, so it was not the shape above — and had no caller anywhere, while
    // its doc comment claimed it was "the conversation list the desk rail
    // renders". The rail calls `conversationsApi.list()`. A duplicate is worse
    // than an orphan here because it is typed weaker:
    // `Array<Record<string, unknown>>` against `Conversation[]`, so the next
    // author who reached for the one in `deskApi` would have given up every
    // field check on the rail without noticing.
    expect('conversations' in deskApi).toBe(false)
    expect(typeof conversationsApi.list).toBe('function')
  })

  it('still has no pause, resume or escalate control', () => {
    // The controls deleted before this one, for the same reason. A click either
    // reaches the platform or it does not happen; there is no honest absence
    // for a control.
    for (const gone of ['pause', 'resume', 'escalate']) {
      expect(gone in deskApi).toBe(false)
    }
  })
})
