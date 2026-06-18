// Compiles the visual builder's RuleStrategySpec into the canonical v1.0
// StrategyDefinition that the Rust backend (`/api/strategies`), the validator,
// and the backtest engine all consume (ADR-0010, DATA-004).
//
// The v1.0 expression grammar is deliberately minimal: a condition node holds a
// single comparison (no boolean `&&`/`||`), a signal node watches one condition,
// and signals that share an `emit` name OR-combine.  Exits and AI-forecast
// conditions have no representation in v1.0 and are reported as warnings/errors
// rather than silently dropped.

import type { Condition, IndicatorSpec, RuleStrategySpec } from '@/types/spec'

export interface StrategyDefinition {
  strategy_id: string
  definition_version: '1.0' | '1.1'
  asset_class: string
  inputs: Array<{ lane: string; instrument: string; features?: string[] }>
  nodes: Array<
    | { id: string; type: 'condition'; expr: string }
    | { id: string; type: 'signal'; when: string; emit: string }
    // v1.1 — AI model forecast node (matches domain NodeKind::ModelForecast)
    | { id: string; type: 'model_forecast'; model_ref: string; alias?: string; direction: string; min_confidence: number }
  >
  actions: Array<{
    on_signal: string
    type: 'place_order'
    order: { side: string; size_mode: string; size: string }
  }>
}

export interface ConvertResult {
  definition: StrategyDefinition | null
  errors: string[]
  warnings: string[]
}

/** Slugifies a display name into a `strategy_id` (`[a-z0-9_]`, non-empty). */
function slugify(name: string): string {
  const s = name
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '_')
    .replace(/^_+|_+$/g, '')
  return s || 'strategy'
}

/** Canonical feature name for an indicator, e.g. `ema_7`. */
function featureName(ind: IndicatorSpec): string {
  return `${ind.kind}_${ind.period}`
}

const COMPARATOR: Partial<Record<Condition['type'], '>' | '<'>> = {
  cross_above: '>',
  greater_than: '>',
  cross_below: '<',
  less_than: '<',
}

/** Builds one operand (`feature('…')`, `bar('close')`, or a numeric literal). */
function operand(
  ref: string | undefined,
  value: number | undefined,
  indById: Map<string, IndicatorSpec>,
  features: Set<string>,
): { expr: string } | { error: string } {
  if (ref !== undefined) {
    if (ref === 'price') return { expr: "bar('close')" }
    const ind = indById.get(ref)
    if (!ind) return { error: `condition references unknown indicator '${ref}'` }
    const name = featureName(ind)
    features.add(name)
    return { expr: `feature('${name}')` }
  }
  if (value !== undefined && Number.isFinite(value)) {
    return { expr: String(value) }
  }
  return { error: 'condition is missing its right-hand operand' }
}

/** Compiles a single builder condition to a v1.0 comparison expression. */
function conditionToExpr(
  cond: Condition,
  indById: Map<string, IndicatorSpec>,
  features: Set<string>,
): { expr: string } | { error: string } {
  if (cond.type === 'rising' || cond.type === 'falling') {
    return {
      error:
        "'Is Rising' / 'Is Falling' conditions aren't supported by the saved v1.0 format yet (it has no access to prior bars).",
    }
  }
  // model_forecast conditions are emitted as a dedicated v1.1 node by
  // `emitConditionNode`, never as an expression — they never reach here.
  const op = COMPARATOR[cond.type]
  if (!op) return { error: `unsupported condition type '${cond.type}'` }

  const left = operand(cond.left, undefined, indById, features)
  if ('error' in left) return left
  const right = operand(cond.right_id, cond.right_value, indById, features)
  if ('error' in right) return right

  return { expr: `${left.expr} ${op} ${right.expr}` }
}

/**
 * Emits the node(s) for a single builder condition into `nodes` and wires a
 * signal that fires on `emit`.  A `model_forecast` condition becomes a v1.1
 * `model_forecast` node; everything else becomes a v1.0 `condition` expression
 * node.  Returns whether a model node was emitted (forces definition_version
 * 1.1) and any compile error.
 */
function emitConditionNode(
  cond: Condition,
  idx: number,
  indById: Map<string, IndicatorSpec>,
  features: Set<string>,
  emit: string,
  nodes: StrategyDefinition['nodes'],
): { hasModel: boolean; error?: string } {
  const signalId = `s${idx + 1}`

  if (cond.type === 'model_forecast') {
    if (!cond.model || !cond.model.trim()) {
      return { hasModel: true, error: 'AI Model node has no model selected.' }
    }
    const nodeId = `m${idx + 1}`
    nodes.push({
      id: nodeId,
      type: 'model_forecast',
      model_ref: cond.model,
      alias: cond.alias || 'production',
      direction: cond.direction ?? 'any',
      min_confidence: cond.right_value ?? 0,
    })
    nodes.push({ id: signalId, type: 'signal', when: nodeId, emit })
    return { hasModel: true }
  }

  const compiled = conditionToExpr(cond, indById, features)
  if ('error' in compiled) return { hasModel: false, error: compiled.error }
  const condId = `c${idx + 1}`
  nodes.push({ id: condId, type: 'condition', expr: compiled.expr })
  nodes.push({ id: signalId, type: 'signal', when: condId, emit })
  return { hasModel: false }
}

/**
 * Converts scanner conditions directly into a canonical v1.0 StrategyDefinition
 * with `actions: []`, producing a Discovery strategy.
 */
export function scannerToDefinition(
  name: string,
  indicators: IndicatorSpec[],
  allOf: Condition[],
  anyOf: Condition[],
  assetClass = 'crypto_spot_cex',
): ConvertResult {
  const errors: string[] = []
  const warnings: string[] = []
  const indById = new Map(indicators.map((i) => [i.id, i]))
  const features = new Set<string>()

  if (indicators.some((i) => i.kind === 'sma' || i.kind === 'atr')) {
    warnings.push('SMA / ATR indicators are not yet supported by the backtest engine.')
  }

  if (allOf.length > 1) {
    errors.push(
      'The saved v1.0 format supports a single entry condition (or OR alternatives); combine the AND-ed conditions into one.',
    )
  }
  if (allOf.length >= 1 && anyOf.length >= 1) {
    errors.push(
      'Mixing required (AND) and alternative (OR) conditions is not supported by the saved v1.0 format yet.',
    )
  }

  const conditions = allOf.length === 1 ? allOf : anyOf
  if (conditions.length === 0 && errors.length === 0) {
    errors.push('Add at least one entry condition.')
  }

  const nodes: StrategyDefinition['nodes'] = []
  const emit = 'scanner_signal'
  let hasModel = false
  conditions.forEach((cond, idx) => {
    const res = emitConditionNode(cond, idx, indById, features, emit, nodes)
    if (res.error) { errors.push(res.error); return }
    if (res.hasModel) hasModel = true
  })

  if (errors.length > 0) return { definition: null, errors, warnings }

  const inputs: StrategyDefinition['inputs'] = [
    { lane: 'market.bars.1m', instrument: '$bound_at_init' },
  ]
  if (features.size > 0) {
    inputs.push({ lane: 'features.technical', instrument: '$bound_at_init', features: [...features] })
  }

  return {
    definition: {
      strategy_id: slugify(name),
      definition_version: hasModel ? '1.1' : '1.0',
      asset_class: assetClass,
      inputs,
      nodes,
      actions: [],
    },
    errors,
    warnings,
  }
}

/**
 * Converts a builder RuleStrategySpec into a canonical v1.0 StrategyDefinition.
 *
 * `assetClass` scopes which instruments the definition may run on (the visual
 * builder doesn't capture it yet, so it defaults to `crypto_spot_cex`).
 */
export function ruleSpecToDefinition(
  spec: RuleStrategySpec,
  assetClass = 'crypto_spot_cex',
): ConvertResult {
  const errors: string[] = []
  const warnings: string[] = []
  const indById = new Map(spec.indicators.map((i) => [i.id, i]))
  const features = new Set<string>()

  // Unsupported-by-backtest indicators still produce a valid definition.
  if (spec.indicators.some((i) => i.kind === 'sma' || i.kind === 'atr')) {
    warnings.push(
      'SMA / ATR indicators are not yet supported by the backtest engine (EMA and RSI are).',
    )
  }

  // The v1.0 grammar has no boolean operators, so AND-combined conditions can't
  // be expressed.  A single condition, or OR-alternatives, are supported.
  if (spec.entry.all.length > 1) {
    errors.push(
      'The saved v1.0 format supports a single entry condition (or OR alternatives); ' +
        'combine the AND-ed conditions into one.',
    )
  }
  if (spec.entry.all.length >= 1 && spec.entry.any.length >= 1) {
    errors.push(
      'Mixing required (AND) and alternative (OR) conditions is not supported by the saved v1.0 format yet.',
    )
  }

  const conditions =
    spec.entry.all.length === 1 ? spec.entry.all : spec.entry.any
  if (conditions.length === 0 && errors.length === 0) {
    errors.push('Add at least one entry condition.')
  }

  const nodes: StrategyDefinition['nodes'] = []
  const emit = 'entry'
  let hasModel = false
  conditions.forEach((cond, idx) => {
    const res = emitConditionNode(cond, idx, indById, features, emit, nodes)
    if (res.error) { errors.push(res.error); return }
    if (res.hasModel) hasModel = true
  })

  if (spec.exits.length > 0) {
    warnings.push(
      'Exit rules (stop loss / take profit / trailing stop) are not represented in the saved v1.0 format and will not be backtested.',
    )
  }

  const sizeMode =
    spec.size.type === 'percent_of_equity' ? 'percent_of_balance' : 'fixed'
  if (sizeMode === 'percent_of_balance') {
    warnings.push(
      'Percent-of-equity sizing is accepted but currently parse-only in the backtest engine (which simulates fixed sizes).',
    )
  }

  if (errors.length > 0) {
    return { definition: null, errors, warnings }
  }

  const inputs: StrategyDefinition['inputs'] = [
    { lane: 'market.bars.1m', instrument: '$bound_at_init' },
  ]
  if (features.size > 0) {
    inputs.push({
      lane: 'features.technical',
      instrument: '$bound_at_init',
      features: [...features],
    })
  }

  return {
    definition: {
      strategy_id: slugify(spec.name),
      definition_version: hasModel ? '1.1' : '1.0',
      asset_class: assetClass,
      inputs,
      nodes,
      actions: [
        {
          on_signal: emit,
          type: 'place_order',
          order: {
            side: spec.entry.side,
            size_mode: sizeMode,
            size: String(spec.size.value),
          },
        },
      ],
    },
    errors,
    warnings,
  }
}
