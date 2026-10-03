// Set U U-7.5: the graph editor's model.
//
// Acceptance tested here: a wire between incompatible ports cannot be made;
// the canvas and the canonical graph are the same strategy (round trip); the
// editor's save is the same API call the agent makes.
import { describe, expect, it } from 'vitest'
import { V3_ROUTES, type Catalogue, type StrategyGraph } from '@/api/strategyV3'
import { canWire, compatible, fromCanvas, kindOfTy, toCanvas } from './graph'

const cat: Catalogue = {
  operators: [
    { name: 'ema', family: 'temporal', inputs: [{ name: 'x', ty: 'num' }, { name: 'n', ty: 'num', literal: true }], output: 'num', stateful: true, open_inputs: false, cross_sectional: false, use_when: 'smooth' },
    { name: 'gt', family: 'compare', inputs: [{ name: 'a', ty: 'num' }, { name: 'b', ty: 'num' }], output: 'bool', stateful: false, open_inputs: false, cross_sectional: false, use_when: 'a > b' },
    { name: 'and', family: 'logic', inputs: [{ name: 'a', ty: 'bool' }, { name: 'b', ty: 'bool' }], output: 'bool', stateful: false, open_inputs: false, cross_sectional: false, use_when: 'both' },
    { name: 'prob', family: 'adapter', inputs: [{ name: 'p', ty: { probs_of: 'L' } }, { name: 'label', ty: 'text', literal: true }], output: 'num', stateful: false, open_inputs: false, cross_sectional: false, use_when: 'p(label)' },
    { name: 'coalesce', family: 'absence', inputs: [{ name: 'x', ty: { var: 'T' } }, { name: 'default', ty: { var: 'T' } }], output: { var: 'T' }, stateful: false, open_inputs: false, cross_sectional: false, use_when: 'fallback' },
    { name: 'weighted_mean', family: 'blend', inputs: [{ name: 'xs', ty: 'num_list' }, { name: 'weights', ty: 'num_list', literal: true }], output: 'num', stateful: false, open_inputs: false, cross_sectional: false, use_when: 'blend' },
  ],
  features: [],
  models: [
    {
      reference: 'clf@1',
      kind: 'classifier',
      contract: {
        inputs: [{ name: 'x', shape: 'scalar', value_type: { type: 'num' } }],
        outputs: [{ name: 'probs', value_type: { type: 'probs', labels: ['down', 'up'] } }, { name: 'class', value_type: { type: 'cat' } }],
      },
    },
  ],
}

const graph: StrategyGraph = {
  format: 'tbot.strategy/3',
  kind: 'trading',
  name: 'g',
  asset_class: 'crypto_spot_cex',
  timeframe: '1h',
  nodes: [
    { id: 'f', op: 'ema', args: { x: 'close', n: '$fast' } },
    { id: 's', op: 'ema', args: { x: 'close', n: 48 } },
    { id: 'up', op: 'gt', args: { a: 'f', b: 's' } },
    { id: 'm', op: 'model', args: { ref: { str: 'clf@1' }, x: 'f' } },
    { id: 'p', op: 'prob', args: { p: 'm.probs', label: { str: 'up' } } },
    { id: 'go', op: 'and', args: { a: 'up', b: { op: 'gt', args: { a: 'p', b: 0.6 } } } },
  ],
  decide: { rules: [{ when: 'go', then: { kind: 'close' } }], otherwise: 'hold' },
  params: { fast: { type: 'int', default: 12, min: 5, max: 50, step: 1 } },
}

describe('typed ports', () => {
  it('a number fits a number port, a condition does not', () => {
    expect(compatible(kindOfTy('num'), kindOfTy('num'))).toBe(true)
    expect(compatible(kindOfTy('bool'), kindOfTy('num'))).toBe(false)
    expect(compatible(kindOfTy({ probs_of: 'L' }), kindOfTy('num'))).toBe(false)
    expect(compatible(kindOfTy('num'), kindOfTy({ var: 'T' }))).toBe(true)
    expect(compatible(kindOfTy('num'), kindOfTy('num_list'))).toBe(true)
    expect(compatible(kindOfTy('bool'), kindOfTy('num_list'))).toBe(false)
  })

  it('a wire between incompatible ports cannot be made, and says why', () => {
    const bad = canWire(graph, cat, { source: 'up', sourceHandle: 'out', target: 's', targetHandle: 'x' })
    expect(bad.ok).toBe(false)
    expect(!bad.ok && bad.why).toMatch(/gives bool.*takes num/)
    expect(canWire(graph, cat, { source: 'm', sourceHandle: 'class', target: 'p', targetHandle: 'p' }).ok).toBe(false)
    expect(canWire(graph, cat, { source: 'm', sourceHandle: 'probs', target: 'p', targetHandle: 'p' }).ok).toBe(true)
    expect(canWire(graph, cat, { source: 's', sourceHandle: 'out', target: 'up', targetHandle: 'a' }).ok).toBe(true)
  })

  it('a literal port takes a value, never a wire', () => {
    const r = canWire(graph, cat, { source: 's', sourceHandle: 'out', target: 'f', targetHandle: 'n' })
    expect(r.ok).toBe(false)
    expect(!r.ok && r.why).toMatch(/takes a value/)
  })

  it('a wire that would close a loop is refused', () => {
    const r = canWire(graph, cat, { source: 'up', sourceHandle: 'out', target: 'f', targetHandle: 'x' })
    expect(r.ok).toBe(false)
    const loop = canWire(graph, cat, { source: 'p', sourceHandle: 'out', target: 'm', targetHandle: 'x' })
    expect(!loop.ok && loop.why).toMatch(/loop/)
  })
})

describe('canvas and graph are one strategy', () => {
  it('round-trips nodes, wires, literals, sources, params and inline nodes', () => {
    const { nodes, wires } = toCanvas(graph)
    expect(wires.map((w) => `${w.source}.${w.sourceHandle}->${w.target}.${w.targetHandle}`).sort()).toEqual(
      ['f.out->m.x', 'f.out->up.a', 'm.probs->p.p', 's.out->up.b', 'up.out->go.a'].sort(),
    )
    expect(nodes.find((n) => n.id === 'f')!.literals).toEqual({ x: 'close', n: '$fast' })
    expect(fromCanvas(graph, nodes, wires)).toEqual(graph)
  })

  it('removing a wire removes the argument; a new wire sets it', () => {
    const { nodes, wires } = toCanvas(graph)
    const without = fromCanvas(graph, nodes, wires.filter((w) => !(w.target === 'up' && w.targetHandle === 'b')))
    expect(without.nodes.find((n) => n.id === 'up')!.args).toEqual({ a: 'f' })
    const rewired = fromCanvas(graph, nodes, [...wires.filter((w) => w.target !== 'up'), { source: 'f', sourceHandle: 'out', target: 'up', targetHandle: 'b' }])
    expect(rewired.nodes.find((n) => n.id === 'up')!.args).toEqual({ b: 'f' })
  })

  it('lays out by dependency depth', () => {
    const { nodes } = toCanvas(graph)
    const x = (id: string) => nodes.find((n) => n.id === id)!.position.x
    expect(x('f')).toBeLessThan(x('up'))
    expect(x('up')).toBeLessThan(x('go'))
  })
})

describe('one API', () => {
  it("the editor saves through the route the agent's save_strategy_v3 calls", () => {
    expect(V3_ROUTES.save).toBe('/api/v3/strategies')
    expect(V3_ROUTES.build).toBe('/api/v3/authoring/build')
  })
})
