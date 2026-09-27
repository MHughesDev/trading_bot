// The product decision, asserted (ADR-0037; AGENT-012 §3.3, §2b, §19.10).
//
//   - Two options and a toggle. Chat spends no trial and reaches no ring-1 or
//     ring-2 tool — and the *desk* cannot offer one either.
//   - Continuous swaps the question field for the charter form, and toggling it
//     off and on again loses nothing.
//   - A Chat question that would need a trial ends with an offer, not a run.
import { render, screen, fireEvent } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import type { ToolCard } from '@/api/desk'
import { modeMaySeeTool, modeMaySpendTrials, toolsVisibleInMode, whyHidden } from '@/lib/desk/rings'
import { Composer } from '../Composer'
import { ToolsTab } from '../RightPanel'
import { classifyNeedsResearch, defaultWorkflow } from '@/components/desk/Composer.helpers'

const tools: ToolCard[] = [
  { name: 'get_bars', namespace: 'data', risk: 'read', spends_trial: false, ring: 'research' },
  { name: 'run_backtest', namespace: 'research', risk: 'write', spends_trial: true, ring: 'research' },
  { name: 'create_strategy_version', namespace: 'platform', risk: 'write', spends_trial: false, ring: 'platform_artifacts' },
  { name: 'deploy_paper', namespace: 'paper', risk: 'destructive', spends_trial: false, ring: 'paper' },
]

describe('a Chat turn cannot reach a trial-spending or outer-ring tool', () => {
  it('spends no trials, by construction', () => {
    expect(modeMaySpendTrials('chat')).toBe(false)
    expect(modeMaySpendTrials('research')).toBe(true)
  })

  it('filters every trial-spending and ring-1/ring-2 tool out of Chat', () => {
    const visible = toolsVisibleInMode(tools, 'chat').map((t) => t.name)
    expect(visible).toEqual(['get_bars'])
    expect(modeMaySeeTool('chat', tools[1])).toBe(false)
    expect(whyHidden('chat', tools[1])).toContain('spends a trial')
  })

  it('Research still stops at ring 0 — rings 1 and 2 are reached by a proposal', () => {
    const visible = toolsVisibleInMode(tools, 'research').map((t) => t.name)
    expect(visible).toEqual(['get_bars', 'run_backtest'])
    expect(whyHidden('research', tools[3])).toContain('ring 2')
  })

  it('the Tools panel renders none of them in Chat, and says why', () => {
    render(<ToolsTab mode="chat" exposed={[]} catalogue={tools} searchedFor={[]} />)
    expect(screen.queryByText('run_backtest')).toBeNull()
    expect(screen.queryByText('deploy_paper')).toBeNull()
    expect(screen.getByText('get_bars')).toBeInTheDocument()
    expect(screen.getByTestId('tools-hidden').textContent).toContain('spends a trial')
  })
})

const profiles = [
  { id: 'claude-opus-5', label: 'claude-opus-5', tier: 'frontier', measured: true, frontier: true },
  { id: 'qwen3.6-35b', label: 'qwen3.6-35b', tier: 'local_high', measured: false, frontier: false },
]

describe('the Continuous toggle', () => {
  function setup() {
    const onSubmit = vi.fn()
    render(
      <Composer
        running={false}
        profiles={profiles}
        model="claude-opus-5"
        onModelChange={() => {}}
        onSubmit={onSubmit}
      />,
    )
    return { onSubmit }
  }

  it('is only offered under Research — Chat has no toggle', () => {
    setup()
    expect(screen.queryByRole('switch', { name: 'Continuous' })).toBeNull()
    fireEvent.click(screen.getByRole('radio', { name: 'Research' }))
    expect(screen.getByRole('switch', { name: 'Continuous' })).toBeInTheDocument()
  })

  it('swaps the question field for the charter form and back without losing typed constraints', () => {
    setup()
    fireEvent.click(screen.getByRole('radio', { name: 'Research' }))
    const question = screen.getByLabelText('Ask, or describe what to research')
    fireEvent.change(question, { target: { value: 'does the overnight gap pay?' } })
    fireEvent.change(screen.getByLabelText('instrument'), { target: { value: 'BTC-USD' } })

    fireEvent.click(screen.getByRole('switch', { name: 'Continuous' }))
    expect(screen.getByTestId('charter-form')).toBeInTheDocument()
    expect(screen.queryByLabelText('Ask, or describe what to research')).toBeNull()
    // The constraint chips survive the swap — they belong to the session, not
    // to either field.
    expect((screen.getByLabelText('instrument') as HTMLInputElement).value).toBe('BTC-USD')
    fireEvent.change(screen.getByLabelText('Instruments'), { target: { value: 'BTC-USD, ETH-USD' } })

    fireEvent.click(screen.getByRole('switch', { name: 'Continuous' }))
    expect(screen.queryByTestId('charter-form')).toBeNull()
    expect((screen.getByLabelText('Ask, or describe what to research') as HTMLTextAreaElement).value).toBe(
      'does the overnight gap pay?',
    )

    fireEvent.click(screen.getByRole('switch', { name: 'Continuous' }))
    expect((screen.getByLabelText('Instruments') as HTMLInputElement).value).toBe('BTC-USD, ETH-USD')
  })

  it('refuses a non-frontier profile inline, naming it', () => {
    const onSubmit = vi.fn()
    render(
      <Composer
        running={false}
        profiles={profiles}
        model="qwen3.6-35b"
        onModelChange={() => {}}
        onSubmit={onSubmit}
      />,
    )
    fireEvent.click(screen.getByRole('radio', { name: 'Research' }))
    fireEvent.click(screen.getByRole('switch', { name: 'Continuous' }))
    expect(screen.getByRole('alert').textContent).toContain('qwen3.6-35b')
    fireEvent.click(screen.getByTestId('send'))
    expect(onSubmit).not.toHaveBeenCalled()
  })
})

describe('a Chat turn that would need a trial ends with an offer', () => {
  it('classifies research-shaped questions', () => {
    expect(classifyNeedsResearch('does the overnight gap on BTC predict the open?')).toBe(true)
    expect(classifyNeedsResearch('sweep the ema window')).toBe(true)
    expect(classifyNeedsResearch('what timezone are the bars in?')).toBe(false)
  })

  it('defaults the workflow from the question shape', () => {
    expect(defaultWorkflow('falsify the momentum claim')).toBe('falsify')
    expect(defaultWorkflow('compare A versus B')).toBe('compare')
    expect(defaultWorkflow('what is this?')).toBe('investigate')
  })

  it('renders the offer instead of submitting a Chat turn that needs a trial', () => {
    const onSubmit = vi.fn()
    const onOffer = vi.fn()
    render(
      <Composer
        running={false}
        profiles={profiles}
        model="claude-opus-5"
        onModelChange={() => {}}
        onSubmit={onSubmit}
        onOffer={onOffer}
      />,
    )
    fireEvent.change(screen.getByLabelText('Ask, or describe what to research'), {
      target: { value: 'is the overnight gap profitable after costs?' },
    })
    fireEvent.click(screen.getByTestId('send'))
    expect(onSubmit).not.toHaveBeenCalled()
    expect(onOffer).toHaveBeenCalledWith('is the overnight gap profitable after costs?')
  })

  it('sends the workflow with a Research turn', () => {
    const onSubmit = vi.fn()
    render(
      <Composer
        running={false}
        profiles={profiles}
        model="claude-opus-5"
        onModelChange={() => {}}
        onSubmit={onSubmit}
      />,
    )
    fireEvent.click(screen.getByRole('radio', { name: 'Research' }))
    fireEvent.change(screen.getByLabelText('Ask, or describe what to research'), {
      target: { value: 'falsify the momentum claim' },
    })
    fireEvent.click(screen.getByTestId('send'))
    expect(onSubmit).toHaveBeenCalledWith(
      expect.objectContaining({ mode: 'research', workflow: 'falsify', continuous: false, steer: false }),
    )
  })

  it('sending while a run is active is a steer, not a queued turn', () => {
    const onSubmit = vi.fn()
    render(
      <Composer
        running
        profiles={profiles}
        model="claude-opus-5"
        onModelChange={() => {}}
        onSubmit={onSubmit}
      />,
    )
    fireEvent.change(screen.getByLabelText('Ask, or describe what to research'), {
      target: { value: 'use 5m bars instead' },
    })
    fireEvent.click(screen.getByTestId('send'))
    expect(onSubmit).toHaveBeenCalledWith(expect.objectContaining({ steer: true }))
  })
})
