# Skill: Trace Uncertainty

## Purpose
Show how much of the design rests on unresolved ground by propagating uncertainty from its sources
(open questions, unvalidated assumptions) downstream into a risk surface — so the design's fragile
parts are visible, and validation effort goes where it buys the most certainty.

## When to Use
- When deciding what to research or validate next
- Before committing to or executing a plan, to gauge how much depends on unsettled questions
- After resolving a question or validating an assumption, to recompute what is now solid
- When asked "how confident are we?" — the risk surface is the honest answer

---

## Procedures Used
- `procedures/trace-uncertainty.md`

---

## Workflow

### 1. Generate it fresh — never store it
The risk surface is a function of the current open questions and assumptions. Compute it on demand;
do not write it to a file, or it will quietly disagree with reality.

### 2. Distinguish "resolves" from "is known"
`verify-traceability` confirms a reference points at something real. This skill asks a different
question: is that something actually *settled*, or is it a guess we haven't tested? An element can
pass every traceability check and still sit squarely on the risk surface.

### 3. Read the surface as a validation priority queue
The source with the biggest downstream footprint is the highest-leverage thing to nail down — it
collapses the most uncertainty per unit of effort. Use the surface to choose the next research
brief or validation task, not just to describe risk.

### 4. Watch the trend, not just the snapshot
A risk surface is expected to be large early and shrink as questions resolve and assumptions get
validated. The warning sign is a surface that stays large late in a project — that means the design
is being built on ground that was never firmed up.

---

## Tips

- The two source types clear differently: an open question clears when it moves to `Resolved`
  (via a research brief and usually an ADR); an assumption clears when it gains a `Validated:`
  value (evidence, the same idea as acceptance-criterion verification).
- Inherited uncertainty is the subtle part: a task can be on the risk surface without ever naming
  the question it depends on. Trust the downstream walk, not just the explicit citations.
- Pair with `skills/analyze-impact.md`: impact tells you the blast radius of *changing* a thing;
  uncertainty tells you the blast radius of a thing turning out to be *wrong*.
