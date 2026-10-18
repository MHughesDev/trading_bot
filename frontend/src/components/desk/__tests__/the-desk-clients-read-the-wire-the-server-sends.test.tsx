/**
 * S-6.10 and S-6.21: the two clients whose routes were registered on 2026-10-03,
 * checked against the **bytes the server writes**, not against a fixture the
 * client's own author shaped.
 *
 * Both routes existed before they were reachable, and each client was written
 * against a handler nobody could call. The failure that leaves is a client that
 * compiles, passes its own tests, and reads a field the server never sends. So
 * the frame below is the literal text `desk_digest::kill_switch_frame` emits
 * (`id:`, `event:` and a `data:` line carrying `seq`, `kind`, `content`,
 * `created_at`), pushed through the hook's own frame parser and the banner's own
 * reducer; and the brief is the literal JSON `desk_running::brief` builds.
 */
import { render, screen, waitFor } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { MemoryRouter } from 'react-router-dom'
import { describe, expect, it, vi } from 'vitest'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'

const h = vi.hoisted(() => ({ get: vi.fn() }))
vi.mock('@/lib/api', async (orig) => ({
  ...(await orig<typeof import('@/lib/api')>()),
  api: { get: h.get, post: vi.fn(), delete: vi.fn() },
}))

import { parseFrame } from '@/hooks/useDeskStream'
import { applyKillSwitchFrame, KILL_SWITCH_STREAM } from '@/components/desk/KillSwitchBanner.helpers'
import { deskApi } from '@/api/desk'
import { MemoryTab } from '@/components/desk/RightPanel'
import type { WireEvent } from '@/lib/desk/events'

describe('S-6.21: the kill-switch frame the server writes halts the banner', () => {
  // `Event::default().id("2").event("kill_switch").data(json)` as it appears on
  // the wire, then the blank line that ends the frame.
  const onTheWire =
    'id: 2\n' +
    'event: kill_switch\n' +
    'data: {"seq":2,"kind":"kill_switch","content":{"readable":true,"global_halted":true,' +
    '"halted_scopes":["global"],"cause":"global"},"created_at":"2026-10-03T10:00:00Z"}'

  it('parses, and a global halt is applied from the frame with no refetch', () => {
    const { id, data } = parseFrame(onTheWire)
    expect(id).toBe('2')
    const e = JSON.parse(data) as WireEvent
    const r = applyKillSwitchFrame({ trading_enabled: true, kill_switch_active: false }, e.content ?? {})
    expect(r.refetch).toBe(false)
    expect(r.next).toMatchObject({ kill_switch_active: true, trading_enabled: false, scope: 'global' })
  })

  it('the unreadable frame is "go and ask", never armed', () => {
    const wire =
      'id: 3\nevent: kill_switch\n' +
      'data: {"seq":3,"kind":"kill_switch","content":{"readable":false,"message":"pool closed","cause":"connected"},"created_at":"x"}'
    const e = JSON.parse(parseFrame(wire).data) as WireEvent
    expect(applyKillSwitchFrame(null, e.content)).toEqual({ next: null, refetch: true })
  })

  it('subscribes to the path the router registers', () => {
    expect(KILL_SWITCH_STREAM).toBe('/api/desk/kill-switch/events')
    const mod = readFileSync(resolve(__dirname, '../../../../../crates/api/src/routes/mod.rs'), 'utf8')
    expect(mod).toContain('"/api/desk/kill-switch/events"')
  })

  it('the banner sits in the shell, above the outlet, so every route has it', () => {
    const shell = readFileSync(resolve(__dirname, '../../layout/AppLayout.tsx'), 'utf8')
    expect(shell.indexOf('<KillSwitchBanner />')).toBeGreaterThan(-1)
    expect(shell.indexOf('<KillSwitchBanner />')).toBeLessThan(shell.indexOf('<Outlet />'))
  })

  it('the banner has no control that could dismiss it', () => {
    const src = readFileSync(resolve(__dirname, '../KillSwitchBanner.tsx'), 'utf8')
    const jsx = src.slice(src.indexOf('export function KillSwitchBanner'))
    // No handler and no button: the words "dismissed" in its comments are the
    // statement of this property, not a violation of it.
    expect(jsx).not.toMatch(/onClick|<Button|<button|setOpen|hidden=/)
  })
})

describe('S-6.10: the brief the server builds reaches the Memory tab', () => {
  // The literal body of `desk_running::brief`, with `cited` null as it is served
  // while nothing marks a citation.
  const body = {
    run_id: 'r1',
    items: [
      {
        entry_id: 'e1',
        rank: 1,
        slot: 'findings',
        why: 'same instrument and horizon as this goal',
        cited: null,
        cited_at: null,
        retrieved_at: '2026-10-03T09:00:00Z',
        entry: { kind: 'finding', bucket: 'findings', subject: 'BTC-USD', claim: 'momentum decays after 3 days', archived: false, superseded: false },
      },
    ],
    entries_readable: true,
    not_recorded: [{ field: 'cited', why: 'nothing in production marks a retrieval as cited' }],
  }

  it('calls the registered path and carries the payload through', async () => {
    h.get.mockResolvedValueOnce({ data: body })
    const r = await deskApi.brief('r1')
    expect(h.get).toHaveBeenCalledWith('/api/desk/runs/r1/brief')
    expect(r).toEqual({ served: true, data: body })
    const mod = readFileSync(resolve(__dirname, '../../../../../crates/api/src/routes/mod.rs'), 'utf8')
    expect(mod).toContain('"/api/desk/runs/{run_id}/brief"')
  })

  it('a 404 is "not served", which the tab says in words, not an empty brief', async () => {
    h.get.mockRejectedValueOnce({ response: { status: 404 } })
    expect(await deskApi.brief('r1')).toEqual({ served: false })
  })

  it('shows why each entry was retrieved and does not claim a citation nobody measured', async () => {
    const qc = new QueryClient()
    render(
      <QueryClientProvider client={qc}>
        <MemoryRouter>
          <MemoryTab brief={{ served: true, data: body }} />
        </MemoryRouter>
      </QueryClientProvider>,
    )
    await waitFor(() => expect(screen.getByText(/same instrument and horizon as this goal/)).toBeTruthy())
    expect(screen.getByText(/momentum decays after 3 days/)).toBeTruthy()
    // `cited: null` is unmeasured. The badge must not read "cited" or "not cited".
    expect(screen.getByTestId('brief-cited-e1').textContent).toBe('citation not recorded')
  })
})
