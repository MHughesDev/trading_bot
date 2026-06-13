# Procedure: Add a Document

## Purpose
Place a new documentation file in the correct folder, register it in the relevant index, and ensure it follows the project's doc conventions.

## Trigger
Use this procedure whenever a new doc file needs to be created — ADR, spec, plan, procedure, skill, research conclusion, operation runbook, or any other doc type.

## Inputs
- **Title** — what the document covers
- **Type** — one of the types in the Decision Table below
- **Content** — the substance to put in the file

## Decision Table

| Document type | Target folder | Naming convention | Index to update |
|---|---|---|---|
| Architecture Decision Record | `docs/adr/` | `NNNN-kebab-slug.md` (next sequential number) | `docs/adr/README.md` |
| Component / data / integration spec | `docs/specs/` | `COMP-NNN-`, `DATA-NNN-`, `INTG-NNN-`, `FEAT-NNN-`, or `SYS-NNN-` | `docs/specs/README.MD` |
| Phase plan | `docs/plans/plan-sets/set-X/` | `phase-N-slug.md` | relevant set's MASTER.md |
| Atomic procedure | `docs/procedures/` | `verb-noun.md` | `docs/procedures/README.md` |
| Agent skill | `docs/skills/` | `verb-noun.md` | `docs/skills/README.md` |
| Research conclusion | `docs/research/CONCLUSIONS.md` | Append a C-NNN row to the table (never create a new file) | — (table is self-indexing) |
| Open question | `docs/research/OPEN_QUESTIONS.md` | Append an OQ-NNN row to the table (never create a new file) | — (table is self-indexing) |
| Operations runbook / playbook | `docs/operations/` | `kebab-slug.md` | `docs/operations/` (no separate index — files are self-describing) |
| Architecture narrative | `docs/architecture/` | `kebab-slug.md` | `docs/architecture.md` (add a reference section if missing) |
| Governance / audit report | `docs/governance/` | `kebab-slug.md` or `AUDIT_REPORT_YYYY-MM-DD_slug.md` | — |
| Backlog / roadmap item | `docs/backlog/` | `kebab-slug.md` | — |
| Foundation / positioning doc | `docs/foundation/` | `kebab-slug.md` | — |
| Reference table (data sources, venues, etc.) | `docs/research/` | `kebab-slug.md` | `docs/README.md` if table-of-contents-worthy |

## Steps

1. **Pick the target folder** from the Decision Table above.

2. **Derive the filename.**
   - ADRs: check `docs/adr/README.md` for the next sequence number.
   - Specs: check `docs/specs/README.MD` for the next sequence number in the relevant prefix family.
   - All others: use `kebab-case` lowercase, no spaces, `.md` extension.

3. **Write the file.** Use the template for the type:
   - ADR → `docs/procedures/add-adr.md` template
   - Spec → `docs/procedures/add-spec.md` template
   - Plan → `docs/procedures/add-plan.md` template
   - Research conclusion or open question → `docs/procedures/research-ledger.md` (rows appended to existing tables, no new file created)
   - Procedure/Skill → the authoring template in `docs/procedures/README.md` / `docs/skills/README.md`
   - All other types → free-form, but include at minimum: a `#` title, a one-line **Purpose** sentence, and a **Status** line (`Draft`, `Active`, or `Superseded`).

4. **Register in the index.** Find the index file listed in the Decision Table row and add a one-line entry: `| [Title](./filename.md) | One-line description |`.

5. **Cross-link if needed.** If the document references or supersedes an existing doc, add a `See also:` or `Supersedes:` note at the top of both files.

6. **Verify.** Confirm the file exists at the target path and the index row points to it correctly.

## Outputs
- New `.md` file at the correct path
- One new row in the relevant index

## Checklist
- [ ] Target folder matches the Decision Table
- [ ] Filename follows the naming convention
- [ ] File has at minimum a title, Purpose, and Status
- [ ] Index updated with a one-line entry
- [ ] Cross-links added if the doc supersedes or references another doc

## Related
- Skill: `docs/skills/add-doc.md`
- Procedure: `docs/procedures/add-adr.md`
- Procedure: `docs/procedures/add-spec.md`
- Procedure: `docs/procedures/add-plan.md`
- Procedure: `docs/procedures/research-ledger.md`
