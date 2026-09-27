import type { Provenance, VerifyResult } from '@/api/desk'
import type { Dossier, DossierClaim } from './Dossier'

/** True when any link in the chain was fetched rather than measured here. */
export function chainTouchesUntrusted(claim: DossierClaim): boolean {
  return claim.chain.includes('untrusted')
}

/**
 * The cost line, which for the whole life of this reader printed `$0.00`.
 *
 * `crates/api/src/dossier.rs` writes `"cost_usd": produced.cost_usd` straight
 * into the stored body, and `produced.cost_usd` is
 * `local_driver.rs:938 → self.yielded.cost_usd` — a field of
 * `harness::lifecycle::yield::Yield` that **nothing in the workspace ever
 * assigns**. A grep for any write to it returns the struct definition, a test
 * fixture and nothing else, so it reaches every compiled dossier at its
 * `Default`. `research_run_worker.rs` is blunter still and passes a literal
 * `Some(0.0)` twice.
 *
 * The platform says so itself, in three places, about its own inability to
 * price a run: `session_driver.rs` — *"there is no per-run USD: llm_usage.cost_usd
 * keys on the auth session, not the run"* — and `desk_running.rs` and
 * `agent_sessions.rs` say the same. The Spend table and the Cost tab already
 * print "not priced… this is not zero" on the strength of it. This document,
 * the one a reviewer opens six months later to judge whether the research was
 * worth what it cost, was the last surface still printing the zero.
 *
 * So a zero here is the absence and is rendered as one. A number that is not a
 * number is a third thing again and never becomes `$0.00` via `toFixed`, which
 * is what the old line did to a missing field by throwing.
 */
export function dossierCostLine(cost: unknown, trials: unknown): string {
  const t = typeof trials === 'number' && Number.isFinite(trials) ? `${trials} trials` : 'trials not recorded'
  if (typeof cost !== 'number' || !Number.isFinite(cost)) {
    return `Cost in dollars is not recorded on this dossier · ${t}`
  }
  if (cost === 0) {
    return `Not priced — no per-run dollar figure is recorded on this platform, so this is an absent measurement and not a run that was free · ${t}`
  }
  return `$${cost.toFixed(2)} · ${t}`
}

/**
 * `n` and `se` as the document holds them.
 *
 * `c.n === null ? '(n unknown)' : c.n` let an **absent** field through as the
 * literal `undefined`, and `c.se ?? '—'` did the same for `se`. A sample size
 * is the number a reviewer sizes every other number against; it does not get to
 * arrive as a word React prints because nobody checked.
 */
export function sampleCell(n: unknown, se: unknown): string {
  const num = (v: unknown): string | null =>
    typeof v === 'number' && Number.isFinite(v) ? String(v) : null
  return `${num(n) ?? '(n unknown)'} / ${num(se) ?? '(se unknown)'}`
}

/**
 * The Markdown export.
 *
 * Generated from the same predicate the table uses, so a claim that is marked on
 * screen is marked in the file a reviewer opens a month later.
 */
export function dossierToMarkdown(d: Dossier): string {
  const rows = d.claims
    .map((c) => {
      const mark = chainTouchesUntrusted(c) ? ' **[fetched — untrusted source in the chain]**' : ''
      // The same cell the table renders, so a reviewer opening the file a month
      // later reads the same `(n unknown)` the screen showed rather than a
      // number the export invented for it.
      const nse = sampleCell(c.n, c.se)
      return `| ${c.claim}${mark} | ${c.evidence_id} | ${nse} | ${c.verification} | ${c.evidence_class} |`
    })
    .join('\n')
  return [
    `# Dossier ${d.ticket}`,
    '',
    `## Verdict`,
    d.verdict,
    '',
    `## Evidence`,
    '| claim | id | n / se | verification | class |',
    '|---|---|---|---|---|',
    rows,
    '',
    `## Unresolved questions`,
    ...d.unresolved_questions.map((q) => `- ${q}`),
    '',
    `## Reviewer critique`,
    ...d.reviewer_critique.map((q) => `- ${q}`),
    '',
    `## Cost`,
    // The rendered sentence, not the raw number: the caveat has to travel with
    // the figure or the file becomes the place the zero gets believed.
    dossierCostLine(d.cost_usd, d.trials),
    '',
    `## Provenance`,
    `harness ${d.provenance.harness} · profile ${d.provenance.profile} · window ${d.provenance.window} · seeds ${d.provenance.seeds}`,
    '',
  ].join('\n')
}

/**
 * What the recheck row says, from the route's own answer.
 *
 * Pure because the sentence is the thing worth a test. Three of the route's
 * replies are **not** "the claim failed": a dossier that is missing, a claim the
 * dossier does not cite, and an evidence id that no longer resolves. The row
 * used to print `does not hold` over all of them, which reads as a refutation of
 * the research when it is a statement about the lookup.
 */
export function verdictOfRecheck(res: VerifyResult): string {
  const head = res.ok ? 'holds' : 'does not hold'
  // `was`/`now` only come back when the route actually re-derived something, so
  // their presence is what distinguishes a real recheck from a failed lookup.
  const rederived = typeof res.was === 'string' && typeof res.now === 'string'
  if (!rederived) return `not re-derived — ${res.note}`
  return `${head} — ${res.note}`
}

/**
 * A reviewer's notes, which are deliberately **not** part of the dossier.
 *
 * The document is a snapshot taken at the end of the run that produced it, and
 * it is immutable on purpose — a reviewer six months later has to be reading the
 * thing the decision was made on. A comment is a later reading of it, so it
 * lives beside the document and is never folded into it.
 *
 * Which is also why the thread renders even when it is empty: "nobody has
 * commented" and "comments are not loaded" look identical if the section is
 * omitted, and that is the same mistake the mandatory-section rule exists to
 * prevent.
 */
/**
 * What an empty thread says.
 *
 * A pure function because the rule is worth a test and rendering it is not: the
 * three states must stay distinguishable, and "loading" collapsing into "none
 * yet" is the same mistake as an omitted mandatory section — it reports an
 * absence of *data* as an absence of *comments*.
 */
export function commentPlaceholder(loading: boolean, count: number, failed: boolean): string | null {
  if (loading) return 'Loading comments…'
  if (failed) return 'Comments could not be loaded — this is not the same as none.'
  if (count === 0) return 'None yet — nobody has commented on this dossier.'
  return null
}
