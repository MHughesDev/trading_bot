# research/

Research briefs, technology evaluations, competitive analyses, and findings from discovery work. This folder feeds into plans, specs, and ADRs — it is the evidence layer.

> To add a new research brief, follow the skill: `skills/create-research-brief.md` and procedure: `procedures/add-research.md`.

---

## What Belongs Here

- Technology comparison briefs ("PostgreSQL vs. DynamoDB for our use case")
- Prior art surveys ("how do other systems solve X?")
- API and integration evaluations
- Performance benchmark summaries
- User research findings (if applicable)
- Domain background research

## What Does Not Belong Here

- Decisions — record those in `adr/`
- Plans that act on the research — record those in `plans/`
- Specifications derived from research — record those in `specs/`

---

## Conventions

- File names: `kebab-case.md` — be descriptive: `postgres-vs-dynamodb-write-throughput.md`
- Each research brief should have:
  - **Question** — what were you trying to find out?
  - **Method** — how did you research it? (docs, benchmarks, prototypes, interviews)
  - **Findings** — what did you learn?
  - **Recommendation** — what does this research suggest? (not a decision — a recommendation)
  - **References** — sources
- Research files are not modified once written. Add a new file if follow-up research changes the picture, and cross-reference the earlier file.

---

## Index

| File | Question | Date |
|------|----------|------|
| [example-auth-provider-comparison.md](./example-auth-provider-comparison.md) | Which auth provider fits a B2B SaaS with multi-tenancy and a 2-person team? | 2024-03-01 |
