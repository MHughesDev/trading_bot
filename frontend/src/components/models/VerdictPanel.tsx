// The usage-rule verdict and the baselines it is judged against (N-3.2 / N-3.5).
//
// Nothing here hides a result. A rule that fails is shown as failing; a rule that
// could not be computed is shown as BLOCKING, in the same position and weight as
// a failure (ADR-N-05); `not_applicable` is the only quiet state, and it still
// shows its reason. Baselines render with the candidate's folds so a reader sees
// whether the comparison was on identical folds.
import { Badge } from '@/components/primitives/Badge'
import { EmptyState, ErrorState, PanelLoading } from '@/components/primitives/States'
import type { UsageRule, Verdict } from '@/api/modelsStudio'
import {
  RULE_TITLES,
  ruleBlocks,
  ruleLabel,
  ruleTone,
  scoreColumns,
  scoreText,
  verdictHeadline,
} from '@/lib/models/verdict'

function measuredText(m: Record<string, unknown>): string {
  const parts = Object.entries(m)
    .filter(([, v]) => typeof v === 'number' || typeof v === 'string' || typeof v === 'boolean')
    .map(([k, v]) => `${k} ${typeof v === 'number' ? v.toFixed(4) : String(v)}`)
  return parts.length > 0 ? parts.join(' · ') : 'nothing measured'
}

export function RuleRow({ rule }: { rule: UsageRule }) {
  const blocks = ruleBlocks(rule)
  return (
    <li
      data-testid="usage-rule"
      data-rule={rule.rule}
      data-status={rule.status}
      data-blocking={blocks ? 'true' : 'false'}
      style={{
        display: 'grid',
        gridTemplateColumns: 'minmax(0, 1fr) auto',
        gap: 'var(--s-1) var(--s-3)',
        padding: 'var(--s-2) 0',
        borderBottom: '1px solid var(--line-hairline)',
      }}
    >
      <span style={{ fontSize: 'var(--t-13)' }}>{RULE_TITLES[rule.rule]}</span>
      <Badge tone={ruleTone(rule.status)}>{ruleLabel(rule)}</Badge>
      <span className="mut" style={{ fontSize: 'var(--t-11)', gridColumn: '1 / -1' }}>
        {rule.reason}
      </span>
      <span className="mono mut" style={{ fontSize: 'var(--t-11)', gridColumn: '1 / -1' }}>
        {measuredText(rule.measured)}
      </span>
    </li>
  )
}

export function BaselineTable({ verdict }: { verdict: Verdict }) {
  const rows = verdict.baselines
  if (rows.length === 0) {
    return (
      <div className="inline-err" role="alert" data-testid="no-baselines">
        <span className="msg">
          This version has no baseline rows. A scorecard without its baselines is not a comparison, so no result here
          should be read as a win.
        </span>
      </div>
    )
  }
  const cols = scoreColumns(rows)
  return (
    <div style={{ overflowX: 'auto' }}>
      <table className="tbl" aria-label="Baselines on the same folds">
        <thead>
          <tr>
            <th>Baseline</th>
            <th>Folds</th>
            {cols.map((c) => (
              <th key={c} className="n">
                {c}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {rows.map((b) => (
            <tr key={b.name} data-testid="baseline-row">
              <td>
                {b.name}
                <span className="mut"> {b.family}/{b.variant}</span>
                {b.contamination && (
                  <Badge tone="warn" title="The model may have seen this period in pretraining">
                    contaminated
                  </Badge>
                )}
              </td>
              <td className="mono">{b.folds.length === 0 ? 'none reported' : b.folds.join(', ')}</td>
              {cols.map((c) => (
                <td key={c} className="n">
                  {scoreText(b.scores[c])}
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  )
}

export function VerdictHeadline({ verdict }: { verdict: Verdict }) {
  const head = verdictHeadline(verdict)
  return (
    <div style={{ display: 'flex', gap: 'var(--s-2)', alignItems: 'center' }}>
      <Badge tone={head.tone} data-testid="verdict-headline">
        {head.text}
      </Badge>
      <span className="mut" style={{ fontSize: 'var(--t-11)' }}>
        version {verdict.version}
      </span>
    </div>
  )
}

export function ComplexityFlag({ verdict }: { verdict: Verdict }) {
  if (!verdict.complexity?.unjustified) return null
  return (
    <div className="banner warn" role="status" data-testid="complexity-unjustified">
      <span>Complexity not justified: {verdict.complexity.detail}</span>
    </div>
  )
}

export function VerdictBody({ verdict }: { verdict: Verdict }) {
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--s-3)' }} data-testid="verdict">
      <VerdictHeadline verdict={verdict} />
      {verdict.rules.length === 0 ? (
        <p className="mut">The server returned no usage rules for this version.</p>
      ) : (
        <ul style={{ listStyle: 'none', margin: 0, padding: 0 }}>
          {verdict.rules.map((r) => (
            <RuleRow key={r.rule} rule={r} />
          ))}
        </ul>
      )}
      <ComplexityFlag verdict={verdict} />
      <BaselineTable verdict={verdict} />
    </div>
  )
}

/** Loading, error, empty (no version) and populated, for one version's verdict. */
export function VerdictPanelBody({
  version,
  verdict,
  loading,
  error,
  onRetry,
}: {
  version: number | null
  verdict: Verdict | undefined
  loading: boolean
  error: boolean
  onRetry: () => void
}) {
  if (version === null) {
    return <EmptyState message="No version to judge" detail="Train a version; its verdict and baselines appear here." />
  }
  if (loading) return <PanelLoading lines={5} />
  if (error || !verdict) {
    return (
      <ErrorState
        message="The verdict could not be read."
        detail="Until it can be, treat this version as not cleared to size a position."
        onRetry={onRetry}
      />
    )
  }
  return <VerdictBody verdict={verdict} />
}
