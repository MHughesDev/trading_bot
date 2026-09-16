// S-6.1 and S-6.22, enforced rather than reviewed.
//
// The recorded risk on this phase was that the desk would be built on the same
// deprecated aliases as the two surfaces it replaces, which would make the whole
// exercise a rename. A grep in review does not catch that, because the review
// happens once and the next file is written next week. This test runs on every
// commit.
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

/** The back-compat aliases `tailwind.theme.css` marks "do not introduce new usages". */
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
  '--color-warning',
]

describe('the desk is not built on the aliases it replaces', () => {
  const files = [
    ...walk(DESK_DIR, ['.tsx', '.ts']),
    resolve(SRC_DIR, 'pages/DeskPage.tsx'),
    resolve(SRC_DIR, 'styles/desk.css'),
    resolve(SRC_DIR, 'api/desk.ts'),
    resolve(SRC_DIR, 'hooks/useDeskStream.ts'),
    ...walk(resolve(SRC_DIR, 'lib/desk'), ['.ts']),
  ]

  it('finds desk source to check', () => {
    expect(files.length).toBeGreaterThan(10)
  })

  for (const alias of FORBIDDEN) {
    it(`no desk file uses ${alias}`, () => {
      const offenders = files.filter((f) => {
        // This test file names the aliases on purpose; exclude only itself.
        if (f === fileURLToPath(import.meta.url)) return false
        return stripComments(readFileSync(f, 'utf8')).includes(alias)
      })
      expect(offenders).toEqual([])
    })
  }
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
