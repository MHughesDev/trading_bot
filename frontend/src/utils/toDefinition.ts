// Compiles the visual builder's RuleStrategySpec into the canonical
// StrategyDefinition that the Rust backend (`/api/strategies`), the validator,
// and the backtest engine all consume (ADR-0010, DATA-004).
//
// The expression grammar is deliberately minimal: a condition node holds a
// single comparison (no boolean `&&`/`||`), a signal node watches one condition,
// and signals that share an `emit` name OR-combine.
//
// AI inference is a first-class node (`model_forecast`, canonical v1.1): the
// block runs data through a model, ensemble, or pipeline and gates a signal on
// the resolved forecast. A definition that uses one is emitted at version 1.1;
// otherwise 1.0. Exits still have no representation and are reported as warnings.

import type { Condition, IndicatorSpec, RuleStrategySpec } from '@/types/spec'

/** Canonical `model_forecast` node (mirrors Rust `NodeKind::ModelForecast`). */
export interface ModelForecastNode {
  id: string
  type: 'model_forecast'
  model_ref: string
  target_kind?: 'model' | 'ensemble' | 'pipeline'
  alias?: string
  direction: string
  min_confidence: number
  input?: { feature_set?: string; timeframe: string; lookback: number }
}

export interface StrategyDefinition {
  strategy_id: string
  definition_version: '1.0' | '1.1'
  asset_class: string
  inputs: Array<{ lane: string; instrument: string; features?: string[] }>
  nodes: Array<
    | { id: string; type: 'condition'; expr: string }
    | { id: string; type: 'signal'; when: string; emit: string }
    | ModelForecastNode
  >
  actions: Array<{
    on_signal: string
    type: 'place_order'
    order: { side: string; size_mode: string; size: string }
  }>
}

/** Builds a canonical `model_forecast` node from an AI condition, or an error. */
function aiForecastNode(cond: Condition, id: string): ModelForecastNode | { error: string } {
  const ai = cond.ai
  if (!ai || !ai.targetRef) {
    return { error: 'AI Inference block is missing its target (model / ensemble / pipeline).' }
  }
  const node: ModelForecastNode = {
    id,
    type: 'model_forecast',
    model_ref: ai.targetRef,
    direction: ai.direction,
    min_confidence: ai.minConfidence,
    input: {
      ...(ai.input.featureSet ? { feature_set: ai.input.featureSet } : {}),
      timeframe: ai.input.timeframe,
      lookback: ai.input.lookback,
    },
  }
  // Omit defaults so the JSON round-trips cleanly with the Rust skip_serializing_if.
  if (ai.targetKind !== 'model') node.target_kind = ai.targetKind
  if (ai.alias && ai.alias !== 'production') node.alias = ai.alias
  return node
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
  if (cond.type === 'model_forecast') {
    return {
      error:
        "AI Forecast conditions can't be saved to the canonical strategy format yet.",
    }
  }
  const op = COMPARATOR[cond.type]
  if (!op) return { error: `unsupported condition type '${cond.type}'` }

  const left = operand(cond.left, undefined, indById, features)
  if ('error' in left) return left
  const right = operand(cond.right_id, cond.right_value, indById, features)
  if ('error' in right) return right

  return { expr: `${left.expr} ${op} ${right.expr}` }
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
  let usedAi = false
  conditions.forEach((cond, idx) => {
    const condId = `c${idx + 1}`
    if (cond.type === 'model_forecast') {
      const node = aiForecastNode(cond, condId)
      if ('error' in node) { errors.push(node.error); return }
      nodes.push(node)
      usedAi = true
    } else {
      const compiled = conditionToExpr(cond, indById, features)
      if ('error' in compiled) { errors.push(compiled.error); return }
      nodes.push({ id: condId, type: 'condition', expr: compiled.expr })
    }
    nodes.push({ id: `s${idx + 1}`, type: 'signal', when: condId, emit })
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
      definition_version: usedAi ? '1.1' : '1.0',
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
  let usedAi = false
  conditions.forEach((cond, idx) => {
    const condId = `c${idx + 1}`
    if (cond.type === 'model_forecast') {
      const node = aiForecastNode(cond, condId)
      if ('error' in node) { errors.push(node.error); return }
      nodes.push(node)
      usedAi = true
    } else {
      const compiled = conditionToExpr(cond, indById, features)
      if ('error' in compiled) {
        errors.push(compiled.error)
        return
      }
      nodes.push({ id: condId, type: 'condition', expr: compiled.expr })
    }
    nodes.push({ id: `s${idx + 1}`, type: 'signal', when: condId, emit })
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
      definition_version: usedAi ? '1.1' : '1.0',
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
