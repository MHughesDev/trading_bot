/**
 * The two widths `styles/desk.css` changes the desk's shape at, and the one
 * question the components need to ask about them (AGENT-012 §1; S-6.22).
 *
 * The desk's responsive behaviour is entirely CSS, which is right — but two
 * pieces of state are decided in JavaScript and have to agree with it. `railOpen`
 * and `sideOpen` both defaulted to `true`, which is correct at desk width, where
 * the rail and the panel are grid columns beside the conversation. Below the
 * breakpoints below they are not columns: `desk.css` makes each a
 * `position: fixed` drawer over the centre. So on a phone the desk opened with
 * two drawers covering the conversation, the controls row and the composer, and
 * the operator's first act had to be closing both.
 *
 * The numbers live here rather than inline so that one file holds them and a test
 * can check them against the stylesheet
 * (`__tests__/a-drawer-that-cannot-close-is-not-a-375px-layout.test.ts`). A
 * breakpoint duplicated between a stylesheet and a `useState` is a pair that
 * drifts, and the symptom of the drift is a panel that covers the composer on
 * exactly the widths nobody reviews.
 */

/** Below this, `.desk-side` stops being a column and becomes a fixed drawer. */
export const PANEL_BECOMES_DRAWER_PX = 1200

/** Below this, `.desk-rail` stops being a column and becomes a fixed sheet. */
export const RAIL_BECOMES_SHEET_PX = 768

/**
 * Does this viewport match `(max-width: {px}px)`?
 *
 * `false` whenever the answer is not available — no `window`, no `matchMedia`
 * (the jsdom environment these tests run in has none), or a `matchMedia` that
 * throws. That default is deliberate: an unknown width must not collapse the
 * desk to its phone layout, because the phone layout hides two of three columns
 * and a wide screen rendered as a phone is a worse error than the reverse. A
 * test that wants the narrow branch defines `window.matchMedia` and says so.
 */
export function viewportIsAtMost(px: number): boolean {
  if (typeof window === 'undefined') return false
  const mm = window.matchMedia
  if (typeof mm !== 'function') return false
  try {
    return mm.call(window, `(max-width: ${px}px)`).matches === true
  } catch {
    return false
  }
}
