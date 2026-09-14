// The approvals inbox (COMP-006 §4, UI-05).
//
// Every kind from `approval_requests`: ask_user, plan, budget, qc_waiver,
// model_promotion, skill_promotion, paper_deployment. A kind with no card is an
// approval that blocks a session forever with nothing on screen to answer it, so the
// generic card renders any kind the server sends and the known kinds add detail.
//
// Three things every card shows, because an approval without them is a button:
// the **evidence** the decision rests on, the **default** that happens on timeout,
// and the **time left**. A reviewer who cannot see what happens if they do nothing
// is not making a decision, they are clearing a notification.

import { useState } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { Clock, FileText, ShieldAlert } from 'lucide-react'
import { researchApi, type Approval } from '@/api/research'

interface Props {
  projectId?: string
  /** Called after an approval is answered, so a parent can refresh. */
  onAnswered?: () => void
}

const KIND_LABELS: Record<string, string> = {
  ask_user: 'Question',
  plan: 'Plan approval',
  budget: 'Budget increase',
  qc_waiver: 'Data quality waiver',
  model_promotion: 'Model promotion',
  skill_promotion: 'Skill promotion',
  paper_deployment: 'Paper deployment',
  // The policy engine paused a tool call (ADR-0032). The rule that fired is in the
  // payload's `code`, so one label serves every rule.
  tool_call: 'Tool call needs approval',
}

/** Option ids, whether the server sent strings or objects. */
function optionIds(options: unknown): string[] {
  if (!Array.isArray(options)) return []
  return options
    .map((o) =>
      typeof o === 'string'
        ? o
        : ((o as { id?: string; label?: string }).id ??
          (o as { label?: string }).label ??
          ''),
    )
    .filter(Boolean)
}

function timeLeft(timeoutAt?: string | null): string | null {
  if (!timeoutAt) return null
  const ms = new Date(timeoutAt).getTime() - Date.now()
  if (ms <= 0) return 'expired'
  const mins = Math.round(ms / 60_000)
  if (mins < 60) return `${mins}m left`
  const hours = Math.round(mins / 60)
  return hours < 48 ? `${hours}h left` : `${Math.round(hours / 24)}d left`
}

/** Evidence handles carried on the payload, rendered as links to the artifact. */
function evidenceOf(payload: Record<string, unknown>): string[] {
  const keys = ['dossier_ref', 'evidence', 'artifact', 'eval_results', 'qc_report', 'diff']
  const out: string[] = []
  for (const k of keys) {
    const v = payload[k]
    if (typeof v === 'string') out.push(v)
    else if (Array.isArray(v)) out.push(...v.filter((x): x is string => typeof x === 'string'))
  }
  return out
}

function ApprovalCard({
  approval,
  onAnswered,
}: {
  approval: Approval
  onAnswered?: () => void
}) {
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const options = optionIds(approval.options)
  const left = timeLeft(approval.timeout_at)
  const evidence = evidenceOf(approval.payload)
  // `reason` is what the policy engine writes (ADR-0032). Without it in this chain a
  // tool-call approval renders as an empty card with two buttons, which is the exact
  // shape of "an approval nobody can answer" — and the loop stays suspended until
  // someone does.
  const question =
    (approval.payload.question as string) ??
    (approval.payload.summary as string) ??
    (approval.payload.text as string) ??
    (approval.payload.reason as string) ??
    ''
  const tool = approval.payload.tool as string | undefined
  const fix = approval.payload.fix as string | undefined

  const answer = async (option: string) => {
    setBusy(true)
    setError(null)
    try {
      await researchApi.answerApproval(approval.approval_id, option)
      onAnswered?.()
    } catch (e) {
      const detail = (e as { response?: { data?: { message?: string } } }).response?.data
      setError(detail?.message ?? 'the answer could not be recorded')
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="rounded-lg border border-border bg-surface p-3">
      <div className="mb-2 flex items-center gap-2">
        <ShieldAlert className="h-3.5 w-3.5 text-accent" />
        <span className="text-xs font-semibold text-text">
          {KIND_LABELS[approval.kind] ?? approval.kind}
        </span>
        {left && (
          <span className="ml-auto flex items-center gap-1 text-[11px] text-text-dim">
            <Clock className="h-3 w-3" />
            {left}
          </span>
        )}
      </div>

      {question && (
        <p className="mb-2 whitespace-pre-wrap text-sm text-text">{question}</p>
      )}

      {/* What is actually about to run. An approval that does not show the call is
          asking someone to authorise something they cannot see. */}
      {tool && (
        <div className="mb-2 rounded border border-border bg-surface-2 p-2">
          <div className="font-mono text-[11px] text-accent">{tool}</div>
          {approval.payload.arguments != null && (
            <pre className="mt-1 overflow-x-auto whitespace-pre-wrap break-all font-mono text-[10px] text-text-muted">
              {JSON.stringify(approval.payload.arguments, null, 2)}
            </pre>
          )}
        </div>
      )}
      {fix && <p className="mb-2 text-xs text-text-muted">Alternative: {fix}</p>}

      {evidence.length > 0 && (
        <div className="mb-2 flex flex-wrap gap-1.5">
          {evidence.map((handle) => (
            <a
              key={handle}
              href={`/api/artifacts/${handle}`}
              target="_blank"
              rel="noreferrer"
              className="flex items-center gap-1 rounded border border-border px-1.5 py-0.5 font-mono text-[10px] text-text-muted hover:text-accent"
            >
              <FileText className="h-3 w-3" />
              {handle}
            </a>
          ))}
        </div>
      )}

      {approval.default_option && (
        <p className="mb-2 text-[11px] text-text-dim">
          If nothing is answered, the default is{' '}
          <span className="font-mono text-text-muted">{approval.default_option}</span>.
        </p>
      )}

      {error && <p className="mb-2 text-[11px] text-pnl-down">{error}</p>}

      <div className="flex flex-wrap gap-1.5">
        {(options.length > 0 ? options : ['approve', 'reject']).map((opt) => (
          <button
            key={opt}
            type="button"
            disabled={busy}
            onClick={() => void answer(opt)}
            className={`rounded-lg border px-2.5 py-1 text-xs disabled:opacity-40 ${
              opt === approval.default_option
                ? 'border-accent text-accent'
                : 'border-border text-text-muted hover:text-text'
            }`}
          >
            {opt}
          </button>
        ))}
      </div>
    </div>
  )
}

export function ApprovalsInbox({ projectId, onAnswered }: Props) {
  const queryClient = useQueryClient()
  const key = ['research', 'approvals', projectId ?? null]
  const { data: approvals = [], isLoading } = useQuery({
    queryKey: key,
    queryFn: async () =>
      (await researchApi.approvals({ project_id: projectId })).data.approvals,
    refetchInterval: 15_000,
  })
  const refresh = () => {
    void queryClient.invalidateQueries({ queryKey: key })
    onAnswered?.()
  }

  if (isLoading)
    return <div className="text-xs text-text-dim">Loading approvals…</div>
  if (approvals.length === 0) {
    return (
      <div className="py-6 text-center text-xs text-text-dim">
        Nothing waiting on you.
      </div>
    )
  }

  return (
    <div className="flex flex-col gap-2">
      {approvals.map((a) => (
        <ApprovalCard key={a.approval_id} approval={a} onAnswered={refresh} />
      ))}
    </div>
  )
}
