// Test lab (N-5.6): run one version against a window and a case set, and show the
// point-in-time (PIT) status of each answer.
//
// What the server serves bounds this: `POST /versions/{v}/test` takes instances
// (an instrument and features), not a historical as-of window. So the "window" is
// the latest bars the server turns into a feature vector
// (`GET /feature-vector?instrument_id&timeframe`), and the page says so. PIT
// status is read from the answer when the server reports it
// (`pit_status`, `point_in_time` or `pit`); otherwise it reads "not reported",
// never "ok".
import { useState } from 'react'
import { useMutation } from '@tanstack/react-query'
import { Button } from '@/components/primitives/Button'
import { Badge } from '@/components/primitives/Badge'
import { Input } from '@/components/primitives/Field'
import { Panel, PanelBody, PanelHeader } from '@/components/primitives/Panel'
import { EmptyState, ErrorState, PanelLoading } from '@/components/primitives/States'
import { modelsApi, type TestCase, type TestInferenceResponse } from '@/api/mlops'
import { useTestCases } from '@/hooks/useMlOps'
import { useSelectedVersion } from '@/lib/models/useSelectedVersion'
import { VersionPicker } from './VersionPicker'

type Instance = { instrument_id?: string; features?: Record<string, number> }

interface Answer {
  label: string
  response: TestInferenceResponse | null
  error: string | null
  featuresFrom: string
}

/** PIT status as the answer reports it; `null` is "not reported". */
function pitOf(r: TestInferenceResponse | null): { tone: 'pos' | 'neg' | 'neutral'; text: string } {
  const raw = r as unknown as Record<string, unknown> | null
  const v = raw?.pit_status ?? raw?.point_in_time ?? raw?.pit
  if (v === true || v === 'ok' || v === 'pass') return { tone: 'pos', text: 'PIT clean' }
  if (v === false || v === 'violation' || v === 'fail') return { tone: 'neg', text: 'PIT violation' }
  if (typeof v === 'string' && v !== '') return { tone: 'neutral', text: `PIT: ${v}` }
  return { tone: 'neutral', text: 'PIT not reported' }
}

function caseInstance(c: TestCase): Instance {
  return { instrument_id: c.input.instrument_id, features: c.input.features }
}

export function TestLab({ modelId }: { modelId: string }) {
  const sel = useSelectedVersion(modelId)
  const cases = useTestCases(modelId)
  const [chosen, setChosen] = useState<string[]>([])
  const [instrument, setInstrument] = useState('')
  const [timeframe, setTimeframe] = useState('1h')
  const [answers, setAnswers] = useState<Answer[] | null>(null)

  const run = useMutation({
    mutationFn: async (version: number) => {
      const out: Answer[] = []
      const jobs: Array<{ label: string; instance: Instance; from: string }> = (cases.data ?? [])
        .filter((c) => chosen.includes(c.case_id))
        .map((c) => ({ label: c.name, instance: caseInstance(c), from: 'saved case' }))
      if (instrument.trim()) {
        const fv = await modelsApi.featureVector(modelId, { instrument_id: instrument.trim(), timeframe })
        jobs.push({
          label: `${instrument.trim()} ${timeframe}`,
          instance: { instrument_id: instrument.trim(), features: fv.data.features },
          from: `${fv.data.source}${fv.data.bars_used ? `, ${fv.data.bars_used} bars` : ''}`,
        })
      }
      for (const j of jobs) {
        try {
          const r = await modelsApi.testVersion(modelId, version, [j.instance])
          out.push({ label: j.label, response: r.data, error: null, featuresFrom: j.from })
        } catch {
          out.push({ label: j.label, response: null, error: 'The version did not answer.', featuresFrom: j.from })
        }
      }
      return out
    },
    onSuccess: (a) => setAnswers(a),
  })

  const canRun = sel.version !== null && (chosen.length > 0 || instrument.trim().length > 0)

  return (
    <div style={{ display: 'grid', gap: 'var(--s-3)', gridTemplateColumns: 'repeat(auto-fit, minmax(340px, 1fr))' }}>
      <Panel>
        <PanelHeader
          title="What to run"
          actions={<VersionPicker versions={sel.versions} aliases={sel.aliases} value={sel.version} onChange={sel.select} />}
        />
        <PanelBody>
          <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--s-3)' }}>
            <div>
              <span className="lbl">Case set</span>
              {cases.isLoading ? (
                <PanelLoading lines={3} />
              ) : cases.isError ? (
                <ErrorState message="Saved cases could not be read." onRetry={() => void cases.refetch()} />
              ) : (cases.data ?? []).length === 0 ? (
                <EmptyState message="No saved cases" detail="Run a window below, or save cases from the classic Test Lab." />
              ) : (
                <ul style={{ listStyle: 'none', margin: 0, padding: 0 }}>
                  {(cases.data ?? []).map((c) => (
                    <li key={c.case_id}>
                      <label style={{ display: 'flex', gap: 'var(--s-2)' }}>
                        <input
                          type="checkbox"
                          checked={chosen.includes(c.case_id)}
                          onChange={(e) =>
                            setChosen((p) => (e.target.checked ? [...p, c.case_id] : p.filter((x) => x !== c.case_id)))
                          }
                        />
                        {c.name}
                      </label>
                    </li>
                  ))}
                </ul>
              )}
            </div>
            <div>
              <span className="lbl">Window: latest bars</span>
              <div style={{ display: 'flex', gap: 'var(--s-2)' }}>
                <Input aria-label="Instrument" placeholder="BTC-USD" value={instrument} onChange={(e) => setInstrument(e.target.value)} />
                <Input aria-label="Timeframe" value={timeframe} onChange={(e) => setTimeframe(e.target.value)} />
              </div>
              <p className="mut" style={{ fontSize: 'var(--t-11)' }}>
                The server builds the feature vector from the most recent stored bars. It serves no historical as-of window.
              </p>
            </div>
            <Button variant="primary" size="sm" disabled={!canRun} loading={run.isPending} onClick={() => sel.version !== null && run.mutate(sel.version)}>
              Run version{sel.version !== null ? ` ${sel.version}` : ''}
            </Button>
          </div>
        </PanelBody>
      </Panel>

      <Panel>
        <PanelHeader title="Answers" />
        <PanelBody>
          {run.isError ? (
            <ErrorState message="The run could not start." detail="The feature vector or the version could not be read." />
          ) : run.isPending ? (
            <PanelLoading lines={4} />
          ) : answers === null ? (
            <EmptyState message="Nothing run yet" detail="Choose cases or a window and run a version." />
          ) : answers.length === 0 ? (
            <EmptyState message="Nothing was run" />
          ) : (
            <table className="tbl" aria-label="Test answers">
              <thead>
                <tr>
                  <th>Input</th>
                  <th>Answer</th>
                  <th>Latency</th>
                  <th>Point in time</th>
                </tr>
              </thead>
              <tbody>
                {answers.map((a, i) => {
                  const pit = pitOf(a.response)
                  const p = a.response?.predictions?.[0]
                  return (
                    <tr key={`${a.label}-${i}`} data-testid="answer-row">
                      <td>
                        {a.label}
                        <div className="mut" style={{ fontSize: 'var(--t-11)' }}>features: {a.featuresFrom}</div>
                      </td>
                      <td>
                        {a.error ? (
                          <span className="neg">{a.error}</span>
                        ) : p ? (
                          `${p.direction} ${p.magnitude} (confidence ${p.confidence}, horizon ${p.horizon})`
                        ) : (
                          'no prediction returned'
                        )}
                      </td>
                      <td className="mono">{a.response ? `${a.response.latency_ms} ms` : 'n/a'}</td>
                      <td>
                        <Badge tone={pit.tone} data-testid="pit-status">{pit.text}</Badge>
                      </td>
                    </tr>
                  )
                })}
              </tbody>
            </table>
          )}
        </PanelBody>
      </Panel>
    </div>
  )
}
