// The dossier reader (AGENT-012 §5; tasks S-6.17, S-6.23).
//
// A document, not a card stack, in the reading layout even when the app is in
// `terminal`. Opened cold, a reviewer must be able to judge it in five minutes,
// so verdict / evidence / unresolved questions / reviewer critique are the first
// four sections and nothing is collapsed above them.
//
// §6.3a's second hard rule lands here: **a claim whose evidence chain touches
// untrusted content is marked in the evidence table and the mark survives
// export.** Both exports are generated from the same function the table renders
// from, so there is no second code path that could forget.
import { useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { Badge } from '@/components/primitives/Badge'
import { Button } from '@/components/primitives/Button'
import { EmptyState } from '@/components/primitives/States'
import type { EvidenceClass, Provenance } from '@/api/desk'
import { deskApi } from '@/api/desk'
import { EvidenceChip, InertText, ProvenanceBadge, type IdResolver } from './Shared'

export interface DossierClaim {
  claim: string
  evidence_id: string
  n: number | null
  se: number | null
  verification: string
  /** The provenance of every link in this claim's chain. */
  chain: Provenance[]
  evidence_class: EvidenceClass
}

export interface Dossier {
  ticket: string
  verdict: string
  claims: DossierClaim[]
  plan_as_executed: string[]
  funnel: Array<{ gate: string; ticket: string | null; passed: boolean }>
  unresolved_questions: string[]
  reviewer_critique: string[]
  cost_usd: number
  trials: number
  provenance: { harness: string; profile: string; window: string; seeds: string }
  /** Any untrusted text quoted in the dossier, rendered inert. */
  quoted_untrusted: Array<{ source: string; text: string }>
}

/** True when any link in the chain was fetched rather than measured here. */
export function chainTouchesUntrusted(claim: DossierClaim): boolean {
  return claim.chain.includes('untrusted')
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
      const n = c.n === null ? '(n unknown)' : String(c.n)
      const se = c.se === null ? '—' : String(c.se)
      return `| ${c.claim}${mark} | ${c.evidence_id} | ${n} | ${se} | ${c.verification} | ${c.evidence_class} |`
    })
    .join('\n')
  return [
    `# Dossier ${d.ticket}`,
    '',
    `## Verdict`,
    d.verdict,
    '',
    `## Evidence`,
    '| claim | id | n | se | verification | class |',
    '|---|---|---|---|---|---|',
    rows,
    '',
    `## Unresolved questions`,
    ...d.unresolved_questions.map((q) => `- ${q}`),
    '',
    `## Reviewer critique`,
    ...d.reviewer_critique.map((q) => `- ${q}`),
    '',
    `## Cost`,
    `$${d.cost_usd.toFixed(2)} · ${d.trials} trials`,
    '',
    `## Provenance`,
    `harness ${d.provenance.harness} · profile ${d.provenance.profile} · window ${d.provenance.window} · seeds ${d.provenance.seeds}`,
    '',
  ].join('\n')
}

export function DossierReader({ ticket, resolve }: { ticket: string; resolve?: IdResolver }) {
  const [verifying, setVerifying] = useState<string | null>(null)
  const [results, setResults] = useState<Record<string, string>>({})

  const q = useQuery({
    queryKey: ['desk', 'dossier', ticket],
    queryFn: () => deskApi.dossier(ticket),
  })

  if (q.isLoading) return <div className="desk-scroll">Loading…</div>
  const d = (q.data as unknown as Dossier | null) ?? null
  if (!d) {
    return (
      <div className="desk-scroll">
        <EmptyState
          message="No dossier for this ticket"
          detail="/api/dossiers/{ticket} is not built yet (AGENT-012 §17). The reader is wired and renders the moment it answers."
        />
      </div>
    )
  }

  async function verify(claim: DossierClaim) {
    setVerifying(claim.evidence_id)
    const res = await deskApi.verifyClaim(ticket, claim.evidence_id)
    setResults((r) => ({ ...r, [claim.evidence_id]: res ? `${res.ok ? 'holds' : 'does not hold'} — ${res.note}` : 'the recheck endpoint did not answer' }))
    setVerifying(null)
  }

  function download(name: string, content: string, type: string) {
    const blob = new Blob([content], { type })
    const url = URL.createObjectURL(blob)
    const a = document.createElement('a')
    a.href = url
    a.download = name
    a.click()
    URL.revokeObjectURL(url)
  }

  return (
    <div className="desk-scroll">
      <div className="dossier" data-theme="paper">
        <h2>Verdict</h2>
        <p>{d.verdict}</p>

        <h2>Evidence</h2>
        <table className="tbl">
          <thead>
            <tr>
              <th style={{ textAlign: 'left' }}>claim</th>
              <th>id</th>
              <th>n / se</th>
              <th>verification</th>
              <th>verify</th>
            </tr>
          </thead>
          <tbody>
            {d.claims.map((c) => (
              <tr key={c.evidence_id}>
                <td style={{ textAlign: 'left' }}>
                  {c.claim}{' '}
                  <ProvenanceBadge
                    mark={{
                      provenance: chainTouchesUntrusted(c) ? 'untrusted' : 'platform',
                      evidence_class: c.evidence_class,
                    }}
                  />
                </td>
                <td>
                  <EvidenceChip id={c.evidence_id} resolve={resolve} />
                </td>
                <td>
                  {c.n === null ? '(n unknown)' : c.n} / {c.se ?? '—'}
                </td>
                <td>{c.verification}</td>
                <td>
                  <Button size="xs" variant="ghost" onClick={() => void verify(c)} loading={verifying === c.evidence_id}>
                    Verify
                  </Button>
                  {results[c.evidence_id] && <div className="sub">{results[c.evidence_id]}</div>}
                </td>
              </tr>
            ))}
          </tbody>
        </table>

        <h2>Unresolved questions</h2>
        {d.unresolved_questions.length === 0 ? (
          <p>
            <Badge tone="warn">mandatory section is empty</Badge> A dossier that lists no unresolved question is
            claiming completeness it has not earned.
          </p>
        ) : (
          <ul>
            {d.unresolved_questions.map((x) => (
              <li key={x}>{x}</li>
            ))}
          </ul>
        )}

        <h2>Reviewer critique</h2>
        {d.reviewer_critique.length === 0 ? (
          <p>
            <Badge tone="warn">mandatory section is empty</Badge>
          </p>
        ) : (
          <ul>
            {d.reviewer_critique.map((x) => (
              <li key={x}>{x}</li>
            ))}
          </ul>
        )}

        <h3>Plan as executed</h3>
        <ol>
          {d.plan_as_executed.map((p, i) => (
            <li key={i}>{p}</li>
          ))}
        </ol>

        <h3>Funnel</h3>
        <ul>
          {d.funnel.map((f) => (
            <li key={f.gate}>
              {f.gate} — {f.passed ? 'passed' : 'not passed'}{' '}
              {f.ticket ? <EvidenceChip id={f.ticket} resolve={resolve} /> : <span className="sub">no ticket</span>}
            </li>
          ))}
        </ul>

        {d.quoted_untrusted.length > 0 && (
          <>
            <h3>Quoted from fetched sources</h3>
            {d.quoted_untrusted.map((qu, i) => (
              <div key={i}>
                <span className="sub">{qu.source}</span>
                <InertText>{qu.text}</InertText>
              </div>
            ))}
          </>
        )}

        <h3>Cost</h3>
        <p>
          ${d.cost_usd.toFixed(2)} · {d.trials} trials
        </p>

        <h3>Provenance</h3>
        <p className="sub">
          harness {d.provenance.harness} · profile {d.provenance.profile} · window {d.provenance.window} · seeds{' '}
          {d.provenance.seeds}
        </p>

        <div className="desk-actions">
          <Button size="xs" onClick={() => window.print()}>
            Export PDF
          </Button>
          <Button
            size="xs"
            variant="secondary"
            onClick={() => download(`${d.ticket}.md`, dossierToMarkdown(d), 'text/markdown')}
          >
            Export Markdown
          </Button>
          <Button
            size="xs"
            variant="secondary"
            onClick={() => download(`${d.ticket}.json`, JSON.stringify(d, null, 2), 'application/json')}
          >
            Export JSON
          </Button>
        </div>
      </div>
    </div>
  )
}
