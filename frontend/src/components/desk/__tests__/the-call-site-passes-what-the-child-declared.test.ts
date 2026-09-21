/**
 * The defect this file guards is not in any component. It is in the seam.
 *
 * Six controls on the Research Desk were declared in a child's props, wired
 * inside that child, and passed by nobody at the one place the child is used.
 * Every component test passed. The child was correct in isolation and the
 * screen did nothing, which is why the family survived so long: a test that
 * renders the child and hands it a spy proves the child, and the bug was never
 * in the child.
 *
 * So these are assertions about the **call site**, written the way this
 * repository already writes call-site invariants — by reading the source, the
 * same technique as `no-deprecated-aliases.test.ts` — plus one runtime check on
 * the API object. A render test of `Conversation` would be better; it needs a
 * live query client, an SSE stub and eight mocked endpoints, and would be
 * asserting on the mocks rather than on the wiring.
 *
 * WHICH OF THESE FAILED BEFORE. Six of the eight, checked by reverting the
 * sources and running this file rather than assumed. The two that held are the
 * ones asserting what was deliberately *kept* — `cancel`, `steer`, and the two
 * unbuilt reads — and they are here so the next deletion stops at the right
 * place.
 */
import { readFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { describe, expect, it } from 'vitest'

import { deskApi } from '@/api/desk'

const here = dirname(fileURLToPath(import.meta.url))
const conversation = readFileSync(resolve(here, '../Conversation.tsx'), 'utf8')

describe('the conversation passes the handlers the composer declared', () => {
  it('gives Write strategy somewhere to go', () => {
    // `onWriteStrategy` was declared on `ComposerProps`, the button called it,
    // and this — its only caller — passed nothing. The composer now draws the
    // button only when a handler arrives, so this assertion is what keeps the
    // button on the screen at all.
    expect(conversation).toContain('onWriteStrategy={')
    // `/desk/strategy`, which `DeskPage` mounts as `StrategyEditor` — the SLv2
    // editor §3.3 names. Not `/strategy`, which is a different, older page.
    expect(conversation).toContain("onOpenPath('/desk/strategy')")
  })
})

describe('the reason typed on an approval survives the call site', () => {
  it('forwards the note rather than dropping the second argument', () => {
    // `Cards.tsx` sends `actions.onApprove?.(id, note.trim() || undefined)`.
    // The call site read `onApprove: (id) => void deskApi.answerApproval(id,
    // 'approve')` — one parameter, so the note was discarded between the field
    // and the wire. `spend_holdout_look` refuses without twenty characters of
    // it, into an append-only log, so this was not a lost nicety: the person
    // typed a justification and the platform was told none was given.
    expect(conversation).toMatch(
      /onApprove:\s*\(id,\s*note\)\s*=>\s*void deskApi\.answerApproval\(id,\s*'approve',\s*note\)/,
    )
  })
})

describe('the memory tab is not handed a literal null', () => {
  it('no longer passes items at all, because there are none to pass', () => {
    expect(conversation).not.toContain('items={null}')
    expect(conversation).toContain('<MemoryTab />')
  })
})

describe('the desk keeps no method for a route that is not mounted', () => {
  // The `prioritisedBy` lesson, applied: a dead name is a name the next caller
  // reaches for, and the fix that holds is deleting it and asserting it is
  // gone. `deskApi.pause`, `.resume` and `.escalate` posted to three routes
  // that answer 404 — through `optional()`, which turns a 404 into `null`, so
  // no call ever appeared to fail.
  for (const name of ['pause', 'resume', 'escalate']) {
    it(`has no ${name}, because nothing answers a ${name}`, () => {
      expect(Object.keys(deskApi)).not.toContain(name)
    })
  }

  it('keeps cancel and steer, which are mounted', () => {
    // 401 to an unauthenticated POST, where the three above answer 404.
    expect(Object.keys(deskApi)).toContain('cancel')
    expect(Object.keys(deskApi)).toContain('steer')
  })

  it('keeps the unbuilt *reads*, which have an honest absence to render', () => {
    // The asymmetry is deliberate and worth stating. `state()` and
    // `trajectory()` also call routes that are not mounted, and they stay: a
    // read that returns nothing has a true sentence the panel can print, and
    // both panels print one. A control has no such sentence — a click either
    // reaches the platform or it does not happen.
    expect(Object.keys(deskApi)).toContain('state')
    expect(Object.keys(deskApi)).toContain('trajectory')
  })
})
