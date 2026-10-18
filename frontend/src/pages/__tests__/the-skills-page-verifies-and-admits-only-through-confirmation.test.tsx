/**
 * RA-4.3: the skills page renders what the registry served, and its writes are deliberate.
 *
 *   * Registry: the glossary with resident skills marked; search says it is lexical.
 *   * Proposals: admit is off until a verify says admissible; the confirmation needs the typed
 *     word, and nothing is POSTed to /admit before it.
 *   * Quarantine: needs a reason, and the registry's refusal is shown in its own words.
 *   * Notices: what live sessions were told, newest first.
 *   * A registry that did not load is an error, never an empty registry.
 */
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { MemoryRouter } from 'react-router-dom'
import { beforeEach, describe, expect, it, vi } from 'vitest'

const h = vi.hoisted(() => ({ get: vi.fn(), post: vi.fn() }))
vi.mock('@/lib/api', () => ({
  api: { get: h.get, post: h.post },
  getStoredToken: () => 'tok',
}))

import { SkillsPage } from '@/pages/SkillsPage'

function mount() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(
    <QueryClientProvider client={qc}>
      <MemoryRouter>
        <SkillsPage />
      </MemoryRouter>
    </QueryClientProvider>,
  )
}

function serve(map: Record<string, unknown>) {
  h.get.mockImplementation((url: string) => {
    const v = map[url]
    if (v === undefined) return Promise.reject({ response: { status: 404 } })
    return Promise.resolve({ data: v })
  })
}

beforeEach(() => {
  h.get.mockReset()
  h.post.mockReset()
})

const skill = (name: string, over: Record<string, unknown> = {}) => ({
  skill_id: `id-${name}`,
  name,
  scope: 'global',
  status: 'admitted',
  category: 'research',
  description: `${name} description`,
  version: 'abc123',
  pinned: false,
  ...over,
})

const proposal = {
  proposal_id: 'p1',
  name: 'exit-review',
  project_id: 'proj',
  proposer: 'agent',
  reason: 'T2',
  status: 'submitted',
  similarity: 0.4,
  created_at: '2026-10-04T10:00:00Z',
}

describe('the registry tab', () => {
  it('lists skills with the resident ones marked, and a read that failed is not an empty list', async () => {
    serve({
      '/api/skills/glossary': { skills: [skill('alpha'), skill('beta')], resident: ['alpha'], cut: [] },
    })
    mount()
    await screen.findByTestId('registry-table')
    expect(screen.getByTestId('skill-alpha').textContent).toMatch(/resident/)
    expect(screen.getByTestId('skill-beta').textContent).toMatch(/on demand/)
  })

  it('says the registry could not be read, rather than showing none', async () => {
    h.get.mockRejectedValue({ response: { status: 500, data: { message: 'database down' } } })
    mount()
    await screen.findByText(/Could not read the registry/)
    expect(screen.queryByTestId('registry-table')).toBeNull()
  })

  it('searches the bodies and prints that the search is lexical', async () => {
    serve({
      '/api/skills/glossary': { skills: [skill('alpha')], resident: [], cut: [] },
      '/api/skills/search': {
        mode: 'lexical',
        embedder: false,
        note: 'ranked by full-text match over the whole skill body',
        results: [{ ...skill('alpha'), rank: 0.4 }],
      },
    })
    mount()
    await screen.findByTestId('registry-table')
    fireEvent.change(screen.getByLabelText('Search skill bodies'), { target: { value: 'exit' } })
    const mode = await screen.findByTestId('search-mode')
    expect(mode.textContent).toMatch(/lexical/)
  })

  it('quarantine needs a reason and shows the server refusal in its own words', async () => {
    serve({
      '/api/skills/glossary': { skills: [skill('alpha')], resident: [], cut: [] },
      '/api/skills/item/id-alpha': {
        skill_id: 'id-alpha',
        name: 'alpha',
        scope: 'global',
        status: 'admitted',
        category: 'research',
        current_version: 'abc123',
        pinned: false,
        alias_of: null,
        resolves_to: null,
        usage: { exposures: 20, activations: 5, balance: 3 },
        versions: [],
      },
    })
    h.post.mockRejectedValue({ response: { status: 403, data: { message: 'needs the `skills.admit` scope' } } })
    mount()
    fireEvent.click(await screen.findByText('alpha'))
    await screen.findByTestId('skill-usage')
    expect(screen.getByTestId('skill-usage').textContent).toMatch(/5 of 20/)
    fireEvent.click(screen.getByTestId('quarantine-open'))
    const confirm = await screen.findByTestId('quarantine-confirm')
    expect((confirm as HTMLButtonElement).disabled).toBe(true)
    fireEvent.change(screen.getByLabelText(/Reason/), { target: { value: 'leaks a ticker' } })
    fireEvent.click(confirm)
    await screen.findByText(/skills\.admit/)
    expect(h.post).toHaveBeenCalledWith('/api/skills/id-alpha/quarantine', { reason: 'leaks a ticker' })
  })
})

describe('a quarantine that the registry accepts', () => {
  it('re-reads the glossary, so the skill leaves the list without a reload', async () => {
    let glossary = { skills: [skill('alpha')], resident: [] as string[], cut: [] }
    h.get.mockImplementation((url: string) => {
      if (url === '/api/skills/glossary') return Promise.resolve({ data: glossary })
      if (url.startsWith('/api/skills/item/')) {
        return Promise.resolve({
          data: { skill_id: 'id-alpha', name: 'alpha', scope: 'global', status: 'admitted', category: 'research',
            current_version: 'abc123', pinned: false, alias_of: null, resolves_to: null,
            usage: { exposures: 0, activations: 0, balance: 0 }, versions: [] },
        })
      }
      return Promise.reject({ response: { status: 404 } })
    })
    h.post.mockImplementation(() => {
      glossary = { skills: [], resident: [], cut: [] }
      return Promise.resolve({ data: { skill_id: 'id-alpha', status: 'quarantined' } })
    })
    mount()
    fireEvent.click(await screen.findByText('alpha'))
    fireEvent.click(await screen.findByTestId('quarantine-open'))
    fireEvent.change(await screen.findByLabelText(/Reason/), { target: { value: 'leaks a ticker' } })
    fireEvent.click(screen.getByTestId('quarantine-confirm'))
    await waitFor(() => expect(screen.queryByTestId('skill-alpha')).toBeNull())
    expect(await screen.findByText('No skill is visible to you')).toBeTruthy()
  })
})

describe('the proposals tab', () => {
  async function open() {
    mount()
    fireEvent.click(await screen.findByRole('radio', { name: 'Proposals' }))
  }

  it('offers admit only after a verify that says admissible, and only through the typed confirmation', async () => {
    serve({
      '/api/skills/glossary': { skills: [], resident: [], cut: [] },
      '/api/skills/proposals': { proposals: [proposal] },
    })
    h.post.mockImplementation((url: string) => {
      if (url.endsWith('/verify')) {
        return Promise.resolve({
          data: {
            proposal_id: 'p1',
            steps: { lint: true, reviewer: true },
            required: ['lint', 'reviewer'],
            first_unmet: null,
            admissible: true,
          },
        })
      }
      return Promise.resolve({ data: { skill_id: 's9', status: 'provisional' } })
    })
    await open()
    await screen.findByTestId('proposals-table')
    const admit = screen.getByRole('button', { name: 'Admit' }) as HTMLButtonElement
    expect(admit.disabled).toBe(true)
    expect(screen.getByText(/run verify first/)).toBeTruthy()

    fireEvent.click(screen.getByRole('button', { name: 'Verify' }))
    await screen.findByTestId('verification')
    await waitFor(() => expect((screen.getByRole('button', { name: 'Admit' }) as HTMLButtonElement).disabled).toBe(false))
    expect(h.post).not.toHaveBeenCalledWith('/api/skills/proposals/p1/admit')

    fireEvent.click(screen.getByRole('button', { name: 'Admit' }))
    const go = await screen.findByRole('button', { name: 'Admit as provisional' })
    expect((go as HTMLButtonElement).disabled).toBe(true)
    expect(h.post).not.toHaveBeenCalledWith('/api/skills/proposals/p1/admit')
    fireEvent.change(screen.getByPlaceholderText('ADMIT'), { target: { value: 'ADMIT' } })
    fireEvent.click(go)
    await waitFor(() => expect(h.post).toHaveBeenCalledWith('/api/skills/proposals/p1/admit'))
  })

  it('names the owed step and keeps admit off when the verification is not admissible', async () => {
    serve({
      '/api/skills/glossary': { skills: [], resident: [], cut: [] },
      '/api/skills/proposals': { proposals: [proposal] },
    })
    h.post.mockResolvedValue({
      data: {
        proposal_id: 'p1',
        steps: { lint: true },
        required: ['lint', 'reviewer'],
        first_unmet: 'reviewer',
        admissible: false,
      },
    })
    await open()
    await screen.findByTestId('proposals-table')
    fireEvent.click(screen.getByRole('button', { name: 'Verify' }))
    await screen.findByText(/step `reviewer` has not passed/)
    expect((screen.getByRole('button', { name: 'Admit' }) as HTMLButtonElement).disabled).toBe(true)
  })
})

describe('the notices tab', () => {
  it('lists what sessions were told, newest first', async () => {
    serve({
      '/api/skills/glossary': { skills: [], resident: [], cut: [] },
      '/api/skills/notices': {
        notices: [
          { notice_id: 1, skill: 'old', kind: 'deprecated', reason: 'replaced' },
          { notice_id: 2, skill: 'leaky', kind: 'quarantined', reason: 'names a ticker' },
        ],
      },
    })
    mount()
    fireEvent.click(await screen.findByRole('radio', { name: 'Notices' }))
    const list = await screen.findByTestId('notices')
    const items = list.querySelectorAll('li')
    expect(items[0].textContent).toMatch(/leaky was quarantined: names a ticker/)
    expect(items[1].textContent).toMatch(/old was deprecated/)
  })
})
