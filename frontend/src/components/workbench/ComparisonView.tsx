import { cn } from '@/lib/utils'

/**
 * Run comparison, lineage diff and "why did A win" (SPEC §11.5; plan 5.4).
 *
 * Three things on one surface, because separately they invite the wrong
 * reading:
 *
 * **The matrix, not a leaderboard.** A ranked list implies a total order that a
 * set of paired comparisons does not establish. Every cell here is one pair's
 * own evidence — the mean difference, the win/tie/loss record, the achieved
 * replicate count — and the reader does the comparing.
 *
 * **The flags, always.** `non_comparable`, `overlapping_labels_unweighted`,
 * `uses_backfilled_knowledge`, `legacy_ungated`, split overrides. A verdict
 * shown without them is the bare number `stats::compare` exists to prevent.
 *
 * **Attribution is arithmetic.** "Why A won" is a decomposition — gate margins,
 * regime-sliced deltas — not a sentence. A narrative explanation of a
 * statistical result is a story fitted to an outcome, and it is persuasive in
 * exactly the cases where it is least justified.
 */

export type Verdict = 'promote' | 'reject' | 'keep_sampling' | 'practically_equivalent'

export interface ComparisonFlags {
  non_comparable: boolean
  overlapping_labels_unweighted: boolean
  split_overrides: boolean
  uses_backfilled_knowledge: boolean
  legacy_ungated: boolean
}

export interface MatrixCell {
  mean_difference: number
  wins: number
  ties: number
  losses: number
  /** Descriptive only — §11.5 is explicit that nothing turns on it. */
  wilcoxon_p_descriptive: number
  verdict: Verdict
  achieved_k: number
}

export interface ComparisonView {
  names: string[]
  /** Keyed `"a|b"`. */
  cells: Record<string, MatrixCell>
  flags: ComparisonFlags
  profile_id: string
  n_eff: number
  delta_practical: number
  /** Per-regime mean difference. Attribution, not narration. */
  regime_deltas: Record<string, number>
  /** Gate-by-gate margin difference, A minus B. */
  gate_margins: Record<string, number>
}

const VERDICT_TONE: Record<Verdict, string> = {
  promote: 'pos',
  reject: 'neg',
  keep_sampling: 'info',
  practically_equivalent: 'neutral',
}

const VERDICT_LABEL: Record<Verdict, string> = {
  promote: 'Promote',
  reject: 'Not by enough',
  keep_sampling: 'Keep sampling',
  // Deliberately not "equivalent": the protocol spent its budget and could not
  // separate them, which is not evidence that they are the same.
  practically_equivalent: 'Not separated',
}

function flagList(flags: ComparisonFlags): string[] {
  return Object.entries(flags)
    .filter(([, v]) => v)
    .map(([k]) => k)
}

export function ComparisonMatrixView({
  view,
  className,
}: {
  view: ComparisonView
  className?: string
}) {
  const raised = flagList(view.flags)

  return (
    <section className={cn('panel p-4', className)}>
      <header className="flex flex-wrap items-baseline justify-between gap-2">
        <h2 className="text-sm font-semibold">Comparison</h2>
        {/* The counter beside the number, always (§12.2). */}
        <span className="font-mono text-xs text-[var(--fg-secondary)]">
          {view.profile_id} · N_eff {view.n_eff.toFixed(1)} · δ {view.delta_practical.toFixed(3)}
        </span>
      </header>

      {raised.length > 0 && (
        <p className="mt-1 text-xs text-[var(--fg-warn)]">
          {raised.join(', ')} — every number below is qualified by this.
        </p>
      )}

      {/* The matrix. No critical-difference diagram exists anywhere in this
          codebase (AT-62): a CD diagram's cliques depend on which other
          candidates happen to be present, so adding an unrelated one can make
          two that did not move become "not significantly different". */}
      <div className="mt-3 overflow-x-auto">
        <table className="w-full text-xs">
          <thead>
            <tr className="text-[var(--fg-tertiary)]">
              <th className="p-1 text-left font-normal" />
              {view.names.map((n) => (
                <th key={n} className="p-1 text-left font-normal">
                  {n}
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {view.names.map((row) => (
              <tr key={row} className="border-t border-[var(--line-subtle)]">
                <th className="p-1 text-left font-medium">{row}</th>
                {view.names.map((col) => {
                  if (row === col) {
                    return (
                      <td key={col} className="p-1 text-[var(--fg-tertiary)]">
                        —
                      </td>
                    )
                  }
                  const cell = view.cells[`${row}|${col}`]
                  if (!cell) {
                    return (
                      <td key={col} className="p-1 italic text-[var(--fg-tertiary)]">
                        not compared
                      </td>
                    )
                  }
                  return (
                    <td key={col} className="p-1" data-verdict={cell.verdict}>
                      <div className="font-mono tabular-nums">
                        {cell.mean_difference >= 0 ? '+' : ''}
                        {cell.mean_difference.toFixed(4)}
                      </div>
                      <div className="text-[var(--fg-secondary)]">
                        {cell.wins}/{cell.ties}/{cell.losses} over k={cell.achieved_k}
                      </div>
                      <span className={cn('badge', VERDICT_TONE[cell.verdict])}>
                        {VERDICT_LABEL[cell.verdict]}
                      </span>
                      {/* Named as descriptive on the surface, not only in the
                          type: a bare p-value beside a verdict reads as the
                          reason for it. */}
                      <div className="text-[10px] text-[var(--fg-tertiary)]">
                        Wilcoxon p {cell.wilcoxon_p_descriptive.toFixed(3)} (descriptive)
                      </div>
                    </td>
                  )
                })}
              </tr>
            ))}
          </tbody>
        </table>
      </div>

      {/* Attribution: a decomposition, not a sentence. */}
      <div className="mt-4 grid gap-4 md:grid-cols-2">
        <div>
          <h3 className="text-xs font-semibold">Where the difference came from</h3>
          {Object.keys(view.regime_deltas).length === 0 ? (
            <p className="mt-1 text-xs italic text-[var(--fg-tertiary)]">
              no regime labels for this window — the difference is not attributed
            </p>
          ) : (
            <ul className="mt-1 space-y-0.5">
              {Object.entries(view.regime_deltas).map(([regime, delta]) => (
                <li key={regime} className="flex justify-between font-mono text-xs">
                  <span className="text-[var(--fg-secondary)]">{regime}</span>
                  <span className="tabular-nums">
                    {delta >= 0 ? '+' : ''}
                    {delta.toFixed(4)}
                  </span>
                </li>
              ))}
            </ul>
          )}
        </div>
        <div>
          <h3 className="text-xs font-semibold">Gate margins, A − B</h3>
          {Object.keys(view.gate_margins).length === 0 ? (
            <p className="mt-1 text-xs italic text-[var(--fg-tertiary)]">
              neither candidate has a recorded gate stack
            </p>
          ) : (
            <ul className="mt-1 space-y-0.5">
              {Object.entries(view.gate_margins).map(([gate, margin]) => (
                <li key={gate} className="flex justify-between font-mono text-xs">
                  <span className="text-[var(--fg-secondary)]">{gate}</span>
                  <span className="tabular-nums">
                    {margin >= 0 ? '+' : ''}
                    {margin.toFixed(3)}
                  </span>
                </li>
              ))}
            </ul>
          )}
        </div>
      </div>
    </section>
  )
}
