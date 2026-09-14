import { useMemo } from 'react'
import { useQuery } from '@tanstack/react-query'
import type { Edge, Node } from '@xyflow/react'
import { ArrowRight, Shield } from 'lucide-react'
import { modelsApi } from '@/api/mlops'
import { familyOf, INDICATOR_LABELS, CONDITION_LABELS, EXIT_LABELS, MARKET_DATA_LABELS } from '@/nodes'
import type { ActionNodeData, AIInferenceNodeData, ConditionNodeData, ExitNodeData, IndicatorNodeData, LogicNodeData, MarketDataNodeData, SizeNodeData } from '@/nodes'
import type { ConditionType, ExitType, IndicatorKind, SizeType } from '@/types/spec'
import { Panel, PanelBody, PanelHeader } from '@/components/primitives/Panel'
import { Badge, Swatch } from '@/components/primitives/Badge'
import { Label, StatTile } from '@/components/primitives/Num'
import { Input, NumberField, Field } from '@/components/primitives/Field'
import { Select } from '@/components/primitives/Overlay'
import { Segmented } from '@/components/primitives/Segmented'
import { EmptyState } from '@/components/primitives/States'
import { Sparkline } from '@/components/charts/Primitives'
import { DASH, pct, signedPct, ratio } from '@/lib/format'
import { TIMEFRAMES } from '@/hooks/useBars'

/* Spec §4.3 — the inspector.

   The inspector replaces in-node editing of long values: nodes show a summary,
   the inspector holds the full form. It also carries a BACKTEST PREVIEW so the
   user never leaves the canvas to judge a change, and a RISK GUARDRAILS block
   showing the account-level limits that will clamp this strategy. */

export function Inspector({
  node,
  nodes,
  edges,
  onPatch,
  className,
}: {
  node: Node | null
  nodes: Node[]
  edges: Edge[]
  onPatch: (id: string, patch: Record<string, unknown>) => void
  className?: string
}) {
  const info = node ? familyOf(node.type ?? 'logic') : null

  const connections = useMemo(() => {
    if (!node) return { inputs: [] as { label: string; via: string }[], outputs: [] as { label: string; via: string }[] }
    const nameOf = (id: string) => {
      const n = nodes.find((x) => x.id === id)
      if (!n) return id
      const f = familyOf(n.type ?? 'logic')
      return `${f.label} — ${describe(n)}`
    }
    return {
      inputs: edges.filter((e) => e.target === node.id).map((e) => ({ label: nameOf(e.source), via: e.targetHandle ?? '' })),
      outputs: edges.filter((e) => e.source === node.id).map((e) => ({ label: nameOf(e.target), via: e.sourceHandle ?? '' })),
    }
  }, [node, nodes, edges])

  return (
    <Panel className={className}>
      <PanelHeader
        title="Inspector"
        actions={
          info ? (
            <Badge tone="neutral">
              <Swatch color={info.token} />
              {info.label}
            </Badge>
          ) : undefined
        }
      />
      <PanelBody flush style={{ overflow: 'auto' }}>
        {!node ? (
          <EmptyState
            message="Nothing selected"
            detail="Click a block on the canvas to see and edit everything it holds."
          />
        ) : (
          <>
            <div className="insp-sec">
              <div className="between" style={{ marginBottom: 'var(--s-3)' }}>
                <span style={{ fontSize: 'var(--t-14)', fontWeight: 'var(--w-semibold)' }}>{describe(node)}</span>
                <span className="lbl mono">{node.id}</span>
              </div>
              <NodeForm node={node} onPatch={onPatch} />
            </div>

            <div className="insp-sec">
              <Label>Connections</Label>
              <div style={{ marginTop: 'var(--s-2)', display: 'flex', flexDirection: 'column', gap: 6 }}>
                {connections.inputs.length === 0 && connections.outputs.length === 0 && (
                  <span className="mut" style={{ fontSize: 'var(--t-12)' }}>
                    Not wired to anything yet.
                  </span>
                )}
                {connections.inputs.map((c, i) => (
                  <div key={`i-${i}`} className="row" style={{ fontSize: 'var(--t-12)' }}>
                    <Badge tone="neutral">in</Badge>
                    <span className="truncate-1">{c.label}</span>
                  </div>
                ))}
                {connections.outputs.map((c, i) => (
                  <div key={`o-${i}`} className="row" style={{ fontSize: 'var(--t-12)' }}>
                    <Badge tone="accent">out</Badge>
                    <span className="truncate-1">{c.label}</span>
                    <ArrowRight size={11} className="mut" aria-hidden />
                  </div>
                ))}
              </div>
            </div>

            <BacktestPreview />
            <RiskGuardrails />
          </>
        )}
      </PanelBody>
    </Panel>
  )
}

function describe(n: Node): string {
  switch (n.type) {
    case 'market_data': {
      const d = n.data as MarketDataNodeData
      return d.symbol || 'Bound at deploy'
    }
    case 'indicator': return (n.data as IndicatorNodeData).indicatorId || 'Unnamed indicator'
    case 'condition': return CONDITION_LABELS[(n.data as ConditionNodeData).conditionType]
    case 'ai_inference': return 'AI forecast'
    case 'logic': return `${(n.data as LogicNodeData).op.toUpperCase()} gate`
    case 'action': return (n.data as ActionNodeData).side === 'buy' ? 'Buy / Long' : 'Sell / Short'
    case 'size': return 'Position size'
    case 'exit': return EXIT_LABELS[(n.data as ExitNodeData).exitType]
    default: return n.type ?? 'Block'
  }
}

/* -----------------------------------------------------------------------------
   Per-family forms
   -------------------------------------------------------------------------- */
function NodeForm({ node, onPatch }: { node: Node; onPatch: (id: string, patch: Record<string, unknown>) => void }) {
  const patch = (p: Record<string, unknown>) => onPatch(node.id, p)

  switch (node.type) {
    case 'market_data': {
      const d = node.data as MarketDataNodeData
      return (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--s-3)' }}>
          <Field label="Source">
            <Select
              ariaLabel="Data source"
              small
              value={d.source}
              onChange={(v) => patch({ source: v })}
              options={(Object.keys(MARKET_DATA_LABELS) as (keyof typeof MARKET_DATA_LABELS)[]).map((k) => ({
                value: k,
                label: MARKET_DATA_LABELS[k],
                text: MARKET_DATA_LABELS[k],
              }))}
            />
          </Field>
          <Input
            label="Instrument"
            hint="Leave blank to bind the instrument when the strategy is deployed."
            placeholder="BTC-USD"
            value={d.symbol}
            onChange={(e) => patch({ symbol: e.target.value.toUpperCase() })}
          />
          <Field label="Timeframe">
            <Select
              ariaLabel="Timeframe"
              small
              value={d.timeframe}
              onChange={(v) => patch({ timeframe: v })}
              options={TIMEFRAMES.map((t) => ({ value: t.label, label: t.label, text: t.label }))}
            />
          </Field>
          {d.source === 'price' && (
            <Field label="Field">
              <Segmented
                ariaLabel="Price field"
                wide
                value={d.field}
                onChange={(v) => patch({ field: v })}
                options={[
                  { value: 'close', label: 'Close' },
                  { value: 'open', label: 'Open' },
                  { value: 'high', label: 'High' },
                  { value: 'low', label: 'Low' },
                ]}
              />
            </Field>
          )}
        </div>
      )
    }

    case 'indicator': {
      const d = node.data as IndicatorNodeData
      return (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--s-3)' }}>
          <Input
            label="Name"
            hint="Other blocks reference this indicator by name."
            value={d.indicatorId}
            spellCheck={false}
            placeholder="ema_fast"
            onChange={(e) => patch({ indicatorId: e.target.value })}
            error={d.indicatorId ? null : 'A name is required.'}
          />
          <Field label="Type">
            <Select
              ariaLabel="Indicator type"
              small
              value={d.kind}
              onChange={(v) => patch({ kind: v as IndicatorKind })}
              options={(Object.keys(INDICATOR_LABELS) as IndicatorKind[]).map((k) => ({
                value: k,
                label: INDICATOR_LABELS[k],
                text: k.toUpperCase(),
              }))}
            />
          </Field>
          <NumberField
            label="Period"
            unit="bars"
            min={1}
            max={500}
            value={String(d.period)}
            onValueChange={(v) => patch({ period: Math.max(1, parseInt(v) || 1) })}
            dp={0}
          />
        </div>
      )
    }

    case 'condition': {
      const d = node.data as ConditionNodeData
      const unary = d.conditionType === 'rising' || d.conditionType === 'falling'
      return (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--s-3)' }}>
          <Field label="Rule">
            <Select
              ariaLabel="Condition"
              small
              value={d.conditionType}
              onChange={(v) => patch({ conditionType: v as ConditionType })}
              options={(Object.keys(CONDITION_LABELS) as ConditionType[]).map((k) => ({
                value: k,
                label: CONDITION_LABELS[k],
                text: CONDITION_LABELS[k],
              }))}
            />
          </Field>
          {unary ? (
            <div className="callout warn">
              <span>
                Rising and falling cannot be saved to the v1.0 definition format yet — it has no access to
                prior bars. Use a comparison against an indicator instead.
              </span>
            </div>
          ) : (
            <>
              <Field label="Compare against">
                <Segmented
                  ariaLabel="Right operand"
                  wide
                  value={d.rightMode}
                  onChange={(v) => patch({ rightMode: v })}
                  options={[
                    { value: 'indicator', label: 'Another block' },
                    { value: 'value', label: 'A fixed value' },
                  ]}
                />
              </Field>
              {d.rightMode === 'value' && (
                <NumberField
                  label="Value"
                  value={String(d.rightValue)}
                  onValueChange={(v) => patch({ rightValue: parseFloat(v) || 0 })}
                />
              )}
            </>
          )}
        </div>
      )
    }

    case 'ai_inference': {
      const d = node.data as AIInferenceNodeData
      return <AiForm data={d} patch={patch} />
    }

    case 'logic': {
      const d = node.data as LogicNodeData
      return (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--s-3)' }}>
          <Field label="Mode">
            <Segmented
              ariaLabel="Gate mode"
              wide
              value={d.op}
              onChange={(v) => patch({ op: v })}
              options={[
                { value: 'and', label: 'AND — all must match' },
                { value: 'or', label: 'OR — any can match' },
              ]}
            />
          </Field>
          <NumberField
            label="Inputs"
            min={2}
            max={6}
            dp={0}
            value={String(d.inputCount)}
            onValueChange={(v) => patch({ inputCount: Math.max(2, Math.min(6, parseInt(v) || 2)) })}
          />
        </div>
      )
    }

    case 'action': {
      const d = node.data as ActionNodeData
      return (
        <Field label="Action">
          <Segmented
            ariaLabel="Order side"
            wide
            variant="buysell"
            value={d.side}
            onChange={(v) => patch({ side: v })}
            options={[
              { value: 'buy', label: 'Buy / Long', side: 'buy' },
              { value: 'sell', label: 'Sell / Short', side: 'sell' },
            ]}
          />
        </Field>
      )
    }

    case 'size': {
      const d = node.data as SizeNodeData
      const isPct = d.sizeType === 'percent_of_equity'
      return (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--s-3)' }}>
          <Field label="Sized by">
            <Segmented
              ariaLabel="Sizing method"
              wide
              value={d.sizeType}
              onChange={(v) => patch({ sizeType: v as SizeType })}
              options={[
                { value: 'percent_of_equity', label: '% of equity' },
                { value: 'fixed_quantity', label: 'Fixed quantity' },
              ]}
            />
          </Field>
          <NumberField
            label={isPct ? 'Percent of equity' : 'Quantity'}
            unit={isPct ? '%' : undefined}
            value={isPct ? String((d.value * 100).toFixed(2)) : String(d.value)}
            onValueChange={(v) => patch({ value: isPct ? (parseFloat(v) || 0) / 100 : parseFloat(v) || 0 })}
            step={isPct ? 0.25 : 1}
            min={0}
          />
        </div>
      )
    }

    case 'exit': {
      const d = node.data as ExitNodeData
      return (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--s-3)' }}>
          <Field label="Type">
            <Select
              ariaLabel="Exit type"
              small
              value={d.exitType}
              onChange={(v) => patch({ exitType: v as ExitType })}
              options={(Object.keys(EXIT_LABELS) as ExitType[]).map((k) => ({
                value: k,
                label: EXIT_LABELS[k],
                text: EXIT_LABELS[k],
              }))}
            />
          </Field>
          <NumberField
            label="Distance"
            unit="%"
            step={0.1}
            min={0.01}
            max={100}
            value={String((d.value * 100).toFixed(2))}
            onValueChange={(v) => patch({ value: (parseFloat(v) || 0) / 100 })}
          />
          <div className="callout warn">
            <span>
              Exit rules are not represented in the saved v1.0 format yet, so this rule is not applied
              during a backtest. It is preserved on the canvas.
            </span>
          </div>
        </div>
      )
    }

    default:
      return null
  }
}

function AiForm({
  data,
  patch,
}: {
  data: AIInferenceNodeData
  patch: (p: Record<string, unknown>) => void
}) {
  const models = useQuery({
    queryKey: ['models', 'for-node', 'forecaster'],
    queryFn: async () => {
      const { data: d } = await modelsApi.forNode('forecaster')
      return d.models ?? []
    },
  })

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--s-3)' }}>
      <Field label="Model">
        <Select
          ariaLabel="Model"
          small
          value={data.targetRef || null}
          placeholder="Choose a forecaster…"
          onChange={(v) => patch({ targetRef: v })}
          options={(models.data ?? []).map((m) => ({
            value: m.id,
            label: `${m.display_name}${m.status !== 'active' || !m.has_production ? ' (unavailable)' : ''}`,
            text: m.display_name,
            disabled: m.status !== 'active' || !m.has_production,
          }))}
        />
      </Field>
      <Field label="Version">
        <Segmented
          ariaLabel="Model alias"
          wide
          value={data.alias}
          onChange={(v) => patch({ alias: v })}
          options={[
            { value: 'production', label: 'Production' },
            { value: 'candidate', label: 'Candidate' },
          ]}
        />
      </Field>
      <Field label="Expects">
        <Segmented
          ariaLabel="Forecast direction"
          wide
          value={data.direction}
          onChange={(v) => patch({ direction: v })}
          options={[
            { value: 'bullish', label: 'Bullish' },
            { value: 'bearish', label: 'Bearish' },
            { value: 'any', label: 'Any' },
          ]}
        />
      </Field>
      <NumberField
        label="Minimum confidence"
        step={0.05}
        min={0}
        max={1}
        dp={2}
        value={String(data.minConfidence)}
        onValueChange={(v) => patch({ minConfidence: parseFloat(v) || 0 })}
        hint="A higher gate filters more signals. 0.62 filtered roughly seven in ten in the last backtest."
      />
      <NumberField
        label="Lookback"
        unit="bars"
        dp={0}
        min={1}
        value={String(data.lookback)}
        onValueChange={(v) => patch({ lookback: parseInt(v) || 1 })}
      />
    </div>
  )
}

/* -----------------------------------------------------------------------------
   Backtest preview + risk guardrails
   -------------------------------------------------------------------------- */
function BacktestPreview() {
  return (
    <div className="insp-sec">
      <div className="between" style={{ marginBottom: 'var(--s-3)' }}>
        <Label>Backtest preview</Label>
        <Badge tone="neutral">not run</Badge>
      </div>
      <div className="mini3">
        <StatTile label="Net P&L" value={<span className="mut">{DASH}</span>} />
        <StatTile label="Win rate" value={<span className="mut">{DASH}</span>} />
        <StatTile label="Trades" value={<span className="mut">{DASH}</span>} />
        <StatTile label="Max DD" value={<span className="mut">{DASH}</span>} />
        <StatTile label="Sharpe" value={<span className="mut">{DASH}</span>} />
        <StatTile label="Exposure" value={<span className="mut">{DASH}</span>} />
      </div>
      <div className="mut" style={{ fontSize: 'var(--t-11)', marginTop: 'var(--s-3)', lineHeight: 1.5 }}>
        Run a backtest from the strategy bar to fill this in without leaving the canvas.
      </div>
    </div>
  )
}

/** Shows results once a preview run exists. Kept beside the empty variant so the
 *  shape is identical and the panel never jumps when data arrives. */
export function BacktestPreviewResult({
  netPct,
  winRate,
  trades,
  maxDd,
  sharpe,
  exposure,
  equity,
  window,
}: {
  netPct: number
  winRate: number
  trades: number
  maxDd: number
  sharpe: number
  exposure: number
  equity: number[]
  window: string
}) {
  return (
    <div className="insp-sec">
      <div className="between" style={{ marginBottom: 'var(--s-3)' }}>
        <Label>Backtest preview</Label>
        <Badge tone="neutral">{window}</Badge>
      </div>
      <div className="mini3">
        <StatTile label="Net P&L" value={<span className={netPct >= 0 ? 'num pos' : 'num neg'}>{signedPct(netPct)}</span>} />
        <StatTile label="Win rate" value={<span className="num">{pct(winRate, 0)}</span>} />
        <StatTile label="Trades" value={<span className="num">{trades}</span>} />
        <StatTile label="Max DD" value={<span className="num neg">{signedPct(-Math.abs(maxDd))}</span>} />
        <StatTile label="Sharpe" value={<span className="num">{ratio(sharpe)}</span>} />
        <StatTile label="Exposure" value={<span className="num">{pct(exposure, 0)}</span>} />
      </div>
      {equity.length > 1 && (
        <div style={{ marginTop: 'var(--s-4)' }}>
          <Sparkline points={equity} width={300} height={64} ariaLabel="Backtest equity curve" />
        </div>
      )}
    </div>
  )
}

function RiskGuardrails() {
  return (
    <div className="insp-sec">
      <div className="row" style={{ marginBottom: 'var(--s-3)', gap: 6 }}>
        <Shield size={12} className="mut" aria-hidden />
        <Label>Risk guardrails</Label>
      </div>
      <div className="kv">
        <span className="k">Max risk per trade</span>
        <span className="v">2.0% of equity</span>
      </div>
      <div className="kv">
        <span className="k">Max concurrent positions</span>
        <span className="v">3</span>
      </div>
      <div className="kv">
        <span className="k">Daily loss circuit breaker</span>
        <span className="v neg">{signedPct(-4)}</span>
      </div>
      <div className="mut" style={{ fontSize: 'var(--t-11)', marginTop: 'var(--s-3)', lineHeight: 1.5 }}>
        Account-level limits clamp this strategy at run time, whatever the blocks say. Change them in
        Settings → Trading &amp; risk.
      </div>
    </div>
  )
}
