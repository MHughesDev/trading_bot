# Procedure: Explain Provenance

## Purpose
Given any design element, walk the traceability graph *upward* to produce the causal chain that
justifies its existence — terminating at the artifact's problem statement or a success condition.
Every line in the design should be able to answer "why do I exist?"

This is the inverse of `analyze-impact.md`: impact walks *down* (what depends on this?), provenance
walks *up* (what reasoning produced this?).

## Trigger
Use this procedure when:
- Someone proposes deleting or changing an element and you need to know what reasoning is at stake
- A reviewer asks "why is this requirement / task / decision here?"
- Auditing whether every element is actually justified (provenance that reaches the artifact)
- Onboarding someone to a design — provenance explains intent, not just structure

## Inputs
- The **target element**: the ID to explain (`FEAT-001 §3.1.C`, a plan task, `ADR-0001`, etc.)
- Read access to the whole `docs/` tree

---

## Steps

### 1. Name the target element
State its ID. Provenance can be produced for any element that carries or sits under an ID:
a requirement, an acceptance criterion, a plan task, a spec, an ADR, an open question.

### 2. Walk one link upward
Follow the element's declared upstream link toward its justification. The upward edges are:

| From | Walks up to |
|------|-------------|
| A plan task | the spec acceptance criterion it traces to (`→ SPEC-ID §6.1.x`) |
| An acceptance criterion (`§6.1.x`) | the requirement it verifies (`§3.x`) |
| A requirement (`§3.x`) | the spec goal it serves (`§2.1`) and the `SC-N` behind it |
| A spec (`§1.3 Related`) | the ADRs, research, and plan that informed it |
| A spec goal / milestone | the artifact success condition (`SC-N`) it advances |
| An ADR | the research and open question (`Q-N`) that drove the decision |
| `SC-N` / `FM-N` | the artifact's problem statement (the root) |

### 3. Continue to the root
Repeat Step 2 on each element reached until you arrive at the artifact — a problem statement,
success condition, or failure mode. That root is where provenance terminates: the reason the
whole project exists.

### 4. Emit the narrative
Produce two forms:
- **Chain** — a compact arrow trail: `task T-3 → FEAT-001 §6.1.C → §3.1.C → §2.1.C → SC-3 → problem`
- **Prose** — the same chain told as a sentence: *"This task exists to satisfy acceptance criterion
  §6.1.C, which verifies requirement §3.1.C (workspace isolation), which serves goal §2.1.C, which
  advances success condition SC-3, which addresses the artifact's core problem: customers must not
  see each other's data. Informed by research R-1, constrained by ADR-0001."*

### 5. Flag broken provenance
If the upward walk dead-ends before reaching the artifact — a requirement that serves no goal, a
spec that advances no `SC-N`, a task that traces to nothing — the element has **broken provenance**.
That is a signal, not a formatting error: the element may be gold-plating (work nobody asked for)
or a missing link. Report it; do not invent a justification to paper over it.

---

## Outputs
- A provenance chain (compact) and narrative (prose) for the target element
- A flag if provenance is broken (the chain does not reach the artifact)

## Checklist
- [ ] Target named at a precise ID
- [ ] Walked upward link by link to the artifact root (or to a documented dead-end)
- [ ] Both chain and prose forms produced
- [ ] Broken provenance flagged, not fabricated

## Related
- Skill: `skills/explain-provenance.md`
- Procedure: `procedures/analyze-impact.md` (the inverse, downstream walk)
- Procedure: `procedures/verify-traceability.md` (broken provenance overlaps with orphan detection)
