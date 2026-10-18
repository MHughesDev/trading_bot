/**
 * N-5.1: a client path the server does not serve fails here, before it renders
 * as an empty panel (the Rust `every_client_path_is_served` does the same for
 * the MCP client). Every `studioApi` call is made against a recording client and
 * its URL is matched against the `.route("...")` table under crates/api/src/routes.
 *
 * The Studio's new routes live in `models_studio.rs`; until that file and its
 * registration in `routes/mod.rs` land this test names exactly which are missing.
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

import { studioApi } from '../modelsStudio'

const here = dirname(fileURLToPath(import.meta.url))
const routesDir = resolve(here, '../../../../crates/api/src/routes')

/** `(method?, pattern)` for every `.route("pattern", get(..).post(..))` in the routes directory. */
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

/** The ids this test passes in; every other URL segment is a literal the route must spell exactly. */
const PARAMS = new Set(['m1', 'p1', '2'])

/**
 * Segment by segment, so `/api/models/studio/families` is NOT satisfied by
 * `/api/models/{id}`: a literal segment of the client path must be a literal
 * segment of the route, and a `{param}` segment must be one of the ids passed.
 */
function matches(pattern: string, url: string): boolean {
  const p = pattern.split('/')
  const u = url.split('/')
  if (p.length !== u.length) return false
  return p.every((seg, i) => (seg.startsWith('{') ? PARAMS.has(u[i]) : seg === u[i]))
}

describe('the studio client', () => {
  it('calls only paths the server routes', async () => {
    calls.length = 0
    await studioApi.families()
    await studioApi.validateDeclaration({ declaration: {} as never })
    await studioApi.fleet()
    await studioApi.verdict('m1', 2)
    await studioApi.promotionEvidence('m1', 2)
    await studioApi.propose('m1', 2, { rationale: 'r' })
    await studioApi.proposals('m1')
    await studioApi.decide('p1', { decision: 'approved', note: 'n' })
    await studioApi.importance('m1', 2)
    await studioApi.train('m1', {})
    expect(calls).toHaveLength(10)

    const served = servedPatterns()
    expect(served.length).toBeGreaterThan(50)
    const unserved = calls.filter((c) => !served.some((p) => matches(p, c.url))).map((c) => `${c.method} ${c.url}`)
    expect(unserved).toEqual([])
  })

  it('has no method for promoting an alias directly: promotion is a proposal', () => {
    expect('promote' in studioApi).toBe(false)
  })
})
