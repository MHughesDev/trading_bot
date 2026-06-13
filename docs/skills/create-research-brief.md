# Skill: Create Research Brief

## Purpose
Produce a focused, evidence-based research brief that answers a specific question and provides a recommendation — without making the decision.

## When to Use
- **Always, before writing any spec, architecture entry, or decision** (Research-First Protocol, `AGENT.md` §3)
- You need to evaluate two or more technology or design options
- A significant unknown must be resolved before planning or specifying can proceed
- A failure mode from the artifact points at a technical risk that needs investigation
- Prior art needs to be surveyed before designing a solution

---

## Procedures Used
- `procedures/add-research.md`

---

## Workflow

### 1. Define the question sharply
Good research starts with a sharp question. Before creating any file:
- State the question in a single sentence.
- Verify it is narrow enough to answer in one brief (if not, split it).
- Confirm the question is answerable with available information.

Examples of sharp questions:
- "What are the write throughput limits of DynamoDB single-table design vs. PostgreSQL for our expected load?"
- "Which OAuth 2.0 providers support PKCE and also have a Go SDK?"
- "How do other B2B SaaS products handle multi-tenant row-level security?"

### 2. Define scope before researching
Write out explicitly what is **out of scope** before gathering information. This prevents the brief from becoming a general survey.

### 3. Execute `procedures/add-research.md`
Follow the procedure. As you research:
- Record findings factually — keep opinions in the Recommendation section
- Note sources as you go — reconstructing them later wastes time
- If the question changes as you research, update the Question and Scope sections and note the revision

### 4. Write the recommendation
The recommendation section should:
- State which option is suggested and why (based on the findings)
- Acknowledge the trade-offs of the recommendation
- Identify any conditions under which a different option would be better
- Explicitly *not* make the decision — that belongs in an ADR

### 5. Update the open-questions register and flag follow-on actions
After completing the brief:
- Move any `docs/open-questions.md` entry this brief resolved to `Resolved`, citing this brief.
- Add any new forks the research surfaced as `Open` entries.
- Does this research warrant an ADR? → run `skills/create-adr.md`
- Does this research change the project plan? → update `docs/plans/plan-sets/`

---

## Tips

- One question per brief. Two questions produce a brief that answers neither well.
- "Findings" are facts; "Recommendation" is interpretation. Keep them separate.
- A brief that says "it depends" in the Recommendation section must specify what it depends *on* — vague recommendations are not useful.
- Research briefs are permanent artifacts. Do not modify a completed brief if new information emerges — write a new brief and cross-reference it.
