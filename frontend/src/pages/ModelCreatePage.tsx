import { useState } from 'react'
import { useNavigate } from 'react-router-dom'
import { motion, AnimatePresence, useReducedMotion } from 'framer-motion'
import {
  Brain,
  BarChart2,
  Zap,
  Shield,
  Code2,
  Bot,
  ChevronRight,
  ChevronLeft,
  Check,
  Loader2,
  Trees,
  Network,
  GitBranch,
  Boxes,
  Cpu,
  Server,
} from 'lucide-react'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import { useCreateModel } from '@/hooks/useMlOps'
import type { ModelKind } from '@/api/mlops'

const SPRING = { type: 'spring' as const, stiffness: 380, damping: 30 }

interface KindOption {
  kind: ModelKind
  label: string
  icon: React.ElementType
  description: string
}

const KIND_OPTIONS: KindOption[] = [
  {
    kind: 'forecaster',
    label: 'Forecaster',
    icon: Brain,
    description: 'Predicts future price direction or magnitude for a given instrument.',
  },
  {
    kind: 'signal_ranker',
    label: 'Signal Ranker',
    icon: BarChart2,
    description: 'Ranks multiple signals by expected return to filter noise.',
  },
  {
    kind: 'trade_decision',
    label: 'Trade Decision',
    icon: Zap,
    description: 'Makes buy/sell/hold decisions given market context.',
  },
  {
    kind: 'risk_sizing',
    label: 'Risk Sizing',
    icon: Shield,
    description: 'Determines optimal position size based on risk parameters.',
  },
  {
    kind: 'embedding',
    label: 'Embedding',
    icon: Code2,
    description: 'Encodes market data into dense feature vectors for downstream models.',
  },
  {
    kind: 'external_llm_adapter',
    label: 'LLM Adapter',
    icon: Bot,
    description: 'Wraps an external LLM endpoint for use in strategy nodes.',
  },
]

type Framework = 'xgboost' | 'lightgbm' | 'sklearn' | 'torch' | 'external_api'
type Runtime = 'python' | 'rust'

interface FrameworkOption {
  framework: Framework
  label: string
  icon: React.ElementType
  description: string
}

// Trainable frameworks the trainer actually routes to (see worker.py `_route`).
const FRAMEWORK_OPTIONS: FrameworkOption[] = [
  {
    framework: 'xgboost',
    label: 'XGBoost',
    icon: Trees,
    description: 'Gradient-boosted trees. Strong default for tabular market features.',
  },
  {
    framework: 'lightgbm',
    label: 'LightGBM',
    icon: GitBranch,
    description: 'Fast histogram-based boosting. Great on large bar histories.',
  },
  {
    framework: 'sklearn',
    label: 'scikit-learn',
    icon: Boxes,
    description: 'Classic linear / ensemble estimators. Simple and interpretable.',
  },
  {
    framework: 'torch',
    label: 'PyTorch',
    icon: Network,
    description: 'Neural nets for sequence / deep forecasting. Most flexible, slowest.',
  },
]

// Frameworks valid for a given model kind (mirrors domain is_compatible).
function frameworksForKind(kind: ModelKind | null): FrameworkOption[] {
  if (kind === 'external_llm_adapter' || kind === 'embedding') return []
  return FRAMEWORK_OPTIONS
}

interface RuntimeOption {
  runtime: Runtime
  label: string
  icon: React.ElementType
  description: string
  comingSoon?: boolean
}

const RUNTIME_OPTIONS: RuntimeOption[] = [
  {
    runtime: 'python',
    label: 'Python sidecar',
    icon: Cpu,
    description: 'Trains in the Python trainer service. Full library support (default).',
  },
  {
    runtime: 'rust',
    label: 'Rust (in-process)',
    icon: Server,
    description:
      'Native in-process training. Coming soon — the trainer still routes through the Python runtime.',
    comingSoon: true,
  },
]

const ASSET_CLASSES = [
  'crypto_spot_cex',
  'crypto_spot_dex',
  'crypto_perp',
  'equity',
  'forex',
  'commodity',
  'fixed_income',
]

function slugify(text: string): string {
  return text
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-|-$/g, '')
    .slice(0, 64)
}

function StepIndicator({ step, total }: { step: number; total: number }) {
  return (
    <div className="flex items-center gap-2">
      {Array.from({ length: total }).map((_, i) => (
        <div key={i} className="flex items-center gap-2">
          <div
            className={cn(
              'h-2 rounded-full transition-all duration-300',
              i < step
                ? 'w-6 bg-accent'
                : i === step
                  ? 'w-4 bg-accent/60'
                  : 'w-2 bg-border',
            )}
          />
        </div>
      ))}
    </div>
  )
}

const STEP_LABELS = ['Purpose', 'Engine', 'Data', 'Identity']

export function ModelCreatePage() {
  const navigate = useNavigate()
  const shouldReduce = useReducedMotion()
  const createMutation = useCreateModel()

  const [step, setStep] = useState(0)
  const [selectedKind, setSelectedKind] = useState<ModelKind | null>(null)
  const [framework, setFramework] = useState<Framework>('xgboost')
  const [runtime, setRuntime] = useState<Runtime>('python')
  const [assetClass, setAssetClass] = useState('crypto_spot_cex')
  const [autoRetrain, setAutoRetrain] = useState(false)
  const [versionNote, setVersionNote] = useState('')
  const [displayName, setDisplayName] = useState('')
  const [description, setDescription] = useState('')

  const slug = slugify(displayName)

  // External kinds (LLM adapter / embedding) are not user-trainable and always
  // use the external_api framework — hide the framework picker for them.
  const isExternalKind =
    selectedKind === 'external_llm_adapter' || selectedKind === 'embedding'
  const availableFrameworks = frameworksForKind(selectedKind)

  // Keep framework valid when the kind changes.
  function selectKind(kind: ModelKind) {
    setSelectedKind(kind)
    if (kind === 'external_llm_adapter' || kind === 'embedding') {
      setFramework('external_api')
      setRuntime('python')
    } else if (framework === 'external_api') {
      setFramework('xgboost')
    }
  }

  const canNext =
    step === 0
      ? selectedKind !== null
      : step === 1
        ? true
        : step === 2
          ? true
          : displayName.trim().length > 0

  async function handleSubmit() {
    if (!selectedKind) return
    try {
      const isExternal =
        selectedKind === 'external_llm_adapter' || selectedKind === 'embedding'
      const effectiveFramework = isExternal ? 'external_api' : framework

      const defaultTargetField: Record<string, string> = {
        forecaster: 'return',
        signal_ranker: 'score',
        trade_decision: 'action',
        risk_sizing: 'size_fraction',
        embedding: 'score',
      }
      const target =
        selectedKind === 'external_llm_adapter'
          ? undefined
          : { field: defaultTargetField[selectedKind] ?? 'return', horizon: '1h', transform: 'logret' }

      const adapter =
        selectedKind === 'external_llm_adapter'
          ? { provider: '', model: '', endpoint: '' }
          : undefined

      const res = await createMutation.mutateAsync({
        display_name: displayName.trim(),
        description: description.trim() || undefined,
        definition: {
          schema_version: '1.0',
          model_kind: selectedKind,
          framework: effectiveFramework,
          runtime,
          asset_class: assetClass,
          auto_retrain: autoRetrain,
          target,
          adapter,
        },
      })
      navigate(`/mlops/${res.data.model_id}`)
    } catch {
      // error shown via mutation state
    }
  }

  return (
    <div className="mx-auto w-full max-w-2xl px-6 py-8">
      {/* Header */}
      <div className="mb-8">
        <h1 className="text-2xl font-semibold text-text">Create Model</h1>
        <p className="mt-1 text-sm text-text-muted">
          {STEP_LABELS[step]} — Step {step + 1} of {STEP_LABELS.length}
        </p>
        <div className="mt-4">
          <StepIndicator step={step} total={STEP_LABELS.length} />
        </div>
      </div>

      {/* Step content */}
      <AnimatePresence mode="wait">
        <motion.div
          key={step}
          initial={{ opacity: 0, x: shouldReduce ? 0 : 20 }}
          animate={{ opacity: 1, x: 0 }}
          exit={{ opacity: 0, x: shouldReduce ? 0 : -20 }}
          transition={shouldReduce ? { duration: 0.001 } : SPRING}
        >
          {step === 0 && (
            <div className="space-y-3">
              <h2 className="text-base font-medium text-text mb-4">
                What will this model do?
              </h2>
              <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
                {KIND_OPTIONS.map(({ kind, label, icon: Icon, description: desc }) => (
                  <button
                    key={kind}
                    onClick={() => selectKind(kind)}
                    className={cn(
                      'text-left rounded-xl border p-4 transition-all',
                      selectedKind === kind
                        ? 'border-accent bg-accent/5 shadow-sm'
                        : 'border-border bg-surface hover:border-border-2 hover:bg-surface-2',
                    )}
                  >
                    <div className="flex items-center gap-3 mb-2">
                      <div
                        className={cn(
                          'flex h-8 w-8 items-center justify-center rounded-lg',
                          selectedKind === kind ? 'bg-accent/15 text-accent' : 'bg-surface-2 text-text-muted',
                        )}
                      >
                        <Icon className="h-4 w-4" />
                      </div>
                      <span className="font-medium text-sm text-text">{label}</span>
                      {selectedKind === kind && (
                        <Check className="h-4 w-4 text-accent ml-auto" />
                      )}
                    </div>
                    <p className="text-xs text-text-muted">{desc}</p>
                  </button>
                ))}
              </div>
            </div>
          )}

          {step === 1 && (
            <div className="space-y-6">
              {isExternalKind ? (
                <div>
                  <h2 className="text-base font-medium text-text mb-4">
                    Engine
                  </h2>
                  <div className="rounded-xl border border-accent bg-accent/5 p-4">
                    <div className="flex items-center gap-3">
                      <div className="flex h-8 w-8 items-center justify-center rounded-lg bg-accent/15 text-accent">
                        <Bot className="h-4 w-4" />
                      </div>
                      <div>
                        <p className="text-sm font-medium text-text">External API</p>
                        <p className="text-xs text-text-muted">
                          This model kind wraps an external provider — no in-house
                          training framework or runtime to choose.
                        </p>
                      </div>
                      <Check className="h-4 w-4 text-accent ml-auto shrink-0" />
                    </div>
                  </div>
                </div>
              ) : (
                <>
                  <div>
                    <h2 className="text-base font-medium text-text mb-1">
                      Training framework
                    </h2>
                    <p className="text-xs text-text-muted mb-4">
                      Which library trains the model. You can fine-tune its
                      hyperparameters later in the training playground.
                    </p>
                    <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
                      {availableFrameworks.map(
                        ({ framework: fw, label, icon: Icon, description: desc }) => (
                          <button
                            key={fw}
                            onClick={() => setFramework(fw)}
                            className={cn(
                              'text-left rounded-xl border p-4 transition-all',
                              framework === fw
                                ? 'border-accent bg-accent/5 shadow-sm'
                                : 'border-border bg-surface hover:border-border-2 hover:bg-surface-2',
                            )}
                          >
                            <div className="flex items-center gap-3 mb-2">
                              <div
                                className={cn(
                                  'flex h-8 w-8 items-center justify-center rounded-lg',
                                  framework === fw
                                    ? 'bg-accent/15 text-accent'
                                    : 'bg-surface-2 text-text-muted',
                                )}
                              >
                                <Icon className="h-4 w-4" />
                              </div>
                              <span className="font-medium text-sm text-text">
                                {label}
                              </span>
                              {framework === fw && (
                                <Check className="h-4 w-4 text-accent ml-auto" />
                              )}
                            </div>
                            <p className="text-xs text-text-muted">{desc}</p>
                          </button>
                        ),
                      )}
                    </div>
                  </div>

                  <div>
                    <h2 className="text-base font-medium text-text mb-1">
                      Runtime
                    </h2>
                    <p className="text-xs text-text-muted mb-4">
                      Where the model executes for training and inference.
                    </p>
                    <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
                      {RUNTIME_OPTIONS.map(
                        ({ runtime: rt, label, icon: Icon, description: desc, comingSoon }) => (
                          <button
                            key={rt}
                            disabled={comingSoon}
                            onClick={() => !comingSoon && setRuntime(rt)}
                            className={cn(
                              'text-left rounded-xl border p-4 transition-all',
                              comingSoon
                                ? 'border-border bg-surface opacity-50 cursor-not-allowed'
                                : runtime === rt
                                  ? 'border-accent bg-accent/5 shadow-sm'
                                  : 'border-border bg-surface hover:border-border-2 hover:bg-surface-2',
                            )}
                          >
                            <div className="flex items-center gap-3 mb-2">
                              <div
                                className={cn(
                                  'flex h-8 w-8 items-center justify-center rounded-lg',
                                  !comingSoon && runtime === rt
                                    ? 'bg-accent/15 text-accent'
                                    : 'bg-surface-2 text-text-muted',
                                )}
                              >
                                <Icon className="h-4 w-4" />
                              </div>
                              <span className="font-medium text-sm text-text">
                                {label}
                              </span>
                              {comingSoon ? (
                                <span className="ml-auto rounded-full bg-surface-2 px-2 py-0.5 text-[10px] font-medium text-text-dim">
                                  Soon
                                </span>
                              ) : (
                                runtime === rt && (
                                  <Check className="h-4 w-4 text-accent ml-auto" />
                                )
                              )}
                            </div>
                            <p className="text-xs text-text-muted">{desc}</p>
                          </button>
                        ),
                      )}
                    </div>
                  </div>
                </>
              )}
            </div>
          )}

          {step === 2 && (
            <div className="space-y-5">
              <h2 className="text-base font-medium text-text mb-4">
                Data configuration
              </h2>

              <div>
                <label className="block text-sm font-medium text-text mb-1.5">
                  Asset class
                </label>
                <select
                  value={assetClass}
                  onChange={(e) => setAssetClass(e.target.value)}
                  className="w-full h-9 rounded-lg border border-border bg-surface px-3 text-sm text-text focus:outline-none focus:border-accent"
                >
                  {ASSET_CLASSES.map((ac) => (
                    <option key={ac} value={ac}>
                      {ac}
                    </option>
                  ))}
                </select>
              </div>

              <div className="flex items-start gap-3 rounded-lg border border-border bg-surface p-4">
                <input
                  type="checkbox"
                  id="auto-retrain"
                  checked={autoRetrain}
                  onChange={(e) => setAutoRetrain(e.target.checked)}
                  className="mt-0.5 h-4 w-4 rounded border-border accent-accent"
                />
                <div>
                  <label
                    htmlFor="auto-retrain"
                    className="text-sm font-medium text-text cursor-pointer"
                  >
                    Auto-retrain
                  </label>
                  <p className="text-xs text-text-muted mt-0.5">
                    Automatically schedule retraining when model performance degrades.
                  </p>
                </div>
              </div>

              <div>
                <label className="block text-sm font-medium text-text mb-1.5">
                  Version note{' '}
                  <span className="text-text-dim font-normal">(optional)</span>
                </label>
                <input
                  type="text"
                  value={versionNote}
                  onChange={(e) => setVersionNote(e.target.value)}
                  placeholder="e.g. Initial architecture v1"
                  className="w-full h-9 rounded-lg border border-border bg-surface px-3 text-sm text-text placeholder:text-text-dim focus:outline-none focus:border-accent"
                />
              </div>
            </div>
          )}

          {step === 3 && (
            <div className="space-y-5">
              <h2 className="text-base font-medium text-text mb-4">
                Model identity
              </h2>

              <div>
                <label className="block text-sm font-medium text-text mb-1.5">
                  Display name <span className="text-pnl-down">*</span>
                </label>
                <input
                  type="text"
                  value={displayName}
                  onChange={(e) => setDisplayName(e.target.value)}
                  placeholder="e.g. BTC Price Forecaster"
                  className="w-full h-9 rounded-lg border border-border bg-surface px-3 text-sm text-text placeholder:text-text-dim focus:outline-none focus:border-accent"
                  autoFocus
                />
                {slug && (
                  <p className="mt-1.5 text-xs text-text-dim font-mono">
                    slug: {slug}
                  </p>
                )}
              </div>

              <div>
                <label className="block text-sm font-medium text-text mb-1.5">
                  Description{' '}
                  <span className="text-text-dim font-normal">(optional)</span>
                </label>
                <textarea
                  value={description}
                  onChange={(e) => setDescription(e.target.value)}
                  placeholder="Brief description of what this model does…"
                  rows={3}
                  className="w-full rounded-lg border border-border bg-surface px-3 py-2 text-sm text-text placeholder:text-text-dim focus:outline-none focus:border-accent resize-none"
                />
              </div>

              {/* Summary */}
              <div className="rounded-lg border border-border bg-surface-2 p-4 space-y-2">
                <p className="text-xs font-medium text-text-muted uppercase tracking-wide">
                  Summary
                </p>
                <div className="grid grid-cols-2 gap-x-4 gap-y-1.5 text-sm">
                  <span className="text-text-muted">Kind</span>
                  <span className="text-text font-medium">
                    {KIND_OPTIONS.find((k) => k.kind === selectedKind)?.label}
                  </span>
                  <span className="text-text-muted">Framework</span>
                  <span className="text-text font-mono text-xs">
                    {isExternalKind ? 'external_api' : framework}
                  </span>
                  {!isExternalKind && (
                    <>
                      <span className="text-text-muted">Runtime</span>
                      <span className="text-text font-mono text-xs">{runtime}</span>
                    </>
                  )}
                  <span className="text-text-muted">Asset class</span>
                  <span className="text-text font-mono text-xs">{assetClass}</span>
                  <span className="text-text-muted">Auto-retrain</span>
                  <span className="text-text">{autoRetrain ? 'Yes' : 'No'}</span>
                </div>
              </div>

              {createMutation.isError && (
                <div className="rounded-lg bg-pnl-down/10 border border-pnl-down/20 px-4 py-3 text-sm text-pnl-down">
                  Failed to create model. Please try again.
                </div>
              )}
            </div>
          )}
        </motion.div>
      </AnimatePresence>

      {/* Navigation */}
      <div className="mt-8 flex items-center justify-between">
        <Button
          variant="outline"
          onClick={() => (step === 0 ? navigate('/mlops') : setStep(step - 1))}
        >
          <ChevronLeft className="h-4 w-4" />
          {step === 0 ? 'Cancel' : 'Back'}
        </Button>

        {step < STEP_LABELS.length - 1 ? (
          <Button onClick={() => setStep(step + 1)} disabled={!canNext}>
            Next
            <ChevronRight className="h-4 w-4" />
          </Button>
        ) : (
          <Button
            onClick={handleSubmit}
            disabled={!canNext || createMutation.isPending}
          >
            {createMutation.isPending ? (
              <>
                <Loader2 className="h-4 w-4 animate-spin" />
                Creating…
              </>
            ) : (
              <>
                <Check className="h-4 w-4" />
                Create Model
              </>
            )}
          </Button>
        )}
      </div>
    </div>
  )
}
