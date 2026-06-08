# Procedure: Trace Uncertainty

## Purpose
Find the parts of the design that rest on unresolved ground. Some elements stand on settled
decisions; others stand on an open question or an untested assumption. This procedure tags the
sources of uncertainty, propagates that uncertainty downstream, and renders a **risk surface** —
the slice of the design that would move if the shaky foundations turn out wrong.

Where `verify-traceability.md` checks whether links *resolve*, this checks whether what they point
at is actually *known*. A reference can resolve perfectly and still rest on a guess.

## Trigger
Use this procedure when:
- Deciding what to validate next — the risk surface ranks where validation buys the most certainty
- Before committing to a plan, to see how much of it depends on unresolved open questions
- After an open question is resolved or an assumption is validated, to recompute what is now solid
- When a stakeholder asks "how confident are we in this design?"

## Inputs
- Read access to the whole `docs/` tree
- `open-questions.md` (the open-question register) and every spec's §7.2 Assumptions

---

## Steps

### 1. Collect the uncertainty sources
Two kinds of element are sources of uncertainty:
- **Open questions** — every row in `open-questions.md` whose status is `Open` or `Researching`
  (a `Resolved` question is settled ground and is not a source).
- **Unvalidated assumptions** — every spec assumption (§7.2.x) that does not carry a
  `Validated:` value (i.e. still `Validated: [—]`). A validated assumption is settled.

List each source with its ID (`Q-5`, `FEAT-001 §7.2.C`).

### 2. Propagate downstream from each source
For each source, walk the graph downward (the same downstream traversal as `analyze-impact.md`)
to find every element that depends on it:
- For an open question: every spec, ADR, plan, and task that cites that `Q-N`.
- For an unvalidated assumption: the spec section that rests on it, and everything downstream of
  that section (its acceptance criteria, the plan tasks that implement them).

Uncertainty is inherited: if a task implements a requirement that depends on open `Q-5`, that task
is standing on unvalidated ground even if it never names `Q-5` directly.

### 3. Assemble the risk surface
Collect the union of every element reached in Step 2. This set is the risk surface — everything in
the design whose correctness is contingent on a source that is not yet settled.

### 4. Render the report
Group the risk surface by source, so it is clear *what* each element is waiting on:

```
Risk surface (N elements resting on M unresolved sources)

Q-5 (email provider — Open)
  ├─ FEAT-003 (entire spec — not yet written)
  ├─ Plan milestone M3 and its 4 tasks
  └─ Architecture §2 Notification component

FEAT-001 §7.2.C (assumes synchronous workspace creation — Validated: [—])
  └─ FEAT-001 §3.1.D and acceptance criterion §6.1.A
```

For each source, note what would change if it resolves unfavorably — that is the actual risk.

### 5. Use it to prioritize, and clear it over time
- The source with the largest downstream surface is where validation buys the most certainty —
  resolve it first.
- When a question moves to `Resolved`, or an assumption gains a `Validated:` value, re-run this
  procedure: those elements drop off the risk surface automatically. The surface should shrink as
  the project matures; a surface that stays large late in a project is a warning.

> The risk surface is always generated on demand from current state — never hand-maintained as a
> file, or it will drift out of sync with the questions and assumptions it summarizes.

---

## Outputs
- A risk-surface report: the elements resting on unresolved ground, grouped by source
- A prioritization signal: which source to validate next for the most certainty gained

## Checklist
- [ ] All `Open`/`Researching` questions collected as sources
- [ ] All unvalidated (§7.2 `Validated: [—]`) assumptions collected as sources
- [ ] Each source propagated downstream to its dependents (inherited uncertainty included)
- [ ] Risk surface grouped by source, with the consequence of unfavorable resolution noted
- [ ] Report generated from current state, not a stale hand-maintained copy

## Related
- Skill: `skills/trace-uncertainty.md`
- Procedure: `procedures/analyze-impact.md` (shares the downstream traversal)
- Procedure: `procedures/add-research.md` (resolving a question clears part of the surface)
