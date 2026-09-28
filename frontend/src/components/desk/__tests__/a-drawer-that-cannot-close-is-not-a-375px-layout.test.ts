/**
 * S-6.22's last open clause: **"at 375px the conversation, composer, controls row
 * and approval cards are fully usable."**
 *
 * It had no test anywhere in this suite. `grep -rn "375"` over `frontend/src`
 * returned nothing, and the nearest thing to a phone-width assertion was the
 * existence of a `@media (max-width: 768px)` block in `desk.css` — which is a
 * statement that somebody thought about narrow widths, not a statement that the
 * result works. The clause was recorded as "unverifiable" on 2026-09-23.
 *
 * It is verifiable, and it was **false**. Two defects, both of the shape this
 * phase keeps producing — a control that reaches nothing:
 *
 *  1. **`Hide panel` hid nothing.** `Conversation.tsx:509` renders
 *     `<aside className="desk-side" hidden={!sideOpen}>`, and the base rule
 *     `.desk-side { … display: flex; }` outranks the user-agent's
 *     `[hidden] { display: none }` — author origin beats UA origin whatever the
 *     specificity. `.desk-rail` has an explicit `[hidden]` rule for exactly this
 *     reason; `.desk-side` never got one. Under 1200px the panel is
 *     `position: fixed; right: 0; width: min(380px, 92vw)`, so at 375px it covers
 *     345 of the 375 available pixels — over the conversation, the controls row,
 *     the composer and any approval card in it — and the button that says it will
 *     close it is itself under the thing it cannot close.
 *
 *  2. **The rail toggle was positioned against the viewport.** `.desk-rail-toggle`
 *     is `position: absolute; left: var(--s-2); top: var(--s-2)` under 768px, and
 *     nothing in its ancestor chain established a containing block: `.desk-main`,
 *     `.desk`, the `<main id="main">` in `AppLayout.tsx` and `.app` in `app.css`
 *     are all unpositioned. So it resolved against the initial containing block —
 *     8px from the top-left of the *viewport*, underneath `.nav`, which is
 *     `position: relative; z-index: 200` against the toggle's 32. On a phone the
 *     rail opens by default and the only control that closes it was drawn behind
 *     the top bar.
 *
 * Together: at 375px the desk opened with two fixed drawers over the composer and
 * no working way to dismiss either. "Fully usable" was not close.
 *
 * **Why this is a stylesheet test and not a render test.** `vitest.config.ts`
 * says it in a comment: Tailwind is deliberately not wired into the jsdom
 * environment, and jsdom does no layout and evaluates no media query. A
 * `render()` at `window.innerWidth = 375` would have passed *before* both fixes,
 * because the desk's responsive behaviour is entirely CSS and none of it is
 * reachable from the DOM these tests see. So this file resolves the cascade
 * itself — parse `desk.css`, keep the blocks that apply at a 375px viewport,
 * match them against a model of the desk's real element tree with nwsapi's own
 * `Element.matches`, and sort by specificity and source order. The model is
 * pinned to the components it models by the last `describe` in this file, so it
 * cannot quietly drift into describing a tree that no longer exists.
 */
import { readFileSync, readdirSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { describe, expect, it } from 'vitest'

import {
  PANEL_BECOMES_DRAWER_PX,
  RAIL_BECOMES_SHEET_PX,
  viewportIsAtMost,
} from '@/lib/desk/viewport'

const here = dirname(fileURLToPath(import.meta.url))
const SRC = resolve(here, '../../..')
const STYLES = join(SRC, 'styles')
const DESK_CSS = join(STYLES, 'desk.css')

/** The clause's width. A phone, and the narrowest the exit criteria name. */
const PHONE = 375
/** A width where none of the desk's breakpoints have fired, for the both-directions half. */
const DESKTOP = 1400

type Rule = { selectors: string[]; decls: Map<string, string>; order: number; media: string }

function stripComments(css: string): string {
  return css.replace(/\/\*[\s\S]*?\*\//g, '')
}

/**
 * Evaluate a `@media` prelude at a given viewport width.
 *
 * An unrecognised feature throws rather than being treated as inapplicable. A
 * media block this function silently drops is a block whose rules vanish from
 * every assertion below, which is how a guard comes to assert less than it
 * claims — the failure S-6.22's own alias guard already had once.
 */
function mediaApplies(prelude: string, width: number): boolean {
  const q = prelude.trim().toLowerCase().replace(/^@media\s*/, '')
  for (const clause of q.split(/\s+and\s+/)) {
    const t = clause.trim()
    if (t === '' || t === 'screen' || t === 'all') continue
    // The dossier's print sheet. Not a screen at any width.
    if (t === 'print') return false
    const m = /^\((max|min)-width:\s*(\d+)px\)$/.exec(t)
    if (!m) throw new Error(`unhandled media condition in desk.css: ${t}`)
    const n = Number(m[2])
    if (m[1] === 'max' ? width > n : width < n) return false
  }
  return true
}

function parseApplicable(css: string, width: number): Rule[] {
  const out: Rule[] = []
  let order = 0

  function walk(text: string, applies: boolean, media: string): void {
    let i = 0
    while (i < text.length) {
      const brace = text.indexOf('{', i)
      if (brace === -1) break
      const prelude = text.slice(i, brace).trim()
      let depth = 1
      let j = brace + 1
      while (j < text.length && depth > 0) {
        if (text[j] === '{') depth++
        else if (text[j] === '}') depth--
        j++
      }
      const body = text.slice(brace + 1, j - 1)
      if (prelude.startsWith('@')) {
        if (!/^@media\b/i.test(prelude)) {
          throw new Error(`unhandled at-rule in desk.css: ${prelude.slice(0, 48)}`)
        }
        walk(body, applies && mediaApplies(prelude, width), prelude)
      } else if (applies) {
        const decls = new Map<string, string>()
        for (const d of body.split(';')) {
          const k = d.indexOf(':')
          if (k === -1) continue
          decls.set(d.slice(0, k).trim(), d.slice(k + 1).trim())
        }
        out.push({ selectors: prelude.split(',').map((s) => s.trim()), decls, order: order++, media })
      }
      i = j
    }
  }

  walk(stripComments(css), true, '')
  return out
}

/** (ids, classes + attributes + pseudo-classes, type selectors). */
function specificity(sel: string): [number, number, number] {
  const s = sel.replace(/\s*[>+~]\s*/g, ' ')
  const ids = (s.match(/#[A-Za-z_-][\w-]*/g) ?? []).length
  const attrs = (s.match(/\[[^\]]*\]/g) ?? []).length
  const classes = (s.match(/\.[A-Za-z_-][\w-]*/g) ?? []).length
  const pseudoClasses = (s.match(/(?<!:):[a-z-]+(?:\([^)]*\))?/g) ?? []).length
  const bare = s
    .replace(/\[[^\]]*\]/g, ' ')
    .replace(/::?[a-z-]+(?:\([^)]*\))?/g, ' ')
    .replace(/[.#][A-Za-z_-][\w-]*/g, ' ')
  const elements = (bare.match(/[A-Za-z][\w-]*/g) ?? []).length
  return [ids, classes + attrs + pseudoClasses, elements]
}

function beats(a: { spec: [number, number, number]; order: number }, b: typeof a): boolean {
  for (let k = 0; k < 3; k++) {
    if (a.spec[k] !== b.spec[k]) return a.spec[k] > b.spec[k]
  }
  return a.order >= b.order
}

/** The declaration that wins for `prop` on `el`, or `null` if nothing declares it. */
function winner(el: Element, rules: Rule[], prop: string): { value: string; selector: string } | null {
  let best: { value: string; selector: string; spec: [number, number, number]; order: number } | null = null
  for (const r of rules) {
    const raw = r.decls.get(prop)
    if (raw === undefined) continue
    for (const sel of r.selectors) {
      if (!el.matches(sel)) continue
      const cand = { value: raw, selector: sel, spec: specificity(sel), order: r.order }
      if (best === null || beats(cand, best)) best = cand
    }
  }
  return best === null ? null : { value: best.value, selector: best.selector }
}

/**
 * A model of the desk's element tree.
 *
 * Read off `pages/DeskPage.tsx` (the shell and the rail toggle) and
 * `components/desk/Conversation.tsx` (the header, the body, the centre column,
 * the dock and the right panel). The last `describe` in this file asserts the
 * nesting order this model assumes still holds in both of those files.
 */
function model(opts: { railHidden: boolean; sideHidden: boolean }) {
  const doc = document.implementation.createHTMLDocument('desk')
  const html = `
    <div class="desk">
      <nav class="desk-rail"${opts.railHidden ? ' hidden' : ''}>
        <div class="desk-rail-hd"></div>
        <ul><li><button></button></li></ul>
      </nav>
      <div class="desk-main">
        <button class="desk-rail-toggle"></button>
        <div class="desk-hd"><span class="title"></span></div>
        <div class="desk-body">
          <div class="desk-center">
            <div class="desk-scroll">
              <div class="desk-step">
                <div class="desk-card">
                  <div class="desk-card-hd"></div>
                  <div class="desk-actions"><input class="desk-approve-note"></div>
                </div>
              </div>
            </div>
            <div class="desk-dock">
              <div class="desk-controls" role="toolbar"></div>
              <div class="desk-composer"><div class="row"></div></div>
            </div>
          </div>
          <aside class="desk-side"${opts.sideHidden ? ' hidden' : ''}>
            <div class="desk-side-hd"></div>
            <div class="desk-side-bd"></div>
          </aside>
        </div>
      </div>
    </div>`
  doc.body.innerHTML = html
  const q = (sel: string): Element => {
    const el = doc.querySelector(sel)
    if (el === null) throw new Error(`the model has no ${sel}`)
    return el
  }
  return { doc, q }
}

const phone = parseApplicable(readFileSync(DESK_CSS, 'utf8'), PHONE)
const desktop = parseApplicable(readFileSync(DESK_CSS, 'utf8'), DESKTOP)

describe('the drawers that cover the composer at 375px can be closed', () => {
  it('hides the right panel when the panel is hidden', () => {
    // `Hide panel` sets the `hidden` attribute and nothing else. If the author
    // stylesheet does not answer it, the UA rule loses on origin and the button
    // is decoration — over the composer, at this width.
    const { q } = model({ railHidden: false, sideHidden: true })
    const d = winner(q('.desk-side'), phone, 'display')
    expect(d, 'no rule in desk.css gives a hidden .desk-side a display').not.toBeNull()
    expect(d?.value).toBe('none')
  })

  it('still draws the right panel when it is open, so the fix is not a blanket hide', () => {
    const { q } = model({ railHidden: false, sideHidden: false })
    expect(winner(q('.desk-side'), phone, 'display')?.value).toBe('flex')
  })

  it('hides the rail when the rail is hidden', () => {
    const { q } = model({ railHidden: true, sideHidden: false })
    expect(winner(q('.desk-rail'), phone, 'display')?.value).toBe('none')
  })

  it('still draws the rail when it is open', () => {
    const { q } = model({ railHidden: false, sideHidden: false })
    const d = winner(q('.desk-rail'), phone, 'display')
    // Nothing declares one: a `nav` is `display: block` from the UA sheet, which
    // is what the rail wants. The assertion is that no author rule has quietly
    // turned it off.
    expect(d?.value ?? 'block').not.toBe('none')
  })
})

describe('the rail toggle is reachable at 375px', () => {
  it('is drawn at 375px and not at 1400px', () => {
    const { q } = model({ railHidden: true, sideHidden: true })
    expect(winner(q('.desk-rail-toggle'), phone, 'display')?.value).toBe('inline-flex')
    expect(winner(q('.desk-rail-toggle'), desktop, 'display')?.value).toBe('none')
  })

  it('resolves against a box inside the desk rather than against the viewport', () => {
    const { q } = model({ railHidden: true, sideHidden: true })
    const toggle = q('.desk-rail-toggle')
    expect(winner(toggle, phone, 'position')?.value).toBe('absolute')

    const positioned: string[] = []
    for (let el = toggle.parentElement; el !== null; el = el.parentElement) {
      const p = winner(el, phone, 'position')?.value
      if (p !== undefined && p !== 'static') positioned.push(`${el.className} → ${p}`)
    }
    // The model's root is `.desk`, which is where the chain has to be closed:
    // nothing above it is positioned either (asserted below), so a toggle with no
    // positioned ancestor inside the desk lands on the viewport, under `.nav`.
    expect(positioned, 'nothing between .desk-rail-toggle and .desk establishes a containing block').not.toEqual([])
  })

  it('sits above the two drawers it exists to dismiss', () => {
    const { q } = model({ railHidden: false, sideHidden: false })
    const z = (sel: string) => Number(winner(q(sel), phone, 'z-index')?.value ?? '0')
    expect(z('.desk-rail-toggle')).toBeGreaterThan(z('.desk-rail'))
    expect(z('.desk-rail-toggle')).toBeGreaterThan(z('.desk-side'))
  })

  it('has nothing positioned above the desk to fall back on', () => {
    // `AppLayout.tsx` renders `.app > main#main > <Outlet/>`, and `main` carries
    // only `flex-1 overflow-hidden` plus an inline `minHeight/display/
    // flexDirection`. Neither it nor `.app` is positioned, so `desk.css` owns
    // this. If that ever changes, the assertion above is being satisfied by
    // accident and should be re-read rather than trusted.
    const app = stripComments(readFileSync(join(STYLES, 'app.css'), 'utf8'))
    const rule = /(^|\})\s*\.app\s*\{([^}]*)\}/m.exec(app)
    expect(rule, 'app.css no longer declares .app').not.toBeNull()
    expect(rule?.[2]).not.toMatch(/position\s*:/)
    const layout = readFileSync(join(SRC, 'components/layout/AppLayout.tsx'), 'utf8')
    expect(layout).toMatch(/<main\b[\s\S]{0,400}?id="main"/)
    expect(layout).not.toMatch(/position:\s*'(relative|absolute|fixed|sticky)'/)
  })
})

describe('nothing the operator has to touch is wider than the phone', () => {
  // A fixed track or minimum wider than 375px is a horizontal scrollbar across
  // the whole desk, and the four things this clause names are the four that
  // would be pushed off it.
  const CHAIN = [
    '.desk',
    '.desk-main',
    '.desk-hd',
    '.desk-body',
    '.desk-center',
    '.desk-scroll',
    '.desk-dock',
    '.desk-controls',
    '.desk-composer',
    '.desk-side',
    '.desk-rail',
    '.desk-card',
    '.desk-actions',
    '.desk-approve-note',
  ]

  function pxOf(value: string): number[] {
    // `min(380px, 92vw)` is not a fixed width — the viewport clamps it. Only a
    // length that no viewport can shrink counts here.
    if (/\b(min|clamp)\s*\(/.test(value)) return []
    return [
      ...[...value.matchAll(/(\d+(?:\.\d+)?)px/g)].map((m) => Number(m[1])),
      ...[...value.matchAll(/(\d+(?:\.\d+)?)rem/g)].map((m) => Number(m[1]) * 16),
    ]
  }

  for (const sel of CHAIN) {
    it(`${sel} declares no fixed width over ${PHONE}px at ${PHONE}px`, () => {
      const { q } = model({ railHidden: false, sideHidden: false })
      const el = q(sel)
      for (const prop of ['width', 'min-width', 'grid-template-columns', 'flex-basis']) {
        const w = winner(el, phone, prop)
        if (w === null) continue
        const over = pxOf(w.value).filter((n) => n > PHONE)
        expect(over, `${sel} { ${prop}: ${w.value} } (from \`${w.selector}\`)`).toEqual([])
      }
    })
  }

  it('gives the desk a single column at 375px, so the rail does not take the conversation with it', () => {
    const { q } = model({ railHidden: false, sideHidden: false })
    const tracks = winner(q('.desk'), phone, 'grid-template-columns')
    expect(tracks?.value).toBe('minmax(0, 1fr)')
  })

  it('wraps the controls row and the composer rows rather than overflowing them', () => {
    // Nine controls and a model picker do not fit on 375px in one line. `wrap` is
    // the difference between a second row and a button nobody can reach.
    const { q } = model({ railHidden: false, sideHidden: false })
    expect(winner(q('.desk-controls'), phone, 'flex-wrap')?.value).toBe('wrap')
    expect(winner(q('.desk-composer .row'), phone, 'flex-wrap')?.value).toBe('wrap')
    expect(winner(q('.desk-actions'), phone, 'flex-wrap')?.value).toBe('wrap')
    expect(winner(q('.desk-card-hd'), phone, 'flex-wrap')?.value).toBe('wrap')
    expect(winner(q('.desk-hd'), phone, 'flex-wrap')?.value).toBe('wrap')
  })

  it('keeps the dock on screen while the conversation scrolls under it', () => {
    const { q } = model({ railHidden: false, sideHidden: false })
    expect(winner(q('.desk-dock'), phone, 'position')?.value).toBe('sticky')
    expect(winner(q('.desk-dock'), phone, 'bottom')?.value).toBe('0')
    expect(winner(q('.desk-scroll'), phone, 'overflow')?.value).toBe('auto')
  })

  it('leaves room in the header for the toggle drawn over it', () => {
    const { q } = model({ railHidden: false, sideHidden: false })
    const pad = winner(q('.desk-hd'), phone, 'padding-left')
    expect(pad, 'the header reserves no room for the absolute toggle').not.toBeNull()
    expect(pxOf(pad?.value ?? '')[0] ?? 0).toBeGreaterThanOrEqual(32)
  })
})

describe('the desk does not open a drawer over the composer it covers', () => {
  // Closing them is necessary and was not sufficient. `railOpen` and `sideOpen`
  // both defaulted to `true`, which is right where they are grid columns and
  // wrong where `desk.css` has turned them into fixed drawers: at 375px the desk
  // opened with 280px of rail and 345px of panel over a 375px conversation, and
  // the operator's first two actions had to be dismissing them.
  //
  // Tested at the seam rather than through a render: no test in this suite mounts
  // `Conversation` — it wants a router, a query client, the event stream and six
  // API modules — and a render under jsdom could not observe the CSS that makes
  // the drawers drawers anyway. So the decision function is tested directly and
  // the two call sites are pinned to it.
  const original = Object.getOwnPropertyDescriptor(window, 'matchMedia')

  function withWidth(width: number | null, fn: () => void): void {
    if (width === null) {
      Object.defineProperty(window, 'matchMedia', { value: undefined, configurable: true, writable: true })
    } else {
      Object.defineProperty(window, 'matchMedia', {
        value: (q: string) => {
          const m = /\(max-width:\s*(\d+)px\)/.exec(q)
          if (m === null) throw new Error(`unexpected query: ${q}`)
          return { matches: width <= Number(m[1]), media: q }
        },
        configurable: true,
        writable: true,
      })
    }
    try {
      fn()
    } finally {
      if (original) Object.defineProperty(window, 'matchMedia', original)
      else Reflect.deleteProperty(window, 'matchMedia')
    }
  }

  it('reads a phone as narrower than both breakpoints', () => {
    withWidth(PHONE, () => {
      expect(viewportIsAtMost(PANEL_BECOMES_DRAWER_PX)).toBe(true)
      expect(viewportIsAtMost(RAIL_BECOMES_SHEET_PX)).toBe(true)
    })
  })

  it('reads a wide screen as narrower than neither', () => {
    withWidth(DESKTOP, () => {
      expect(viewportIsAtMost(PANEL_BECOMES_DRAWER_PX)).toBe(false)
      expect(viewportIsAtMost(RAIL_BECOMES_SHEET_PX)).toBe(false)
    })
  })

  it('answers false rather than guessing narrow when the width cannot be read', () => {
    // A wide screen collapsed to the phone layout hides two of three columns; the
    // reverse leaves a drawer to close. The unknown case takes the recoverable
    // error, and every existing test in this suite is in the unknown case.
    withWidth(null, () => {
      expect(viewportIsAtMost(PANEL_BECOMES_DRAWER_PX)).toBe(false)
    })
    Object.defineProperty(window, 'matchMedia', {
      value: () => {
        throw new Error('not implemented')
      },
      configurable: true,
      writable: true,
    })
    try {
      expect(viewportIsAtMost(RAIL_BECOMES_SHEET_PX)).toBe(false)
    } finally {
      if (original) Object.defineProperty(window, 'matchMedia', original)
      else Reflect.deleteProperty(window, 'matchMedia')
    }
  })

  it('agrees with the widths desk.css actually changes shape at', () => {
    // The breakpoints exist twice — in a stylesheet and in a module constant — so
    // this is the join. A pair that drifts shows up as a panel covering the
    // composer on exactly the widths nobody reviews.
    const css = stripComments(readFileSync(DESK_CSS, 'utf8'))
    const widths = [...css.matchAll(/@media\s*\(max-width:\s*(\d+)px\)/g)].map((m) => Number(m[1]))
    expect(widths).toContain(PANEL_BECOMES_DRAWER_PX)
    expect(widths).toContain(RAIL_BECOMES_SHEET_PX)
    expect([...new Set(widths)].sort((a, b) => a - b)).toEqual([RAIL_BECOMES_SHEET_PX, PANEL_BECOMES_DRAWER_PX])
  })

  it('has both call sites defaulting from the viewport and not from true', () => {
    const convo = readFileSync(join(SRC, 'components/desk/Conversation.tsx'), 'utf8')
    expect(convo).toMatch(
      /const \[sideOpen, setSideOpen\] = useState\(\(\) => !viewportIsAtMost\(PANEL_BECOMES_DRAWER_PX\)\)/,
    )
    const page = readFileSync(join(SRC, 'pages/DeskPage.tsx'), 'utf8')
    expect(page).toMatch(
      /const \[railOpen, setRailOpen\] = useState\(\(\) => !viewportIsAtMost\(RAIL_BECOMES_SHEET_PX\)\)/,
    )
  })
})

describe('the model is the tree the components actually render', () => {
  function orderedIn(file: string, needles: string[]): void {
    const text = readFileSync(join(SRC, file), 'utf8')
    let at = 0
    for (const n of needles) {
      const found = text.indexOf(n, at)
      expect(found, `${file} no longer contains ${n} after the previous marker`).toBeGreaterThan(-1)
      at = found + n.length
    }
  }

  it('keeps the rail toggle inside .desk-main, which is the box the padding reserves room in', () => {
    orderedIn('pages/DeskPage.tsx', [
      'className="desk"',
      'className="desk-main"',
      'className="desk-rail-toggle"',
    ])
  })

  it('keeps the dock and the right panel where the cascade above assumes they are', () => {
    orderedIn('components/desk/Conversation.tsx', [
      'className="desk-hd"',
      'className="desk-body"',
      'className="desk-center"',
      'className="desk-scroll"',
      'className="desk-dock"',
      'className="desk-side"',
    ])
  })

  it('still answers Hide panel with the hidden attribute and nothing else', () => {
    const text = readFileSync(join(SRC, 'components/desk/Conversation.tsx'), 'utf8')
    expect(text).toMatch(/className="desk-side"\s+hidden=\{!sideOpen\}/)
    expect(text).toMatch(/sideOpen \? 'Hide panel' : 'Show panel'/)
  })

  it('is reading the only stylesheet that styles the desk', () => {
    const others = readdirSync(STYLES)
      .filter((f) => f.endsWith('.css') && f !== 'desk.css')
      .filter((f) => /(^|[\s,>{])\.desk[\s.,:[{>]/.test(stripComments(readFileSync(join(STYLES, f), 'utf8'))))
    expect(others, 'a second stylesheet styles the desk and this cascade does not read it').toEqual([])
  })
})
