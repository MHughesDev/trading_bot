# Skill: Add a Document

## Purpose
Place any new documentation file in the correct folder, register it in the relevant index, and ensure it follows project conventions — without having to remember the folder layout by heart.

## When to Use
Any time you need to create a new doc file: ADR, spec, plan, procedure, skill, research conclusion, operations runbook, architecture narrative, or any other doc type. If you are unsure which folder the file belongs in, this skill resolves that ambiguity.

## Procedures Used
- `docs/procedures/add-doc.md` — primary procedure (Decision Table, naming, indexing, cross-linking)

Supporting procedures (for specific types):
- `docs/procedures/add-adr.md`
- `docs/procedures/add-spec.md`
- `docs/procedures/add-plan.md`
- `docs/procedures/research-ledger.md`

## Workflow

1. **Identify the document type.** Ask: is this an architectural decision, a spec, a plan, a procedure/skill, a research conclusion or open question, a runbook, or something else?

2. **Open `docs/procedures/add-doc.md`** and look up the type in the Decision Table to get: target folder, naming convention, and which index to update.

3. **For ADR, spec, or plan**, open the corresponding procedure (`add-adr.md`, `add-spec.md`, `add-plan.md`) for the detailed template and steps. **For a research conclusion or open question**, use `research-ledger.md` — rows are appended to existing tables, no new file is created.

4. **Write the file** using the appropriate template. Every file needs at minimum: `#` title, one-line Purpose, Status line.

5. **Update the index** listed in the Decision Table.

6. **Add cross-links** if the new doc supersedes or is referenced by an existing doc.

## Tips
- Research conclusions always go in `docs/research/CONCLUSIONS.md` as a new row — never in a separate file.
- Open questions always go in `docs/research/OPEN_QUESTIONS.md` as a new row.
- If a straggler file exists at the docs root (`docs/UPPERCASE_NAME.MD`), it either belongs in a subdirectory (use this skill to move it) or is superseded material (move it to `docs/archive/`).
- When in doubt about whether something is a spec vs. an ADR: specs describe *what* the system should do; ADRs record *why* a specific design choice was made.
