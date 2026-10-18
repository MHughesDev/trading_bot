// Create wizard (N-5.3) -- /mlops/studio/new
//
// Declares a `DatasetDeclaration` step by step and submits training through
// `POST /api/models/{id}/train` with `declaration` + `family`.
//
// Purge and embargo are SHOWN, never asked for. The review step asks the server
// (`POST /api/models/declarations/validate`) and renders what it derived; the
// form has no field for either, and `buildDeclaration` cannot emit one.
// Submission is refused until the server has accepted the declaration AND said
// it lowers to the trainer (`lowering.ok`): a declaration the trainer cannot run
// is not a thing to submit.
import { useEffect, useMemo, useState } from 'react'
import { useNavigate } from 'react-router-dom'
import { Button } from '@/components/primitives/Button'
import { Badge, KeyValue } from '@/components/primitives/Badge'
import { Field, Input } from '@/components/primitives/Field'
import { Panel, PanelBody, PanelFooter, PanelHeader } from '@/components/primitives/Panel'
import { EmptyState, ErrorState, PanelLoading } from '@/components/primitives/States'
import { useCreateModel } from '@/hooks/useMlOps'
import { useFamilies, useValidateDeclaration } from '@/lib/models/queries'
import {
  EMPTY_FORM,
  STEPS,
  TIMEFRAMES,
  buildDeclaration,
  stepOfField,
  type DeclarationForm,
  type StepId,
} from '@/lib/models/declaration'
import { studioApi, type ModelFamily, type ValidateResult } from '@/api/modelsStudio'

const SELECT_STYLE: React.CSSProperties = {
  height: 32,
  background: 'var(--bg-input, var(--bg-sunken))',
  color: 'var(--fg-primary)',
  border: '1px solid var(--line-hairline)',
  borderRadius: 'var(--r-sm)',
  padding: '0 var(--s-2)',
}

function familyKey(f: { family: string; variant: string }): string {
  return `${f.family}/${f.variant}`
}

function Sel<T extends string>({
  label,
  value,
  onChange,
  options,
  hint,
}: {
  label: string
  value: T
  onChange: (v: T) => void
  options: Array<{ value: T; label: string }>
  hint?: string
}) {
  return (
    <Field label={label} hint={hint}>
      <select aria-label={label} style={SELECT_STYLE} value={value} onChange={(e) => onChange(e.target.value as T)}>
        {options.map((o) => (
          <option key={o.value} value={o.value}>
            {o.label}
          </option>
        ))}
      </select>
    </Field>
  )
}

// ── steps ──

function FamilyStep({
  selected,
  onSelect,
}: {
  selected: string
  onSelect: (f: ModelFamily) => void
}) {
  const families = useFamilies()
  if (families.isLoading) return <PanelLoading lines={5} />
  if (families.isError) {
    return <ErrorState message="The model families could not be read." onRetry={() => void families.refetch()} />
  }
  const all = families.data ?? []
  if (all.length === 0) {
    return <EmptyState message="No model families registered" detail="The server returned an empty registry." />
  }
  const candidates = all.filter((f) => f.selectable_as_candidate)
  const baselines = all.filter((f) => !f.selectable_as_candidate)
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--s-4)' }}>
      <div role="radiogroup" aria-label="Candidate family" style={{ display: 'flex', flexDirection: 'column', gap: 'var(--s-2)' }}>
        {candidates.length === 0 && <p className="mut">No family can be chosen as a candidate.</p>}
        {candidates.map((f) => (
          <label
            key={familyKey(f)}
            data-testid="family-option"
            style={{
              display: 'flex',
              gap: 'var(--s-3)',
              padding: 'var(--s-3)',
              border: `1px solid ${selected === familyKey(f) ? 'var(--bg-accent)' : 'var(--line-hairline)'}`,
              borderRadius: 'var(--r-md)',
              cursor: 'pointer',
            }}
          >
            <input
              type="radio"
              name="family"
              checked={selected === familyKey(f)}
              onChange={() => onSelect(f)}
              aria-label={familyKey(f)}
            />
            <span style={{ flex: 1 }}>
              <strong>{familyKey(f)}</strong>
              <span className="mut"> {f.framework ?? 'no framework'}</span>
              <div className="mut" style={{ fontSize: 'var(--t-11)' }}>
                {f.description}
              </div>
              <div style={{ display: 'flex', gap: 'var(--s-1)', flexWrap: 'wrap', marginTop: 4 }}>
                {f.uses_gpu && <Badge tone="info">GPU</Badge>}
                {f.needs_sequence && <Badge tone="info">sequence window</Badge>}
                {f.requires.map((r) => (
                  <Badge key={r} tone="outline">
                    needs {r}
                  </Badge>
                ))}
              </div>
            </span>
          </label>
        ))}
      </div>
      {baselines.length > 0 && (
        <div>
          <span className="lbl">Always trained beside your candidate</span>
          <ul className="mut" style={{ margin: 'var(--s-1) 0 0', paddingLeft: 'var(--s-4)', fontSize: 'var(--t-11)' }} data-testid="baseline-families">
            {baselines.map((b) => (
              <li key={familyKey(b)}>
                {familyKey(b)}: {b.description}
                {b.contamination && ` (model cutoff ${b.contamination.model_cutoff})`}
              </li>
            ))}
          </ul>
        </div>
      )}
    </div>
  )
}

function useSet(form: DeclarationForm, setForm: (f: DeclarationForm) => void) {
  return <K extends keyof DeclarationForm>(k: K) => (v: DeclarationForm[K]) => setForm({ ...form, [k]: v })
}

function UniverseStep({ form, setForm }: { form: DeclarationForm; setForm: (f: DeclarationForm) => void }) {
  const set = useSet(form, setForm)
  return (
    <div style={{ display: 'grid', gap: 'var(--s-3)', gridTemplateColumns: 'repeat(auto-fit, minmax(220px, 1fr))' }}>
      <Input label="Instruments" value={form.instruments} onChange={(e) => set('instruments')(e.target.value)} placeholder="BTC-USD, ETH-USD" hint="Separated by spaces or commas" />
      <Sel label="Timeframe" value={form.timeframe} onChange={set('timeframe')} options={TIMEFRAMES.map((t) => ({ value: t, label: t }))} />
      <Input label="Window start" type="date" value={form.start} onChange={(e) => set('start')(e.target.value)} />
      <Input label="Window end" type="date" value={form.end} onChange={(e) => set('end')(e.target.value)} hint="Clipped to the project cutoff if later; the review says so" />
    </div>
  )
}

function FeaturesStep({ form, setForm, needsSequence }: { form: DeclarationForm; setForm: (f: DeclarationForm) => void; needsSequence: boolean }) {
  const set = useSet(form, setForm)
  return (
    <div style={{ display: 'grid', gap: 'var(--s-3)', gridTemplateColumns: 'repeat(auto-fit, minmax(240px, 1fr))' }}>
      <Field label="Features" hint="One expression per line, for example ema(close,21)">
        <textarea
          aria-label="Features"
          rows={5}
          value={form.features}
          onChange={(e) => set('features')(e.target.value)}
          style={{ ...SELECT_STYLE, height: 'auto', padding: 'var(--s-2)', fontFamily: 'var(--font-mono)' }}
        />
      </Field>
      <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--s-3)' }}>
        <Input label="Lags (bars)" value={form.lags} onChange={(e) => set('lags')(e.target.value)} placeholder="1 2 3" hint="Positive and strictly increasing" />
        <Input label="Winsorise tail" value={form.winsorize} onChange={(e) => set('winsorize')(e.target.value)} placeholder="0.01" hint="A fraction in (0, 0.5); blank for none" />
        <Sel
          label="Scaler"
          value={form.scaler}
          onChange={set('scaler')}
          options={[
            { value: 'none', label: 'None' },
            { value: 'per_fold_standard', label: 'Standard, fitted per fold' },
            { value: 'per_fold_robust', label: 'Robust, fitted per fold' },
          ]}
        />
        {needsSequence && (
          <>
            <Input label="Sequence window (bars)" value={form.seqWindow} onChange={(e) => set('seqWindow')(e.target.value)} />
            <Input label="Sequence stride" value={form.seqStride} onChange={(e) => set('seqStride')(e.target.value)} />
          </>
        )}
      </div>
    </div>
  )
}

function TargetStep({ form, setForm }: { form: DeclarationForm; setForm: (f: DeclarationForm) => void }) {
  const set = useSet(form, setForm)
  const barrier = form.targetKind === 'triple_barrier' || form.targetKind === 'meta_label'
  return (
    <div style={{ display: 'grid', gap: 'var(--s-3)', gridTemplateColumns: 'repeat(auto-fit, minmax(220px, 1fr))' }}>
      <Sel
        label="Target"
        value={form.targetKind}
        onChange={set('targetKind')}
        options={[
          { value: 'forward_return', label: 'Forward return' },
          { value: 'realized_vol', label: 'Realised volatility' },
          { value: 'direction', label: 'Direction' },
          { value: 'rank', label: 'Rank' },
          { value: 'triple_barrier', label: 'Triple barrier' },
          { value: 'meta_label', label: 'Meta-label' },
        ]}
      />
      {!barrier && <Input label="Horizon (bars)" value={form.horizon} onChange={(e) => set('horizon')(e.target.value)} hint="The label's length; the purge is derived from it" />}
      {barrier && (
        <>
          <Input label="Profit-take (sigma)" value={form.ptSigma} onChange={(e) => set('ptSigma')(e.target.value)} />
          <Input label="Stop (sigma)" value={form.slSigma} onChange={(e) => set('slSigma')(e.target.value)} />
          <Input label="Max bars" value={form.maxBars} onChange={(e) => set('maxBars')(e.target.value)} />
        </>
      )}
      {form.targetKind === 'meta_label' && (
        <Input label="Primary strategy version" value={form.metaStrategy} onChange={(e) => set('metaStrategy')(e.target.value)} />
      )}
      <Sel
        label="Sampling"
        value={form.samplingKind}
        onChange={set('samplingKind')}
        options={[
          { value: 'every_bar', label: 'Every bar' },
          { value: 'cusum', label: 'CUSUM filter' },
          { value: 'strategy_events', label: 'Strategy events' },
        ]}
      />
      {form.samplingKind === 'cusum' && <Input label="CUSUM threshold (sigma)" value={form.hSigma} onChange={(e) => set('hSigma')(e.target.value)} />}
      {form.samplingKind === 'strategy_events' && (
        <Input label="Strategy version" value={form.samplingStrategy} onChange={(e) => set('samplingStrategy')(e.target.value)} />
      )}
      <Sel
        label="Sample weights"
        value={form.weights}
        onChange={set('weights')}
        hint="Required. There is no default weighting."
        options={[
          { value: '', label: 'Choose...' },
          { value: 'uniqueness', label: 'Uniqueness' },
          { value: 'time_decay', label: 'Time decay' },
          { value: 'none', label: 'None (explicitly)' },
        ]}
      />
      {form.weights === 'time_decay' && <Input label="Half-life (days)" value={form.halfLife} onChange={(e) => set('halfLife')(e.target.value)} />}
    </div>
  )
}

function CvStep({ form, setForm }: { form: DeclarationForm; setForm: (f: DeclarationForm) => void }) {
  const set = useSet(form, setForm)
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--s-3)' }}>
      <div style={{ display: 'grid', gap: 'var(--s-3)', gridTemplateColumns: 'repeat(auto-fit, minmax(200px, 1fr))' }}>
        <Sel
          label="Scheme"
          value={form.cvKind}
          onChange={set('cvKind')}
          options={[
            { value: 'walk_forward', label: 'Walk-forward' },
            { value: 'purged_kfold', label: 'Purged k-fold' },
            { value: 'cpcv', label: 'Combinatorial purged CV' },
          ]}
        />
        {form.cvKind === 'walk_forward' && (
          <>
            <Input label="Train days" value={form.trainDays} onChange={(e) => set('trainDays')(e.target.value)} />
            <Input label="Test days" value={form.testDays} onChange={(e) => set('testDays')(e.target.value)} />
            <Input label="Refit days" value={form.refitDays} onChange={(e) => set('refitDays')(e.target.value)} />
            <Sel
              label="Mode"
              value={form.walkMode}
              onChange={set('walkMode')}
              options={[
                { value: 'expanding', label: 'Expanding' },
                { value: 'rolling', label: 'Rolling' },
              ]}
            />
          </>
        )}
        {form.cvKind === 'purged_kfold' && <Input label="k" value={form.k} onChange={(e) => set('k')(e.target.value)} />}
        {form.cvKind === 'cpcv' && (
          <>
            <Input label="n groups" value={form.n} onChange={(e) => set('n')(e.target.value)} />
            <Input label="k test groups" value={form.k} onChange={(e) => set('k')(e.target.value)} />
          </>
        )}
      </div>
      <p className="mut" style={{ fontSize: 'var(--t-11)' }} data-testid="derived-note">
        Purge and embargo are not set here. They are derived from the label horizon and the feature look-back, and
        shown on the review step.
      </p>
    </div>
  )
}

function ReviewStep({
  result,
  pending,
  failed,
  problems,
  name,
  setName,
  assetClass,
  setAssetClass,
  onGoTo,
}: {
  result: ValidateResult | undefined
  pending: boolean
  failed: boolean
  problems: Array<{ field: string; message: string }>
  name: string
  setName: (s: string) => void
  assetClass: string
  setAssetClass: (s: string) => void
  onGoTo: (s: StepId) => void
}) {
  const body = (() => {
    if (problems.length > 0) {
      return (
        <div className="inline-err" role="alert" data-testid="form-problems">
          <span className="msg">The declaration is not complete yet.</span>
          <ul>
            {problems.map((p) => (
              <li key={p.field + p.message}>
                <button type="button" className="btn ghost xs" onClick={() => onGoTo(stepOfField(p.field))}>
                  {p.field}
                </button>{' '}
                {p.message}
              </li>
            ))}
          </ul>
        </div>
      )
    }
    if (pending) return <PanelLoading lines={4} />
    if (failed || !result) {
      return <ErrorState message="The declaration could not be validated." detail="The server did not answer; nothing was submitted." />
    }
    if (!result.ok) {
      return (
        <div className="inline-err" role="alert" data-testid="server-refusal">
          <span className="msg">The server refused this declaration.</span>
          <ul>
            {result.errors.map((e) => (
              <li key={e.field + e.message}>
                <button type="button" className="btn ghost xs" onClick={() => onGoTo(stepOfField(e.field))}>
                  {e.field}
                </button>{' '}
                {e.message}
              </li>
            ))}
          </ul>
        </div>
      )
    }
    const d = result.derived
    return (
      <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--s-3)' }} data-testid="derived">
        <div className="well" style={{ padding: 'var(--s-3)' }}>
          <span className="lbl">Derived by the server, not editable</span>
          <KeyValue k="Purge" v={<span data-testid="derived-purge">{d.purge_bars} bars</span>} />
          <KeyValue k="Embargo" v={<span data-testid="derived-embargo">{d.embargo_bars} bars</span>} />
          <KeyValue k="Label horizon" v={`${d.horizon_bars} bars`} />
          <KeyValue k="Feature look-back" v={`${d.lookback_bars} bars`} />
          <KeyValue k="Sample estimate" v={d.sample_estimate === undefined ? 'not reported' : `${d.sample_estimate} rows`} />
          <KeyValue k="Declaration hash" v={<span className="mono">{result.declaration_hash.slice(0, 16)}</span>} />
        </div>
        {result.clip && (
          <div className="banner warn" role="status" data-testid="clip-notice">
            The window end {result.clip.requested_end} is after the project cutoff and was clipped to {result.clip.clipped_end}.
          </div>
        )}
        {!result.lowering.ok && (
          <div className="inline-err" role="alert" data-testid="lowering-refused">
            <span className="msg">The trainer cannot run this declaration yet. Submission is disabled.</span>
            <ul>
              {result.lowering.unsupported.map((u) => (
                <li key={u.field}>
                  <span className="mono">{u.field}</span>: {u.why}
                </li>
              ))}
            </ul>
          </div>
        )}
      </div>
    )
  })()
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--s-4)' }}>
      {body}
      <div style={{ display: 'grid', gap: 'var(--s-3)', gridTemplateColumns: 'repeat(auto-fit, minmax(220px, 1fr))' }}>
        <Input label="Model name" value={name} onChange={(e) => setName(e.target.value)} placeholder="BTC hourly forecaster" />
        <Input label="Asset class" value={assetClass} onChange={(e) => setAssetClass(e.target.value)} />
      </div>
    </div>
  )
}

// ── page ──

export function ModelCreateWizardPage() {
  const navigate = useNavigate()
  const families = useFamilies()
  const validate = useValidateDeclaration()
  const createModel = useCreateModel()
  const [stepIdx, setStepIdx] = useState(0)
  const [form, setForm] = useState<DeclarationForm>(EMPTY_FORM)
  const [familySel, setFamilySel] = useState<string>('')
  const [name, setName] = useState('')
  const [assetClass, setAssetClass] = useState('crypto')
  const [submitError, setSubmitError] = useState<string | null>(null)

  const family = useMemo(
    () => (families.data ?? []).find((f) => familyKey(f) === familySel) ?? null,
    [families.data, familySel],
  )
  const step = STEPS[stepIdx]
  const built = useMemo(() => buildDeclaration(form, { needsSequence: family?.needs_sequence ?? false }), [form, family])
  const declJson = built.declaration ? JSON.stringify(built.declaration) : ''

  // Ask the server what it derives whenever the review step is open on a
  // complete declaration. The mutation is a read: it changes nothing.
  const { mutate: runValidate, reset: resetValidate } = validate
  useEffect(() => {
    if (step.id !== 'review') return
    if (!declJson) {
      resetValidate()
      return
    }
    runValidate({ declaration: JSON.parse(declJson) })
  }, [step.id, declJson, runValidate, resetValidate])

  const result = validate.data
  const accepted = step.id === 'review' && result?.ok === true && result.lowering.ok
  const canSubmit = accepted && !!family && name.trim().length > 0

  const [busy, setBusy] = useState(false)
  async function submit() {
    if (!built.declaration || !family) return
    setBusy(true)
    setSubmitError(null)
    try {
      const created = await createModel.mutateAsync({
        display_name: name.trim(),
        definition: {
          schema_version: '1.0',
          model_kind: 'forecaster',
          framework: family.framework ?? 'sklearn',
          runtime: 'python',
          asset_class: assetClass.trim() || 'crypto',
          auto_retrain: false,
          target: { field: 'return', horizon: '1h', transform: 'logret' },
        },
      })
      await studioApi.train(created.data.model_id, {
        declaration: built.declaration,
        family: { family: family.family, variant: family.variant },
      })
      navigate(`/mlops/studio/${created.data.model_id}/training`)
    } catch {
      setSubmitError('The model could not be created and trained. Nothing was started; check the server and try again.')
    } finally {
      setBusy(false)
    }
  }

  const canNext = step.id === 'family' ? family !== null : true

  return (
    <>
      <div className="pagehead">
        <h1 className="h1">New model</h1>
        <span className="lbl">Declare the dataset; the server derives the purge and embargo</span>
        <div className="spacer" />
        <Button size="sm" onClick={() => navigate('/mlops/studio')}>
          Cancel
        </Button>
      </div>
      <div className="page-body" style={{ maxWidth: 880 }}>
        <ol
          aria-label="Wizard steps"
          style={{ display: 'flex', gap: 'var(--s-2)', listStyle: 'none', padding: 0, margin: '0 0 var(--s-3)', flexWrap: 'wrap' }}
        >
          {STEPS.map((s, i) => (
            <li key={s.id}>
              <Badge tone={i === stepIdx ? 'accent' : 'neutral'} aria-current={i === stepIdx ? 'step' : undefined}>
                {i + 1}. {s.label}
              </Badge>
            </li>
          ))}
        </ol>
        <Panel>
          <PanelHeader title={step.label} />
          <PanelBody>
            {step.id === 'family' && (
              <FamilyStep
                selected={familySel}
                onSelect={(f) => setFamilySel(familyKey(f))}
              />
            )}
            {step.id === 'universe' && <UniverseStep form={form} setForm={setForm} />}
            {step.id === 'features' && <FeaturesStep form={form} setForm={setForm} needsSequence={family?.needs_sequence ?? false} />}
            {step.id === 'target' && <TargetStep form={form} setForm={setForm} />}
            {step.id === 'cv' && <CvStep form={form} setForm={setForm} />}
            {step.id === 'review' && (
              <ReviewStep
                result={result}
                pending={validate.isPending || (validate.isIdle && built.problems.length === 0)}
                failed={validate.isError}
                problems={built.problems}
                name={name}
                setName={setName}
                assetClass={assetClass}
                setAssetClass={setAssetClass}
                onGoTo={(id) => setStepIdx(STEPS.findIndex((s) => s.id === id))}
              />
            )}
            {submitError && (
              <div className="inline-err" role="alert" style={{ marginTop: 'var(--s-3)' }}>
                <span className="msg">{submitError}</span>
              </div>
            )}
          </PanelBody>
          <PanelFooter>
            <Button size="sm" disabled={stepIdx === 0} onClick={() => setStepIdx(stepIdx - 1)}>
              Back
            </Button>
            <div className="spacer" />
            {step.id !== 'review' ? (
              <Button size="sm" variant="primary" disabled={!canNext} onClick={() => setStepIdx(stepIdx + 1)}>
                Next
              </Button>
            ) : (
              <Button size="sm" variant="primary" disabled={!canSubmit} loading={busy} onClick={() => void submit()}>
                Create and train
              </Button>
            )}
          </PanelFooter>
        </Panel>
      </div>
    </>
  )
}
