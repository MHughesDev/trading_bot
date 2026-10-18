/**
 * RA-4.3: a skills-client path the server does not serve fails here, before it renders as an
 * empty panel. Every `skillsApi` call is made against a recording client and its URL is
 * matched against the `.route("...")` table under crates/api/src/routes (the same method as
 * the studio client's test; the Rust `the_desk_calls_routes_that_exist` scans these literals too).
 */
import { readdirSync, readFileSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { describe, expect, it, vi } from 'vitest'

const calls = vi.hoisted(() => [] as Array<{ method: string; url: string }>)
vi.mock('@/lib/api', () => {
  const rec = (method: string) => (url: string) => {
    calls.push({ method, url })
    return Promise.resolve({ data: {} })
  }
  return { api: { get: rec('GET'), post: rec('POST'), patch: rec('PATCH'), delete: rec('DELETE') } }
})

import { skillsApi } from '../skills'

const here = dirname(fileURLToPath(import.meta.url))
const routesDir = resolve(here, '../../../../crates/api/src/routes')

function servedPatterns(): string[] {
  const out: string[] = []
  const re = /\.route\(\s*"([^"]+)"/g
  for (const f of readdirSync(routesDir)) {
    if (!f.endsWith('.rs')) continue
    const src = readFileSync(join(routesDir, f), 'utf8')
    for (const m of src.matchAll(re)) out.push(m[1])
  }
  return out
}

/** The ids this test passes in; every other segment is a literal the route must spell exactly. */
const PARAMS = new Set(['s1', 'p1'])

function matches(pattern: string, url: string): boolean {
  const p = pattern.split('/')
  const u = url.split('/')
  if (p.length !== u.length) return false
  return p.every((seg, i) => (seg.startsWith('{') ? PARAMS.has(u[i]) : seg === u[i]))
}

describe('the skills client', () => {
  it('calls only paths the server routes', async () => {
    calls.length = 0
    await skillsApi.glossary()
    await skillsApi.search('exit')
    await skillsApi.item('s1')
    await skillsApi.proposals('submitted')
    await skillsApi.notices(3)
    await skillsApi.verify('p1')
    await skillsApi.admit('p1')
    await skillsApi.quarantine('s1', 'a reason')
    expect(calls).toHaveLength(8)

    const served = servedPatterns()
    expect(served.length).toBeGreaterThan(50)
    const unserved = calls.filter((c) => !served.some((p) => matches(p, c.url))).map((c) => `${c.method} ${c.url}`)
    expect(unserved).toEqual([])
  })

  it('has no promote, deprecate or curator method: those decisions are not this page', () => {
    for (const absent of ['promote', 'deprecate', 'curator', 'vote']) {
      expect(absent in skillsApi).toBe(false)
    }
  })
})
