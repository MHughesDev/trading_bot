import { useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { motion, useReducedMotion } from 'framer-motion'
import { Bug, Skull, ShieldCheck, ShieldAlert, ShieldX } from 'lucide-react'
import { cn } from '@/lib/utils'
import {
  falsificationApi,
  scoreLabel,
  DEFECT_CLASS_LABEL,
  VERIFICATION_LABEL,
  type FalsificationReport,
} from '@/api/falsification'

/**
 * The falsification ledger (EXTERNAL-REVIEW-2026-09 §4; ADR-P6-05, ADR-P6-06).
 *
 * Most research surfaces show what was found. This one shows what was *refuted*,
 * because that is the agent's primary output: "find alpha" produces a usable
 * signal about twice a year on a ninety-day lag, and "prove this is noise"
 * produces one every few minutes.
 *
 * Two rendering decisions carry the whole design:
 *
 * 1. **An evaluator defect is rendered above a kill, and a verified one is
 *    rendered as the most valuable thing on the page.** If the surface ranked
 *    promotions first, it would be teaching the reader the opposite of what the
 *    scoring rule says — and the scoring rule exists because an agent scored on
 *    promotions hides simulator bugs.
 *
 * 2. **Unverified is a word, never a zero.** A report nobody has re-run scores
 *    nothing *yet*, which is a different fact from a report that was re-run and
 *    failed. Rendering both as `0` would make the queue look settled.
 */

const VERIFICATION_CONFIG = {
  verified: { tone: 'pos', Icon: ShieldCheck },
  unverified: { tone: 'neutral', Icon: ShieldAlert },
  refuted: { tone: 'neutral', Icon: ShieldX },
} as const

function ReportRow({ report }: { report: FalsificationReport }) {
  const reduce = useReducedMotion()
  const [open, setOpen] = useState(false)
  const cfg = VERIFICATION_CONFIG[report.verification]
  const isDefect = report.report_kind === 'evaluator_defect'
  const Kind = isDefect ? Bug : Skull

  return (
    <motion.li
      initial={reduce ? false : { opacity: 0, y: 4 }}
      animate={{ opacity: 1, y: 0 }}
      className="border-b border-[var(--border-subtle)] py-2 last:border-b-0"
    >
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        aria-expanded={open}
        className="flex w-full items-start gap-2 text-left"
      >
        <Kind
          aria-hidden
          className={cn(
            'mt-0.5 size-4 shrink-0',
            isDefect ? 'text-[var(--fg-warn)]' : 'text-[var(--fg-tertiary)]',
          )}
        />
        <span className="min-w-0 flex-1">
          <span className="block truncate text-sm">
            {isDefect ? report.component : DEFECT_CLASS_LABEL[report.defect_class ?? 'invalid_metric']}
          </span>
          {/* The claim, in one line. `expected` is what the reporter predicted,
              `observed` is what happened, and the database refuses a report
              where they match — which is what makes a bare low score unfilable. */}
          <span className="block truncate text-xs text-[var(--fg-secondary)]">
            expected {report.expected} · observed {report.observed}
          </span>
        </span>
        <span className="shrink-0 text-right">
          <span className={cn('badge', cfg.tone)}>
            {VERIFICATION_LABEL[report.verification]}
          </span>
          <span className="mt-0.5 block font-mono text-[10px] text-[var(--fg-tertiary)]">
            {scoreLabel(report)}
          </span>
        </span>
      </button>

      {open && (
        <div className="mt-2 space-y-2 pl-6 text-xs">
          {isDefect && report.claimed_guarantee && (
            <p className="text-[var(--fg-secondary)]">
              <span className="text-[var(--fg-tertiary)]">Claimed guarantee: </span>
              {report.claimed_guarantee}
            </p>
          )}
          <div>
            <p className="text-[var(--fg-tertiary)]">
              Reproduction — {report.artifact_ref}, seed {report.seed}
            </p>
            <ol className="mt-1 list-decimal space-y-0.5 pl-4 text-[var(--fg-secondary)]">
              {report.steps.map((s, i) => (
                <li key={i}>{s}</li>
              ))}
            </ol>
          </div>
          {/* Verification closes the question; only a test closes the hole. A
              verified defect with no regression test stays on the queue, and
              saying so here is cheaper than letting somebody assume otherwise. */}
          {isDefect && report.verification === 'verified' && (
            <p
              className={cn(
                report.regression_test
                  ? 'text-[var(--fg-pos)]'
                  : 'text-[var(--fg-warn)]',
              )}
            >
              {report.regression_test
                ? `Covered by ${report.regression_test}`
                : 'Verified, but no regression test covers it — this defect can come back.'}
            </p>
          )}
          <p className="font-mono text-[10px] text-[var(--fg-tertiary)]">
            filed by {report.reported_by} ({report.reporter_kind}) ·{' '}
            {new Date(report.reported_at).toLocaleString()}
            {report.verified_by ? ` · checked by ${report.verified_by}` : ''}
          </p>
        </div>
      )}
    </motion.li>
  )
}

export function FalsificationPanel({ className }: { className?: string }) {
  const { data, isLoading, isError, error } = useQuery({
    queryKey: ['falsification', 'ledger'],
    queryFn: () => falsificationApi.list().then((r) => r.data),
    staleTime: 30_000,
  })

  if (isLoading) {
    return (
      <section className={cn('panel p-4', className)}>
        <h2 className="text-sm font-semibold">Falsification ledger</h2>
        <div className="mt-3 space-y-2" aria-busy>
          {Array.from({ length: 4 }).map((_, i) => (
            <div key={i} className="h-10 animate-pulse rounded bg-[var(--bg-surface-raised)]" />
          ))}
        </div>
      </section>
    )
  }

  if (isError || !data) {
    return (
      <section className={cn('panel p-4', className)}>
        <h2 className="text-sm font-semibold">Falsification ledger</h2>
        <p className="mt-2 text-sm text-[var(--fg-warn)]">
          The ledger could not be loaded, so nothing here is known — including whether
          anything is waiting to be checked.
        </p>
        <p className="mt-1 font-mono text-xs text-[var(--fg-secondary)]">
          {error instanceof Error ? error.message : 'unknown error'}
        </p>
      </section>
    )
  }

  // Defects first, then kills; within each, unverified first — the queue is
  // what a reader can act on, and a settled report is not.
  const ordered = [...data.reports].sort((a, b) => {
    const kind = Number(b.report_kind === 'evaluator_defect') - Number(a.report_kind === 'evaluator_defect')
    if (kind !== 0) return kind
    return Number(a.verification !== 'unverified') - Number(b.verification !== 'unverified')
  })

  return (
    <section className={cn('panel p-4', className)}>
      <header className="flex items-baseline justify-between gap-3">
        <h2 className="text-sm font-semibold">Falsification ledger</h2>
        <span className="font-mono text-xs text-[var(--fg-secondary)]">
          {data.counts.total} reports
        </span>
      </header>

      {/* Counted separately, for the same reason the health panel counts its
          states separately: "3 open" would fold a defect nobody has reproduced
          into the same number as one that is verified and uncovered, and those
          are different jobs for different people. */}
      <p className="mt-1 text-xs text-[var(--fg-secondary)]">
        {data.counts.undischarged_defects} verified defect
        {data.counts.undischarged_defects === 1 ? '' : 's'} with no regression test ·{' '}
        {data.counts.unverified} report{data.counts.unverified === 1 ? '' : 's'} nobody has
        re-run
      </p>

      {ordered.length === 0 ? (
        <p className="mt-3 text-sm text-[var(--fg-secondary)]">
          Nothing filed yet. An empty ledger is not the same as a platform with no
          defects — it means nothing has looked.
        </p>
      ) : (
        <ul className="mt-2">
          {ordered.map((r) => (
            <ReportRow key={r.falsification_id} report={r} />
          ))}
        </ul>
      )}
    </section>
  )
}
