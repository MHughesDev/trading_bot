import { describe, expect, it } from 'vitest'
import type { AiModel, ModelDeployment, ModelVersion } from '@/api/mlops'
import {
  aliasSlots,
  appendSample,
  buildActivity,
  compareToBest,
  confusionMatrix,
  deriveLineage,
  etaSeconds,
  failedRequiredChecks,
  layoutLineage,
  metricDeltas,
  metricSeries,
  normaliseUsedBy,
  parseCompare,
  parsePromoteResult,
  pinProduction,
  preflightBlocksLive,
  preflightChecks,
  primaryMetric,
  readViewMode,
  rollbackTarget,
  sortModels,
  summariseModel,
  trafficAllocated,
  traceStats,
  trustedWinner,
  validateTraffic,
  writeViewMode,
  type EvalRow,
} from '../studio.helpers'

const ver = (version: number, metrics: Record<string, unknown> | undefined, status: ModelVersion['status'] = 'candidate'): ModelVersion => ({
  version,
  model_id: 'm1',
  status,
  metrics,
  artifact_hash: 'abc',
  created_at: '2026-10-01T00:00:00Z',
})

const dep = (traffic: number, environment = 'paper', status = 'active'): ModelDeployment => ({
  deployment_id: `d${traffic}${environment}${status}`,
  version: 1,
  environment,
  alias: 'production',
  status,
  traffic_pct: traffic,
  deployed_at: '2026-10-01T00:00:00Z',
})

describe('metrics are measured or absent, never zero', () => {
  it('has no primary metric when no headline metric was recorded', () => {
    expect(primaryMetric({ n_train: 120, objective: 'binary' })).toBeNull()
    expect(primaryMetric(undefined)).toBeNull()
  })

  it('picks the headline metric before any other numeric one', () => {
    expect(primaryMetric({ n_train: 5, val_auc: 0.61, accuracy: 0.5 })).toEqual({ key: 'val_auc', value: 0.61 })
  })

  it('draws a series only from versions reporting the same metric as the newest', () => {
    const s = metricSeries([
      ver(1, { val_auc: 0.5 }),
      ver(2, { rmse: 1.2 }),
      ver(3, { val_auc: 0.7 }),
    ])
    expect(s).toEqual({ key: 'val_auc', points: [0.5, 0.7] })
  })

  it('returns no series for no versions', () => {
    expect(metricSeries(undefined)).toEqual({ key: null, points: [] })
  })
})

describe('used-by accepts the shape the route serves', () => {
  it('reads strategy_id/display_name and the older id/name', () => {
    expect(
      normaliseUsedBy([
        { strategy_id: 's1', display_name: 'Trend' },
        { id: 's2', name: 'Mean rev' },
        { display_name: 'no id' },
        null,
      ]),
    ).toEqual([
      { id: 's1', name: 'Trend' },
      { id: 's2', name: 'Mean rev' },
    ])
  })

  it('treats a non-array as no rows rather than throwing', () => {
    expect(normaliseUsedBy({ strategies: [] })).toEqual([])
  })
})

describe('the card quotes the production version, not the latest', () => {
  it('uses the production version metric when one is promoted', () => {
    const s = summariseModel({
      versions: [ver(1, { val_auc: 0.9 }), ver(2, { val_auc: 0.4 })],
      aliases: { production: 1 },
    })
    expect(s.primary?.value).toBe(0.9)
    expect(s.productionVersion).toBe(1)
    expect(s.latestVersion).toBe(2)
  })

  it('leaves used-by unknown until its read answered', () => {
    expect(summariseModel({ versions: [] }).usedBy).toBeNull()
    expect(summariseModel({ usedBy: [] }).usedBy).toEqual([])
  })
})

describe('sorting', () => {
  const m = (id: string, name: string): AiModel =>
    ({ model_id: id, display_name: name, model_kind: 'forecaster', status: 'draft', updated_at: '2026-10-01T00:00:00Z' }) as AiModel
  const models = [m('a', 'Alpha'), m('b', 'Beta'), m('c', 'Gamma')]
  const summaries = {
    a: summariseModel({ versions: [ver(1, { val_auc: 0.6 })] }),
    b: summariseModel({ versions: [ver(1, {})] }),
    c: summariseModel({ versions: [ver(1, { val_auc: 0.8 })] }),
  }

  it('sorts a model with no metric last in both directions', () => {
    expect(sortModels(models, summaries, 'metric', 'desc').map((x) => x.model_id)).toEqual(['c', 'a', 'b'])
    expect(sortModels(models, summaries, 'metric', 'asc').map((x) => x.model_id)).toEqual(['a', 'c', 'b'])
  })

  it('sorts by name', () => {
    expect(sortModels(models, summaries, 'name', 'desc').map((x) => x.model_id)).toEqual(['c', 'b', 'a'])
  })
})

describe('the table toggle survives broken storage', () => {
  it('reads table and defaults to cards', () => {
    expect(readViewMode({ getItem: () => 'table' })).toBe('table')
    expect(readViewMode({ getItem: () => null })).toBe('cards')
    expect(readViewMode(null)).toBe('cards')
  })

  it('does not throw when storage throws', () => {
    const bad = {
      getItem: () => {
        throw new Error('blocked')
      },
      setItem: () => {
        throw new Error('full')
      },
    }
    expect(readViewMode(bad)).toBe('cards')
    expect(() => writeViewMode(bad, 'table')).not.toThrow()
  })

  it('round-trips through a real store', () => {
    const store = new Map<string, string>()
    const s = { getItem: (k: string) => store.get(k) ?? null, setItem: (k: string, v: string) => void store.set(k, v) }
    writeViewMode(s, 'table')
    expect(readViewMode(s)).toBe('table')
  })
})

describe('a refused gate is not a promotion', () => {
  it('reads promoted:false from the body', () => {
    const r = parsePromoteResult({
      promoted: false,
      reason: 'gate_failed',
      gate_checks: [
        { check: 'passed_eval_suite', passed: false, required: true },
        { check: 'rollback_available', passed: false, required: false },
      ],
    })
    expect(r.promoted).toBe(false)
    expect(r.refused).toBe(true)
    expect(failedRequiredChecks(r.checks).map((c) => c.check)).toEqual(['passed_eval_suite'])
  })

  it('treats a missing body as not promoted', () => {
    expect(parsePromoteResult(undefined).promoted).toBe(false)
  })

  it('reads a real promotion and its override reason', () => {
    const r = parsePromoteResult({ promoted: true, override_reason: 'hotfix' })
    expect(r.promoted).toBe(true)
    expect(r.overrideReason).toBe('hotfix')
  })

  it('flags an unevaluated version and leaves regression to the server', () => {
    const checks = preflightChecks({ status: 'draft', artifact_hash: 'h' }, true, 'live')
    expect(checks.find((c) => c.check === 'passed_eval_suite')?.state).toBe('fail')
    expect(checks.find((c) => c.check === 'no_metric_regression')?.state).toBe('server')
    expect(preflightBlocksLive(checks)).toBe(true)
  })

  it('needs a prior production only for live', () => {
    const live = preflightChecks({ status: 'candidate', artifact_hash: 'h' }, false, 'live')
    const paper = preflightChecks({ status: 'candidate', artifact_hash: 'h' }, false, 'paper')
    expect(live.find((c) => c.check === 'rollback_available')?.state).toBe('fail')
    expect(paper.find((c) => c.check === 'rollback_available')?.state).toBe('pass')
  })
})

describe('traffic split validates against the active deployments', () => {
  it('counts only active deployments in the same environment', () => {
    const deps = [dep(60), dep(30, 'live'), dep(20, 'paper', 'stopped')]
    expect(trafficAllocated(deps, 'paper')).toBe(60)
    expect(trafficAllocated(deps, 'live')).toBe(30)
  })

  it('refuses an allocation over 100 and says why', () => {
    const r = validateTraffic(50, [dep(60)], 'paper')
    expect(r.ok).toBe(false)
    expect(r.remaining).toBe(40)
    expect(r.message).toMatch(/exceed 100%/)
  })

  it('accepts exactly 100 in total and refuses out-of-range input', () => {
    expect(validateTraffic(40, [dep(60)], 'paper').ok).toBe(true)
    expect(validateTraffic(101, [], 'paper').ok).toBe(false)
    expect(validateTraffic(-1, [], 'paper').ok).toBe(false)
    expect(validateTraffic(Number.NaN, [], 'paper').ok).toBe(false)
  })
})

describe('aliases and rollback', () => {
  it('lists all four canonical slots with unset ones null', () => {
    expect(aliasSlots({ production: 3 })).toEqual([
      { alias: 'production', version: 3 },
      { alias: 'candidate', version: null },
      { alias: 'staging', version: null },
      { alias: 'fallback', version: null },
    ])
  })

  it('rolls back to the next lower version, and to nothing from the first', () => {
    expect(rollbackTarget(5, [5, 4, 2, 1])).toBe(4)
    expect(rollbackTarget(1, [5, 4, 2, 1])).toBeNull()
    expect(rollbackTarget(null, [1])).toBeNull()
  })
})

const row = (version: number, createdAt: string): EvalRow => ({
  evalId: `e${version}`,
  version,
  status: 'succeeded',
  metrics: [],
  primary: null,
  scorecard: null,
  verdict: null,
  checks: [],
  baselineVersion: null,
  createdAt,
  rawMetrics: null,
})

describe('the arena pins production', () => {
  it('puts the production row first and the rest newest first', () => {
    const rows = [row(1, '2026-10-01T00:00:00Z'), row(2, '2026-10-03T00:00:00Z'), row(3, '2026-10-02T00:00:00Z')]
    expect(pinProduction(rows, 1).map((r) => r.version)).toEqual([1, 2, 3])
    expect(pinProduction(rows, null).map((r) => r.version)).toEqual([2, 3, 1])
  })
})

const card = (overall: number) => ({ overall, quality: 1, speed: 1, cost: 1, safety: 1, reliability: 1 })

describe('a winner needs a measurement on both sides', () => {
  it('does not crown the side with an evaluation over the side with none', () => {
    const c = parseCompare({
      versions: [
        { version: 1, metrics: null, scorecard: null },
        { version: 2, metrics: { val_auc: 0.6 }, scorecard: card(55) },
      ],
      winner_version: 2,
    })
    expect(c).not.toBeNull()
    const w = trustedWinner(c!)
    expect(w.version).toBeNull()
    expect(w.reason).toMatch(/v1 has no completed evaluation/)
  })

  it('names the higher overall score, and calls a tie a tie', () => {
    const mk = (a: number, b: number) =>
      parseCompare({
        versions: [
          { version: 1, metrics: {}, scorecard: card(a) },
          { version: 2, metrics: {}, scorecard: card(b) },
        ],
      })!
    expect(trustedWinner(mk(60, 70)).version).toBe(2)
    expect(trustedWinner(mk(80, 70)).version).toBe(1)
    expect(trustedWinner(mk(70, 70)).version).toBeNull()
  })

  it('rejects a compare body that is not two sides', () => {
    expect(parseCompare({ versions: [{ version: 1 }] })).toBeNull()
    expect(parseCompare(null)).toBeNull()
  })
})

describe('metric deltas', () => {
  const side = (version: number, metrics: Array<[string, number]>) => ({ version, metrics, scorecard: null })

  it('has no delta for a metric one side never reported', () => {
    const d = metricDeltas(side(1, [['val_auc', 0.6]]), side(2, [['val_auc', 0.7], ['accuracy', 0.5]]))
    expect(d.find((x) => x.key === 'accuracy')).toMatchObject({ a: null, b: 0.5, delta: null, better: null })
  })

  it('treats a lower error as better and a higher accuracy as better', () => {
    const d = metricDeltas(side(1, [['rmse', 1], ['val_auc', 0.6]]), side(2, [['rmse', 0.8], ['val_auc', 0.7]]))
    expect(d.find((x) => x.key === 'rmse')?.better).toBe('b')
    expect(d.find((x) => x.key === 'val_auc')?.better).toBe('b')
    const worse = metricDeltas(side(1, [['rmse', 1]]), side(2, [['rmse', 1.4]]))
    expect(worse[0].better).toBe('a')
  })
})

describe('structured evaluation values are drawn only when present', () => {
  it('accepts a square matrix and nothing else', () => {
    expect(confusionMatrix({ confusion_matrix: [[5, 1], [2, 7]] })).toEqual([[5, 1], [2, 7]])
    expect(confusionMatrix({ confusion_matrix: [[5, 1]] })).toBeNull()
    expect(confusionMatrix({ confusion_matrix: [[5, 'x'], [2, 7]] })).toBeNull()
    expect(confusionMatrix({})).toBeNull()
    expect(confusionMatrix(null)).toBeNull()
  })
})

describe('training console', () => {
  it('drops a repeated reading but keeps a changed one', () => {
    let s = appendSample([], { t: 0, progress: 10 })
    s = appendSample(s, { t: 2000, progress: 10 })
    expect(s).toHaveLength(1)
    s = appendSample(s, { t: 4000, progress: 20 })
    expect(s).toHaveLength(2)
  })

  it('estimates an ETA from the observed rate and is silent without one', () => {
    expect(etaSeconds([])).toBeNull()
    expect(etaSeconds([{ t: 0, progress: 10 }, { t: 10_000, progress: 10 }])).toBeNull()
    expect(etaSeconds([{ t: 0, progress: 0 }, { t: 10_000, progress: 50 }])).toBe(10)
    expect(etaSeconds([{ t: 0, progress: 0 }, { t: 10_000, progress: 100 }])).toBeNull()
  })

  it('compares a run to the best earlier version by direction', () => {
    const versions = [ver(1, { val_auc: 0.6, rmse: 2 }), ver(2, { val_auc: 0.7, rmse: 1.5 })]
    const rows = compareToBest({ val_auc: 0.65, rmse: 1.0 }, versions)
    const auc = rows.find((r) => r.key === 'val_auc')!
    expect(auc).toMatchObject({ best: 0.7, bestVersion: 2, better: false })
    const rmse = rows.find((r) => r.key === 'rmse')!
    expect(rmse).toMatchObject({ best: 1.5, better: true })
  })

  it('has nothing to compare when no earlier version shares a metric', () => {
    expect(compareToBest({ val_auc: 0.6 }, [ver(1, { rmse: 1 })])).toEqual([])
    expect(compareToBest({ val_auc: 0.6 }, undefined)).toEqual([])
  })
})

describe('derived lineage', () => {
  const g = deriveLineage({
    modelId: 'm1',
    modelName: 'Alpha',
    versions: [ver(1, {}), ver(2, {})],
    runs: [
      { run_id: 'aaaaaaaa-1', kind: 'training', status: 'succeeded' },
      { run_id: 'bbbbbbbb-2', kind: 'evaluation', status: 'succeeded' },
    ],
    deployments: [dep(100)],
    aliases: { production: 2 },
    usedBy: [{ id: 's1', name: 'Trend' }],
  })

  it('links the model to its versions, deployments to their version and strategies to production', () => {
    const ids = g.nodes.map((n) => n.id)
    expect(ids).toContain('version:1')
    expect(ids).toContain('strategy:s1')
    expect(g.edges.some((e) => e.source === 'version:1' && e.target.startsWith('deployment:'))).toBe(true)
    expect(g.edges.some((e) => e.source === 'version:2' && e.target === 'strategy:s1')).toBe(true)
  })

  it('draws only training runs and never invents a dataset', () => {
    expect(g.nodes.filter((n) => n.type === 'training_run')).toHaveLength(1)
    expect(g.nodes.some((n) => n.label.toLowerCase().includes('dataset'))).toBe(false)
  })

  it('places every node and none overlaps in its column', () => {
    const pos = layoutLineage(g.nodes)
    expect(pos.size).toBe(g.nodes.length)
    const seen = new Set([...pos.values()].map((p) => `${p.x},${p.y}`))
    expect(seen.size).toBe(g.nodes.length)
  })
})

describe('activity', () => {
  it('merges runs, versions and deployments newest first', () => {
    const items = buildActivity({
      runs: [{ run_id: 'r1xxxxxxxx', kind: 'training', status: 'succeeded', created_at: '2026-10-02T00:00:00Z' }],
      versions: [ver(1, {})],
      deployments: [{ ...dep(100), deployed_at: '2026-10-03T00:00:00Z' }],
    })
    expect(items.map((i) => i.key)).toEqual([expect.stringMatching(/^dep:/), 'run:r1xxxxxxxx', 'version:1'])
  })
})

describe('trace stats are over the traces served, and abstention is not an attempt', () => {
  const t = (result: string, latency_ms: number) => ({ result, latency_ms })

  it('is unknown, not zero, with no traces', () => {
    expect(traceStats(undefined)).toBeNull()
    expect(traceStats([])).toMatchObject({ n: 0, p95LatencyMs: null, errorRate: null })
  })

  it('takes p95 over real calls only and error rate over attempts only', () => {
    const traces = [
      ...Array.from({ length: 19 }, (_, i) => t('miss', 100 + i)),
      t('miss', 900),
      t('hit', 1),
      t('error', 5),
      t('abstain', 0),
      t('abstain', 0),
    ]
    const s = traceStats(traces)!
    expect(s.n).toBe(24)
    expect(s.p95LatencyMs).toBe(118)
    expect(s.abstained).toBe(2)
    // 1 error of (1 hit + 20 miss + 1 error) attempts
    expect(s.errorRate).toBeCloseTo(1 / 22, 5)
  })
})
