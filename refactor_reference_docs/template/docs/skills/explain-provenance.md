# Skill: Explain Provenance

## Purpose
Make any element of the design justify its own existence by tracing the causal chain upward to the
artifact's problem statement — turning the reference graph into an explanation engine.

## When to Use
- When someone proposes removing or changing an element and you need to surface the reasoning at stake
- When a reviewer or new team member asks "why is this here?"
- When auditing intentionality — does every element actually trace back to a real need?
- Whenever a requirement, decision, or task looks arbitrary and you want to confirm it isn't

---

## Procedures Used
- `procedures/explain-provenance.md`

---

## Workflow

### 1. Walk up, not down
Provenance answers "why does this exist?" — it follows links toward the artifact, the opposite of
impact analysis. If you want "what depends on this?", use `skills/analyze-impact.md` instead.

### 2. Always terminate at the artifact
A complete provenance chain ends at a problem statement, success condition, or failure mode in
`docs/artifact.md`. If it ends anywhere else, the chain is broken — that is the finding, not a
detail to gloss over.

### 3. Use it as the counter to deletion
The single highest-value moment for provenance is when someone says "let's just remove this." The
narrative tells you, in one sentence, exactly what reasoning you would be unwinding — and whether
anything downstream still needs it. Pair it with `analyze-impact` for the full picture: provenance
says *why it exists*, impact says *what would break*.

### 4. Treat broken provenance as a real finding
An element whose chain does not reach the artifact is either gold-plating or a missing link. Both
are worth fixing. Do not fabricate a justification to make the chain look complete.

---

## Tips

- Provenance is about *intent*; structure (does the link resolve?) is `verify-traceability`'s job.
  An element can have a resolving link and still have no real justification behind it.
- The prose form is the one people actually read. Lead with it; keep the arrow chain as the index.
- If producing provenance is hard because the upward links don't exist, that itself is the lesson:
  the design was built without deriving it — exactly what the artifact-first workflow exists to prevent.
