// Evaluations arena (N-5.7): the candidate against its baselines on identical
// folds, with the usage-rule verdicts rendered and never hidden. A rule that is
// `not_computable` blocks, and is drawn as blocking.
import { Button } from '@/components/primitives/Button'
import { Badge } from '@/components/primitives/Badge'
import { Panel, PanelBody, PanelHeader } from '@/components/primitives/Panel'
import { EmptyState, ErrorState, PanelLoading } from '@/components/primitives/States'
import { useEvaluateVersion, useModelEvals } from '@/hooks/useMlOps'
import { useVerdict } from '@/lib/models/queries'
import { useSelectedVersion } from '@/lib/models/useSelectedVersion'
import { numericMetrics } from '@/components/mlops/studio.helpers'
import { scoreColumns, scoreText } from '@/lib/models/verdict'
import type { Verdict } from '@/api/modelsStudio'
import { ComplexityFlag, RuleRow, VerdictHeadline } from './VerdictPanel'
import { VersionPicker } from './VersionPicker'

/** True when every baseline names the same folds: the comparison is on identical folds. */
function foldsIdentical(v: Verdict): boolean {
  const sets = v.baselines.map((b) => [...b.folds].sort((a, c) => a - c).join(','))
  return sets.every((s) => s === sets[0])
}

export function EvaluationsArena({ modelId }: { modelId: string }) {
  const sel = useSelectedVersion(modelId)
  const verdict = useVerdict(modelId, sel.version)
  const evals = useModelEvals(modelId)
  const evaluate = useEvaluateVersion(modelId)

  const candidate = sel.versions.find((v) => v.version === sel.version)
  const candidateScores = new Map(numericMetrics(candidate?.metrics))

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--s-3)' }}>
      <Panel>
        <PanelHeader
          title="Candidate against baselines"
          actions={
            <>
              <VersionPicker versions={sel.versions} aliases={sel.aliases} value={sel.version} onChange={sel.select} />
              <Button
                size="xs"
                disabled={sel.version === null}
                loading={evaluate.isPending}
                onClick={() => sel.version !== null && evaluate.mutate(sel.version)}
              >
                Run evaluation
              </Button>
            </>
          }
        />
        <PanelBody>
          {sel.query.isLoading || (sel.version !== null && verdict.isLoading) ? (
            <PanelLoading lines={6} />
          ) : sel.version === null ? (
            <EmptyState message="Nothing to evaluate" detail="Train a version first; the arena compares it with its baselines." />
          ) : verdict.isError || !verdict.data ? (
            <ErrorState
              message="The verdict could not be read."
              detail="Without it no rule can be shown, so this version is not cleared."
              onRetry={() => void verdict.refetch()}
            />
          ) : (
            <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--s-3)' }}>
              <VerdictHeadline verdict={verdict.data} />
              <ComplexityFlag verdict={verdict.data} />
              {verdict.data.baselines.length === 0 && (
                <div className="inline-err" role="alert" data-testid="no-baselines">
                  <span className="msg">This version has no baseline rows, so nothing here is a comparison.</span>
                </div>
              )}
              {verdict.data.baselines.length > 0 && !foldsIdentical(verdict.data) && (
                <div className="banner warn" role="status" data-testid="folds-differ">
                  The baselines were not scored on identical folds. Treat the comparison below as unreliable.
                </div>
              )}
              {verdict.data.baselines.length > 0 && (
                <div style={{ overflowX: 'auto' }}>
                  <table className="tbl" aria-label="Candidate and baselines">
                    <thead>
                      <tr>
                        <th>Model</th>
                        {scoreColumns(verdict.data.baselines).map((c) => (
                          <th key={c} className="n">
                            {c}
                          </th>
                        ))}
                      </tr>
                    </thead>
                    <tbody>
                      <tr data-testid="candidate-row">
                        <td>
                          <strong>Candidate v{sel.version}</strong>
                        </td>
                        {scoreColumns(verdict.data.baselines).map((c) => (
                          <td key={c} className="n">
                            {candidateScores.has(c) ? scoreText(candidateScores.get(c)) : 'not reported'}
                          </td>
                        ))}
                      </tr>
                      {verdict.data.baselines.map((b) => (
                        <tr key={b.name}>
                          <td>
                            {b.name}
                            <span className="mut"> folds {b.folds.length === 0 ? 'none' : b.folds.join(',')}</span>
                          </td>
                          {scoreColumns(verdict.data!.baselines).map((c) => (
                            <td key={c} className="n">
                              {scoreText(b.scores[c])}
                            </td>
                          ))}
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              )}
              <ul style={{ listStyle: 'none', margin: 0, padding: 0 }} aria-label="Usage rules">
                {verdict.data.rules.map((r) => (
                  <RuleRow key={r.rule} rule={r} />
                ))}
              </ul>
            </div>
          )}
        </PanelBody>
      </Panel>

      <Panel>
        <PanelHeader title="Evaluation runs" />
        <PanelBody flush={(evals.data ?? []).length > 0}>
          {evals.isLoading ? (
            <PanelLoading lines={3} />
          ) : evals.isError ? (
            <ErrorState message="Evaluation runs could not be read." onRetry={() => void evals.refetch()} />
          ) : (evals.data ?? []).length === 0 ? (
            <EmptyState message="No evaluation runs" detail="Run an evaluation to produce a scorecard." />
          ) : (
            <table className="tbl dense" aria-label="Evaluation runs">
              <tbody>
                {(evals.data ?? []).map((e) => (
                  <tr key={e.run_id}>
                    <td className="mono">{e.run_id.slice(0, 8)}</td>
                    <td>
                      <Badge tone={e.status === 'succeeded' ? 'pos' : e.status === 'failed' ? 'neg' : 'neutral'}>{e.status}</Badge>
                    </td>
                    <td className="mut">{e.created_at.slice(0, 16).replace('T', ' ')}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </PanelBody>
      </Panel>
    </div>
  )
}
