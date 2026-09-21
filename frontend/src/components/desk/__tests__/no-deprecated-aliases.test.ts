// S-6.1 and S-6.22, enforced rather than reviewed.
//
// The recorded risk on this phase was that the desk would be built on the same
// deprecated aliases as the two surfaces it replaces, which would make the whole
// exercise a rename. A grep in review does not catch that, because the review
// happens once and the next file is written next week. This test runs on every
// commit.
//
// SCOPE. S-6.22's acceptance criterion is a claim about the whole frontend:
// "a grep for bg-surface-2, text-text-muted, text-text-dim, bg-pnl-down,
// text-warning returns nothing outside compat.css". Until 2026-09-21 this file
// walked only `components/desk/` plus five named files while asserting that
// claim, and the criterion was marked met while 27 files outside the desk still
// used those aliases. A guard narrower than its claim is worse than no guard:
// it converts an open question into a settled one. It now walks all of
// `src/`, and the only files allowed to name an alias are the ones listed in
// ALLOWED below, each with a reason.
//
// It also enforces the other thing a review would miss: the desk must show the
// user **two** options — Chat and Research — with a Continuous toggle, and no
// third word anywhere in the surface (ADR-0037 §3, AGENT-012 §19.10).
import { readFileSync, readdirSync, statSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { describe, expect, it } from 'vitest'

const here = dirname(fileURLToPath(import.meta.url))
const DESK_DIR = resolve(here, '..')
const SRC_DIR = resolve(here, '../../..')

function walk(dir: string, exts: string[]): string[] {
  const out: string[] = []
  for (const name of readdirSync(dir)) {
    const p = join(dir, name)
    if (statSync(p).isDirectory()) out.push(...walk(p, exts))
    else if (exts.some((e) => name.endsWith(e))) out.push(p)
  }
  return out
}

/**
 * Strip comments before scanning.
 *
 * A ban you may not name in its own documentation is a ban nobody can explain.
 * `styles/desk.css` opens by saying which aliases must not appear in it, and a
 * scan over raw text reads that sentence as a violation — so the file that
 * documents the rule was the file breaking it.
 *
 * The Rust side of this repository already solved the same problem the same way
 * (`crates/invariants/tests/static_invariants.rs::strip_comments`): these scans
 * forbid *usages*, and explaining a ban is not a usage. Line comments are only
 * stripped when the `//` is not part of a `://`, so a URL survives.
 */
function stripComments(text: string): string {
  return text.replace(/\/\*[\s\S]*?\*\//g, '').replace(/(^|[^:])\/\/.*$/gm, '$1')
}

/**
 * The back-compat aliases `tailwind.theme.css` marks "do not introduce new usages".
 *
 * The five the acceptance criterion names, plus `bg-pnl-up` and the custom
 * property each one is declared as — because a screen can reach the same
 * deprecated colour either way, and a list that bans only the utility class
 * leaves the other door open. Wider than the criterion is allowed; narrower is
 * the thing this file exists to stop.
 *
 * What each alias was migrated to, and why that pair and no other: the modern
 * class resolves to the *same* `var(…)` as the alias, so the migration is a
 * rename and not a restyle. Read off `styles/tailwind.theme.css`.
 *
 *   bg-surface-2    → bg-sunken          (--color-surface-2  = var(--bg-sunken))
 *   text-text-muted → text-fg-secondary  (--color-text-muted = var(--fg-secondary))
 *   text-text-dim   → text-fg-tertiary   (--color-text-dim   = var(--fg-tertiary))
 *   bg-pnl-down     → bg-neg             (--color-pnl-down   = var(--fg-neg))
 *   bg-pnl-up       → bg-pos             (--color-pnl-up     = var(--fg-pos))
 *   text-warning    → text-warn          (--color-warning    = var(--fg-warn))
 */
const FORBIDDEN = [
  'bg-surface-2',
  'text-text-muted',
  'text-text-dim',
  'bg-pnl-down',
  'bg-pnl-up',
  'text-warning',
  '--color-surface-2',
  '--color-text-muted',
  '--color-text-dim',
  '--color-pnl-down',
  '--color-pnl-up',
  '--color-warning',
]

/**
 * Files permitted to name a deprecated alias, and why.
 *
 * Nothing goes in here to make a red test green. Each entry is a file that
 * *must* say the word to do its job.
 */
const ALLOWED: Record<string, string> = {
  // This file names them to ban them. Explaining a ban is not a usage.
  //
  // It is the only entry, and that is the point. The acceptance criterion says
  // the grep must return nothing "outside `compat.css`" — but these aliases were
  // never in `compat.css`; they were declared in `styles/tailwind.theme.css`,
  // and on 2026-09-21 the six this guard covers were deleted from there too,
  // their last consumer having been migrated. So there is now no file that has
  // to say the word except the one forbidding it.
  'components/desk/__tests__/no-deprecated-aliases.test.ts': 'states the ban',
}

function rel(f: string): string {
  return f.slice(SRC_DIR.length + 1).replace(/\\/g, '/')
}

describe('no source file outside the alias declaration uses a deprecated alias', () => {
  const files = walk(SRC_DIR, ['.tsx', '.ts', '.css'])

  it('finds the whole frontend source tree to check, not just the desk', () => {
    // The scope this test claims. `components/desk/` is 63 files and `src/` is
    // 216, so a regression that narrowed the walk back to the desk — which is
    // exactly how this guard came to be weaker than its own claim — fails here
    // before it can silently pass every alias case below.
    expect(files.length).toBeGreaterThan(150)
    expect(files.some((f) => rel(f).startsWith('pages/'))).toBe(true)
    expect(files.some((f) => rel(f).startsWith('components/mlops/'))).toBe(true)
    expect(files.some((f) => rel(f).startsWith('components/workbench/'))).toBe(true)
    expect(files.some((f) => rel(f).startsWith('components/automations/'))).toBe(true)
  })

  for (const alias of FORBIDDEN) {
    it(`no file under src/ uses ${alias}`, () => {
      const offenders = files
        .filter((f) => !(rel(f) in ALLOWED))
        .filter((f) => stripComments(readFileSync(f, 'utf8')).includes(alias))
        .map(rel)
      expect(offenders).toEqual([])
    })
  }

  /**
   * The colour names behind the aliases, banned in **any** utility position.
   *
   * The acceptance criterion greps for five class spellings. That is narrower
   * than the thing it is trying to assert, in the same way the old walk was:
   * Tailwind builds a utility for every prefix from one `--color-*`, so
   * `--color-pnl-down` was still reachable as `text-pnl-down`, `border-pnl-down`
   * and `border-pnl-down/20` after every `bg-pnl-down` was gone. On 2026-09-21
   * the five-name grep returned clean while 36 such usages remained across 16
   * files — the criterion could have been signed off with the aliases still in
   * service and `tailwind.theme.css` still unable to drop them.
   *
   * So the ban is on the colour name, not on six spellings of it.
   */
  const RETIRED_COLORS = ['surface-2', 'text-muted', 'text-dim', 'pnl-down', 'pnl-up', 'warning']
  const PREFIXES =
    'bg|text|border|ring|fill|stroke|from|via|to|divide|outline|decoration|accent|caret|shadow|placeholder'

  for (const color of RETIRED_COLORS) {
    it(`no file under src/ builds any utility on ${color}`, () => {
      const re = new RegExp(`(?:^|[\\s"'\`:])(?:${PREFIXES})-${color}(?![a-z0-9-])`, 'm')
      const offenders = files
        .filter((f) => !(rel(f) in ALLOWED))
        .filter((f) => re.test(stripComments(readFileSync(f, 'utf8'))))
        .map(rel)
      expect(offenders).toEqual([])
    })
  }

  it('is not banning a colour the theme still declares', () => {
    // A ban on a name the theme still maps would be a ban on working code. Each
    // retired name must be gone from `@theme` too, or this guard is asserting
    // something the stylesheet contradicts.
    const theme = readFileSync(resolve(SRC_DIR, 'styles/tailwind.theme.css'), 'utf8')
    const declarations = stripComments(theme)
    for (const color of RETIRED_COLORS) {
      expect(declarations, color).not.toMatch(new RegExp(`--color-${color}\\s*:`))
    }
  })
})

describe('two options and a toggle, not three modes', () => {
  const files = [...walk(DESK_DIR, ['.tsx', '.ts']), resolve(SRC_DIR, 'pages/DeskPage.tsx')].filter(
    (f) => f !== fileURLToPath(import.meta.url),
  )

  it('no desk source contains a user-visible third mode word', () => {
    // Case-insensitive and whole-word: the word must not appear at all, in a
    // label, a comment or a route. A word that survives in a comment is a word
    // that gets copied into a label.
    const offenders = files.filter((f) => /\bprograms?\b/i.test(readFileSync(f, 'utf8')))
    expect(offenders).toEqual([])
  })

  it('there is no /desk/programs route', () => {
    const routes = readFileSync(resolve(SRC_DIR, 'pages/DeskPage.tsx'), 'utf8')
    expect(routes).not.toMatch(/desk\/program/i)
  })
})

describe('the surfaces the desk replaces are gone', () => {
  it('nothing imports @/components/agent/* or @/components/workspace/*', () => {
    const all = walk(SRC_DIR, ['.ts', '.tsx'])
    const offenders = all.filter((f) => {
      const t = readFileSync(f, 'utf8')
      return /from '@\/components\/(agent|workspace)\//.test(t)
    })
    expect(offenders).toEqual([])
  })
})
