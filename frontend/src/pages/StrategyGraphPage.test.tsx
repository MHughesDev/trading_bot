// Set U U-7.5: the graph editor, rendered against a stand-in API.
//
// The palette is the catalogue; the text view shows the server's canonical
// text for the graph; editing the text and applying it replaces the graph; and
// Save sends the graph through `strategyV3Api.save` — the route the agent's
// save_strategy_v3 tool calls.
import { describe, expect, it, vi, beforeAll } from 'vitest'
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import type { Catalogue, StrategyGraph } from '@/api/strategyV3'

const catalogue: Catalogue = {
  operators: [
    { name: 'ema', family: 'temporal', inputs: [{ name: 'x', ty: 'num' }, { name: 'n', ty: 'num', literal: true }], output: 'num', stateful: true, open_inputs: false, cross_sectional: false, use_when: 'Smooth a series.' },
    { name: 'gt', family: 'compare', inputs: [{ name: 'a', ty: 'num' }, { name: 'b', ty: 'num' }], output: 'bool', stateful: false, open_inputs: false, cross_sectional: false, use_when: 'a > b.' },
    { name: 'cs_rank', family: 'cross_sectional', inputs: [{ name: 'x', ty: 'num' }], output: 'num', stateful: false, open_inputs: false, cross_sectional: true, use_when: 'Rank across a universe.' },
  ],
  features: [{ family: 'rsi', example: 'rsi_14', warmup_bars: 15, use_when: 'Momentum.' }],
  models: [],
}

const textGraph: StrategyGraph = {
  format: 'tbot.strategy/3',
  kind: 'trading',
  name: 'from_text',
  asset_class: 'crypto_spot_cex',
  timeframe: '1h',
  nodes: [{ id: 'fast', op: 'ema', args: { x: 'close', n: 5 } }, { id: 'go', op: 'gt', args: { a: 'close', b: 'fast' } }],
  decide: { rules: [{ when: 'go', then: { kind: 'close' } }], otherwise: 'hold' },
}

const build = vi.fn(async (input: { text: string } | { graph: StrategyGraph }) =>
  'text' in input
    ? { valid: true, graph: textGraph, text: input.text, findings: [] }
    : { valid: false, graph: input.graph, text: `strategy ${input.graph.name} kind trading`, findings: [{ code: 'no_rules', at: 'decide', message: 'a trading strategy with no rules never acts', fix: 'add a rule' }] },
)
const save = vi.fn(async (id: string) => ({ strategy_id: id, revision_hash: 'a'.repeat(64), actor_kind: 'human', parent_revision: null, text: '' }))

vi.mock('@/api/strategyV3', async (orig) => ({
  ...(await orig<typeof import('@/api/strategyV3')>()),
  strategyV3Api: { catalogue: async () => catalogue, build: (i: never) => build(i), save: (id: string, g: never) => save(id, g), get: vi.fn() },
}))

beforeAll(() => {
  // React Flow measures; jsdom does not.
  globalThis.ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  } as unknown as typeof ResizeObserver
})

async function renderPage() {
  const { StrategyGraphPage } = await import('./StrategyGraphPage')
  render(
    <QueryClientProvider client={new QueryClient()}>
      <StrategyGraphPage />
    </QueryClientProvider>,
  )
}

describe('the graph editor', () => {
  it('builds its palette from the catalogue, hiding cross-sectional operators in a trading strategy', async () => {
    await renderPage()
    expect(await screen.findByText('ema')).toBeInTheDocument()
    expect(screen.getByText('gt')).toBeInTheDocument()
    expect(screen.getByText('rsi_14')).toBeInTheDocument()
    expect(screen.queryByText('cs_rank')).not.toBeInTheDocument()
  })

  it("shows the server's text and findings for the graph, and the text drives the graph back", async () => {
    await renderPage()
    const area = (await screen.findByLabelText('SLv3 text')) as HTMLTextAreaElement
    await waitFor(() => expect(area.value).toBe('strategy new_strategy kind trading'))
    expect(await screen.findByText('a trading strategy with no rules never acts')).toBeInTheDocument()

    fireEvent.change(area, { target: { value: 'strategy from_text kind trading …' } })
    fireEvent.click(screen.getByText('Apply text'))
    await waitFor(() => expect(build).toHaveBeenCalledWith({ text: 'strategy from_text kind trading …' }))
    // The rule form now offers the nodes the text declared.
    await waitFor(() => expect(screen.getAllByRole('option', { name: 'go' }).length).toBeGreaterThan(0))
    expect((screen.getByLabelText('Strategy name') as HTMLInputElement).value).toBe('from_text')
  })

  it('saves through the same call the agent makes', async () => {
    await renderPage()
    await screen.findByText('ema')
    fireEvent.change(screen.getByLabelText('Strategy id'), { target: { value: 'ema_cross_v3' } })
    fireEvent.click(screen.getByText('Save'))
    await waitFor(() => expect(save).toHaveBeenCalled())
    expect(save.mock.calls[0][0]).toBe('ema_cross_v3')
  })
})
