# ADR-0036: Context assembly is retrieval over a run store, not accumulation in a buffer

**Status:** Proposed
**Date:** 2026-09-14
**Spec:** `docs/specs/AGENT-005-context-assembly.md` Part II (§13–§17)
**Amends:** ADR-0034 §2.2 (the fixed layout with per-section budget shares) and HARNESS_CONFORMANCE item 10
**Related:** AGENT-007 (the same retrieval discipline, across runs), AGENT-008 §2 (the working state is the query), AGENT-009 (the policy is measured)

## Context

ADR-0034 fixed the arithmetic of context assembly: one budget, honestly counted, with the
schemas and the charter reserved before the body is sized. That was correct and it is not
enough. The design underneath it is still the one the harness started with — a chronological
list of blocks with hard-coded integer priorities, compacted when full by dedupe, then
dropping low-priority blocks, then cutting the remaining text in the middle.

Three properties of that design put a ceiling on the agent that no amount of better
arithmetic lifts:

1. **It is chronological, not relevant.** A block's place in the prompt is decided by when it
   arrived and by a constant written in the source (255 charter, 250 findings, 215 notices,
   100 raw results). The same ordering is applied to a one-step lookup and a forty-step
   sweep. Nothing asks whether a block has anything to do with what the model is about to do.
2. **It degrades lexically.** `truncate_middle` cuts a rendered string at a byte offset. A
   500-row JSON table becomes half a JSON table — syntactically broken and semantically
   worthless — when the useful degradation was five rows and a row count.
3. **It is unmeasured.** The priorities, the shares and the trigger are asserted in code. No
   signal anywhere says which blocks the model actually used, so the numbers cannot improve
   and have not.

The platform's own doctrine says a number that matters should be measured, and that rigor
belongs in the mechanism rather than the model. Context assembly is the largest remaining
place where the platform violates both.

## Decision

Context assembly becomes **retrieval over a run-scoped store**, rendered semantically, under a
policy that is versioned and evaluated.

1. **A run context store.** Every observation a run produces — tool results, findings,
   corrections, plan changes, retrieved memory — is an addressable, typed record in a store,
   not a line in a buffer. Records are never deleted during a run.
2. **Assembly is a query.** Before each call the harness scores the store against the current
   step's need (the working state: goal, active plan step, hypothesis, last error) and selects
   what to render. The prompt is the answer to "what does this step need", not "what happened
   recently".
3. **Degradation is semantic.** Every record declares how to shrink: which rows, which
   columns, which precision, in what order. A record is never cut at a byte offset.
4. **A manifest is always present.** A small, always-rendered index of everything the run
   holds, so removing content never removes the knowledge that the content exists. The model
   can always see what it could ask for.
5. **The model can pull.** `expand(handle)` and `focus(topic)` let the model request context
   rather than only receive it, within the same budget.
6. **Budget follows value.** A step about to spend a trial gets more context than a step doing
   a lookup.
7. **The policy is an object.** Layout, weights, degradation order and trigger are a
   versioned `ContextPolicy`, recorded on every run, A/B tested by the eval suite.
8. **Utility is measured.** Every rendered record is attributed against what the model did
   next, and the weights are learned from that rather than declared.

## Consequences

- The fixed layout and the per-section shares from ADR-0034 §2.2 become the *default policy*
  rather than the mechanism. They stay as `ContextPolicy v1` so nothing regresses on day one,
  and they become the baseline the retrieval policy must beat on the eval suite before it is
  promoted.
- The budget arithmetic from ADR-0034 is unchanged and now governs a selection rather than a
  truncation. Reservation, the 25% floor and the calibrated estimator all still apply.
- Small models gain the most: a 7B's entire problem is that its window cannot hold the run, and
  a relevance-selected 3,500-token body is a categorically different thing from the most recent
  3,500 tokens. This is the change that makes the 7B-to-frontier range a real range rather than
  a graceful degradation.
- Cost falls on every tier, because the dominant waste — re-rendering observations the model
  will not use, and re-sending observations it has already seen unchanged — is addressed at the
  source rather than at the truncation step.
- Risk: retrieval can omit something the model needed. The manifest (4) is the mitigation and
  it is why the manifest is not optional; a retrieval miss becomes a visible, recoverable
  `expand` rather than a silent absence.

## Alternatives considered

- **Better priorities.** Tuning the constants by hand. Rejected: it is the same design, it does
  not measure, and it cannot be right for both a lookup and a sweep simultaneously.
- **Model-side summarisation of everything.** Ask the model to compress its own context each
  step. Rejected: it spends the scarce resource (model calls on a small tier) to save the
  abundant one, it is non-deterministic, and AGENT-005 P5 puts rigor in the tool.
- **A vector index over the run.** Embedding every record. Deferred rather than rejected: the
  lexical scorer plus structural signals (same instrument, same step, cited id) is most of the
  value at none of the operational cost, and §16's attribution will show whether embeddings
  are worth adding. The interface is the same either way.
