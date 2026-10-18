/**
 * S-6.24 — "each line opens the thing it names".
 *
 * `desk_digest.rs` links a concluded run to `/desk/runs/{id}` and a defaulted
 * approval to `/desk/approvals/{id}`. Neither was a desk route, so both fell to
 * `*` and redirected to desk home: the line opened the page the reader was
 * already on. This reads every `/desk/…` href the route emits and asserts a
 * `<Route path>` in `DeskPage.tsx` matches it.
 */
import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { describe, expect, it } from 'vitest'

const root = join(__dirname, '..', '..', '..', '..', '..')
const digest = readFileSync(join(root, 'crates', 'api', 'src', 'routes', 'desk_digest.rs'), 'utf8')
const page = readFileSync(join(root, 'frontend', 'src', 'pages', 'DeskPage.tsx'), 'utf8')

/** `/desk/runs/{id}` → `runs/:x`; the page's routes are relative to `/desk`. */
function emitted(): string[] {
  const out = new Set<string>()
  for (const m of digest.matchAll(/"(\/desk\/[^"]*)"/g)) out.add(m[1])
  return [...out]
}

function routes(): RegExp[] {
  return [...page.matchAll(/<Route\s+path="([^"]+)"/g)]
    .map((m) => m[1])
    .filter((p) => p !== '*')
    .map((p) => new RegExp(`^/desk/${p.replace(/:[A-Za-z]+/g, '[^/]+')}$`))
}

describe('every link the digest emits is a desk route', () => {
  it('finds the hrefs, so an empty scan cannot pass', () => {
    expect(emitted().length).toBeGreaterThanOrEqual(4)
  })
  it('matches each one to a route that is not the catch-all', () => {
    const rs = routes()
    for (const href of emitted()) {
      const concrete = href.replace(/\{[a-z_]+\}/g, 'x1')
      expect(rs.some((r) => r.test(concrete)), `${href} reaches no desk route`).toBe(true)
    }
  })
})
