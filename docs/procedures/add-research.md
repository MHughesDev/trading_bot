# Procedure: Add Research Brief

## Purpose
Create a focused research brief that documents a specific question, the findings, and a recommendation — providing an evidence base for future plans, specs, and ADRs.

## Trigger
Use this procedure when:
- Evaluating a technology, library, or service for adoption
- Investigating how other systems solve a problem you face
- Gathering domain background needed to design a feature
- Benchmarking or validating a technical assumption

## Inputs
Before starting, define:
- The specific **question** being researched (one question per brief)
- The **method** to be used — start with internet research into prevailing practice and existing
  building blocks (APIs, services, libraries) before forming any opinion
- The **scope boundary** — what are you explicitly not researching here?

> Per the Research-First Protocol in `AGENT.md`, internet research is mandatory before designing.
> If you cannot access the internet, stop and tell the user — do not substitute assumptions.

---

## Steps

### 1. Create the research file
1. Choose a descriptive file name in kebab-case that reflects the question, not the answer.
   - Good: `postgres-vs-dynamodb-write-throughput.md`
   - Good: `oauth2-provider-comparison.md`
   - Avoid: `database-research.md` (too vague)
2. Create the file at: `research/your-file-name.md`

### 2. Fill in the research brief template

```markdown
# [Research Question as a Title]

**Date:** YYYY-MM-DD
**Researcher:** [name or "Agent"]
**Status:** In Progress | Complete

## Question

[State the specific question this brief answers. One question only.]

## Scope

**In scope:**
- [What this research covers]

**Out of scope:**
- [What this research explicitly does not cover — prevents scope creep]

## Method

[How was this researched? Examples:
- Read official documentation for X, Y, Z
- Ran a benchmark using [tool] with [parameters]
- Reviewed [N] case studies from [source]
- Built a proof-of-concept to test [hypothesis]]

## Findings

[What did the research reveal? Use sub-headings for each option or dimension compared.
Be factual here — save interpretation for the Recommendation section.]

### [Option A / Topic A]
- Finding 1
- Finding 2

### [Option B / Topic B]
- Finding 1
- Finding 2

## Comparison Matrix (if applicable)

| Criterion | Option A | Option B | Option C |
|-----------|----------|----------|----------|
| [criterion] | [value] | [value] | [value] |

## Recommendation

[Based on the findings, what does this research suggest? 
This is a recommendation, not a decision. Decisions belong in `adr/`.]

## Open Questions

- [What remains unknown or needs further investigation?]

## References

- [Source 1](url)
- [Source 2](url)
```

### 3. Conduct the research
1. Fill in findings as you research — do not wait until the end.
2. If the question evolves during research, update the **Question** and **Scope** sections and note the change.
3. Mark status as `Complete` when findings and recommendation are written.

### 4. Update the research index
1. Open `research/README.md`.
2. Add a row to the Index table:
   ```
   | [file-name.md](./file-name.md) | [One-sentence question] | YYYY-MM-DD |
   ```

### 5. Update the open-questions register
- If this research was triggered by a trade-off in `docs/open-questions.md`, move that entry to
  `Resolved`, record the answer, and link this brief as the evidence.
- If the research surfaced new forks, add them to `docs/open-questions.md` as `Open`.

### 6. Link forward (if applicable)
- If this research was requested to inform a specific ADR or spec, add a note at the top of that ADR/spec:
  ```
  > See research: [research/file-name.md](../research/file-name.md)
  ```

---

## Outputs
- A new file: `research/descriptive-name.md`
- Updated index in `research/README.md`

## Checklist
- [ ] File name is descriptive of the question, not the answer
- [ ] Question is specific and singular
- [ ] Scope explicitly defines what is out of scope
- [ ] Findings are factual (not opinionated)
- [ ] Recommendation is present and clearly labeled as a recommendation
- [ ] Status set to `Complete`
- [ ] Index row added to `research/README.md`
- [ ] Relevant ADRs or specs linked back to this research

## Related
- Skill: `skills/create-research-brief.md`
- Procedure: `procedures/add-adr.md` (research often leads to an ADR)
- Procedure: `procedures/add-spec.md` (research informs specs)
