/**
 * The Tools panel explained its emptiness with a reason that had stopped being
 * true (AGENT-012 §3.5, §6.5; Set S task S-6.11).
 *
 * It rendered "The catalogue endpoint is not built yet (AGENT-006, Set S phase
 * 3)" whenever `catalogue` was null — and `Conversation.tsx` passed a literal
 * `null`, because nothing in the conversation ever called `/api/catalogue`. That
 * endpoint has been served since S-6.19. So the panel was a built surface,
 * reading from a built endpoint it never called, telling the operator the
 * endpoint did not exist.
 *
 * That is worse than saying nothing: an empty panel invites you to look
 * elsewhere, and a panel that names a phase invites you to stop looking until
 * that phase lands. It is also this repository's dominant defect shape — complete
 * code with no caller — wearing its own excuse.
 *
 * The three states below have to stay three, because they are three different
 * instructions to the person reading: wait, check the logs, and believe it.
 */
import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import type { ToolCard } from '@/api/desk'
import { ToolsTab } from '../RightPanel'

const tools: ToolCard[] = [
  { name: 'get_bars', namespace: 'data', risk: 'read', spends_trial: false, ring: 'research' },
  { name: 'post_webhook', namespace: 'net', risk: 'outbound', spends_trial: false, ring: 'research' },
]

describe('the catalogue panel has three empty states, not one', () => {
  it('says the request failed, never that the endpoint is unbuilt', () => {
    render(<ToolsTab mode="research" exposed={null} catalogue={null} searchedFor={[]} />)
    const note = screen.getByTestId('catalogue-unavailable')
    expect(note.textContent).toContain('/api/catalogue is served')
    // The stale excuse, gone. A grep rather than a paraphrase, because the exact
    // sentence is what sent a reader away to wait for a landed phase.
    expect(note.textContent).not.toContain('not built yet')
    expect(screen.queryByTestId('catalogue-empty')).toBeNull()
  })

  it('distinguishes "the registry is empty" from "the request failed"', () => {
    render(<ToolsTab mode="research" exposed={null} catalogue={[]} searchedFor={[]} />)
    expect(screen.getByTestId('catalogue-empty')).toBeInTheDocument()
    expect(screen.queryByTestId('catalogue-unavailable')).toBeNull()
  })

  it('reports a request in flight as neither of those', () => {
    render(<ToolsTab mode="research" exposed={null} catalogue={null} catalogueLoading searchedFor={[]} />)
    expect(screen.queryByTestId('catalogue-unavailable')).toBeNull()
    expect(screen.queryByTestId('catalogue-empty')).toBeNull()
    expect(screen.getByText(/Reading the catalogue/)).toBeInTheDocument()
  })
})

describe('a tool card says what the tool can do before you pin it (§6.5)', () => {
  it('carries the risk badge that was on the type and on the wire all along', () => {
    render(<ToolsTab mode="research" exposed={null} catalogue={tools} searchedFor={[]} />)
    expect(screen.getByText('read')).toBeInTheDocument()
    expect(screen.getByText('outbound')).toBeInTheDocument()
    // The card itself rendered, so the two assertions above are not matching an
    // empty panel's stray text.
    expect(screen.getByText('get_bars')).toBeInTheDocument()
  })

  it('says a search matched nothing rather than showing a blank list', () => {
    // A filtered-to-nothing list and a catalogue with nothing in it are the same
    // blank space and different facts (Meridian §3.22).
    render(<ToolsTab mode="research" exposed={null} catalogue={tools} searchedFor={[]} />)
    fireEvent.change(screen.getByLabelText('Search tools'), { target: { value: 'zzz' } })
    expect(screen.queryByText('get_bars')).toBeNull()
    expect(screen.getByText(/No tool this mode can see matches/)).toBeInTheDocument()
    expect(screen.queryByTestId('catalogue-empty')).toBeNull()
  })
})
