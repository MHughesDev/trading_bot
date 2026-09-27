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
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Badge } from '@/components/primitives/Badge'
import { Button } from '@/components/primitives/Button'
import { EmptyState } from '@/components/primitives/States'
import type { DossierComment, DossierRead, EvidenceClass, Provenance, VerifyResult } from '@/api/desk'
import { deskApi } from '@/api/desk'
import { EvidenceChip, InertText, ProvenanceBadge, type IdResolver } from './Shared'
import { chainTouchesUntrusted, dossierCostLine, sampleCell, dossierToMarkdown, verdictOfRecheck, commentPlaceholder } from './Dossier.helpers'

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

export function DossierReader({ ticket, resolve }: { ticket: string; resolve?: IdResolver }) {
  const [verifying, setVerifying] = useState<string | null>(null)
  const [results, setResults] = useState<Record<string, string>>({})

  const q = useQuery({
    queryKey: ['desk', 'dossier', ticket],
    queryFn: () => deskApi.dossier(ticket),
  })

  if (q.isLoading) return <div className="desk-scroll">Loading…</div>
  const read: DossierRead = q.data ?? {
    kind: 'unreadable',
    message: q.error instanceof Error ? q.error.message : 'the request did not come back',
  }
  if (read.kind === 'none') {
    // **Not "the route is not built".** It is, and it answered — with the one
    // explanation a reader of an empty dossier page wants, which is why the
    // route bothers to send it.
    return (
      <div className="desk-scroll">
        <EmptyState
          message={`No dossier was compiled for ${ticket}`}
          detail={
            read.fix ??
            'The route answered that it holds no dossier under this ticket, and gave no further reason.'
          }
        />
      </div>
    )
  }
  if (read.kind === 'unreadable') {
    return (
      <div className="desk-scroll">
        <EmptyState
          message="The dossier could not be read"
          detail={`${read.message}. This is the request failing, not a run that produced no document — do not read it as either.`}
        />
      </div>
    )
  }
  const d = read.body as unknown as Dossier

  async function verify(claim: DossierClaim) {
    setVerifying(claim.evidence_id)
    const res = await deskApi.verifyClaim(ticket, claim.evidence_id)
    setResults((r) => ({ ...r, [claim.evidence_id]: verdictOfRecheck(res) }))
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
                <td>{sampleCell(c.n, c.se)}</td>
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
        <p data-testid="dossier-cost">{dossierCostLine(d.cost_usd, d.trials)}</p>

        <h3>Provenance</h3>
        <p className="sub">
          harness {d.provenance.harness} · profile {d.provenance.profile} · window {d.provenance.window} · seeds{' '}
          {d.provenance.seeds}
        </p>

        <h3>Comments</h3>
        <CommentThread ticket={d.ticket} />

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
          {/* **Not "signed JSON", which is what AGENT-012 §5 asks for.**
              Nothing on this platform can sign one: there is no key, no
              signature field on `mlops.dossier`, and no signing anywhere in
              `crates/api/src/dossier.rs` or `routes/dossiers.rs`. Doing it in
              the browser would be theatre — a signature a page computes over
              bytes the same page received attests to nothing a reader could
              check.
              A label claiming a cryptographic property the artifact does not
              have is worse than no label, because it is exactly the label that
              stops someone asking. So the button says what the file is, and
              the line below says what it is not. */}
          <Button
            size="xs"
            variant="secondary"
            title="The platform's stored body, byte for byte. It carries no signature."
            onClick={() => download(`${d.ticket}.json`, JSON.stringify(d, null, 2), 'application/json')}
          >
            Export JSON
          </Button>
        </div>
        <p className="sub" data-testid="export-not-signed">
          The JSON is the stored record verbatim and is <strong>not signed</strong> — this platform
          has no signing key, so nothing in the file proves it came from here. Check it against{' '}
          <code>/api/dossiers/{d.ticket}</code> rather than trusting the copy.
        </p>
      </div>
    </div>
  )
}

export function CommentThread({ ticket }: { ticket: string }) {
  const qc = useQueryClient()
  const [draft, setDraft] = useState('')

  const q = useQuery({
    queryKey: ['desk', 'dossier', ticket, 'comments'],
    queryFn: () => deskApi.dossierComments(ticket),
  })
  const add = useMutation({
    mutationFn: (body: string) => deskApi.addDossierComment(ticket, body),
    onSuccess: () => {
      setDraft('')
      void qc.invalidateQueries({ queryKey: ['desk', 'dossier', ticket, 'comments'] })
    },
  })

  const comments: DossierComment[] = q.data?.comments ?? []
  const placeholder = commentPlaceholder(q.isLoading, comments.length, q.isError)

  return (
    <div className="dossier-comments">
      {placeholder !== null ? (
        <p className="sub">{placeholder}</p>
      ) : (
        <ul>
          {comments.map((c) => (
            <li key={c.comment_id}>
              <span className="sub">
                {c.author}
                {c.claim ? ` · on ${c.claim}` : ''}
              </span>
              {/* A comment is text somebody else wrote. Rendered inert for the
                  same reason a fetched page is. */}
              <InertText>{c.body}</InertText>
            </li>
          ))}
        </ul>
      )}
      <textarea
        value={draft}
        onChange={(e) => setDraft(e.target.value)}
        placeholder="Add a note for the next reader"
        rows={3}
      />
      <Button size="xs" disabled={draft.trim().length === 0 || add.isPending} onClick={() => add.mutate(draft)}>
        {add.isPending ? 'Saving…' : 'Comment'}
      </Button>
      {add.isError && <p className="sub">The comment was not saved.</p>}
    </div>
  )
}
