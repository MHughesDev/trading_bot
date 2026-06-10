# Procedure: Research Ledger

## Purpose
Add conclusions or open questions to the project research ledger — the two files `research/CONCLUSIONS.md` and `research/OPEN_QUESTIONS.md` — which are the authoritative source of design intent for this project.

## Trigger
Use this procedure when:
- A design question has been answered and the decision must be recorded
- A new unresolved question, dependency, risk, or contradiction is discovered
- An existing open question is resolved and needs to be closed
- Research uncovers a conflict between sources that must remain visible until resolved

> The full operating rules for the research ledger are in `research/SKILL.md`. Read it before
> starting a research batch. This procedure covers the mechanical steps only.

---

## Sub-Procedures

### A. Add a Conclusion Row

**When:** A decision has been made or research has resolved an open question.

**Steps:**

1. Open `research/CONCLUSIONS.md`.
2. Find the highest existing `C-NNN` ID and add one for the new row.
3. Write a **decision**, not a question. State what is now true or chosen.
   - Include `Closes OQ-NNN.` when this conclusion resolves an open question.
4. Write a **source-backed rationale**: name the vendor docs, ADR/spec IDs, prior conclusion IDs, or
   explicit user direction that supports the decision. Do not write vague rationales.
5. Append the row to the end of the table using this format:
   ```markdown
   | C-XXX | Topic | **Decision summary.** Supporting details. | Source-backed rationale. Include source names, repo refs, URLs, or conclusion IDs. | YYYY-MM-DD |
   ```
6. If this conclusion closes an open question, immediately execute Sub-Procedure C below.

**Checklist before saving:**
- [ ] ID is unique (next after highest existing)
- [ ] States a concrete decision (not a question)
- [ ] Rationale names the source basis
- [ ] Any closed OQ is explicitly referenced with `Closes OQ-NNN.`

---

### B. Add an Open Question Row

**When:** Research uncovers something unresolved — a dependency, conflict, missing external fact,
or decision that requires user input or future implementation context.

**Steps:**

1. Open `research/OPEN_QUESTIONS.md`.
2. Find the highest existing `OQ-NNN` ID and add one for the new row.
3. Write a **short, searchable topic** — specific enough that a later agent can research it cold.
4. Write `Options / Sub-questions` as concrete sub-questions or candidate options, not vague concern.
5. In `Notes`, explain what triggered the question: source conflict, missing API field, repo gap,
   user decision needed, or external verification required.
6. Set `Blocking` to the most relevant system area.
7. Append the row using this format:
   ```markdown
   | OQ-XXX | Topic | Open | Specific sub-questions and candidate options. | Why this remains unresolved, plus source/context. | Blocking area |
   ```

**Checklist before saving:**
- [ ] ID is unique (next after highest existing)
- [ ] Topic is short and searchable
- [ ] Options / Sub-questions are concrete, not vague
- [ ] Notes explain what triggered the question
- [ ] Blocking area is named

---

### C. Close an Open Question Row

**When:** A conclusion (Sub-Procedure A) resolves an open question.

**Steps:**

1. Open `research/OPEN_QUESTIONS.md` and locate the row by its `OQ-NNN` ID.
2. Change `Status` from `Open` or `In Progress` to `Closed (see C-NNN)`, citing the conclusion ID.
3. Replace the `Options / Sub-questions` text with the resolved answer in compact form.
4. Update `Notes` with the closure date and a short research note.
5. Leave the `Blocking` column unchanged or set to `—` if no longer relevant.
6. Do **not** delete the row.

**Example result:**
```markdown
| OQ-042 | Paper trading fill simulator — exact design per asset class | Closed (see C-086) | Resolved answer. | Closed 2026-06-10. Sources: venue docs + C-070. | Paper trading |
```

---

## Anti-Patterns

- Closing an OQ without adding a corresponding conclusion.
- Adding a conclusion with no source basis (vague rationale like "research showed this").
- Reusing an existing C-NNN or OQ-NNN ID.
- Hiding unresolved concerns inside a conclusion's rationale text.
- Batching OQ cleanup for later — update status immediately after each resolved question.
- Adding implementation changes to a research-only ledger update.

---

## Outputs
- Updated `research/CONCLUSIONS.md` (new rows appended)
- Updated `research/OPEN_QUESTIONS.md` (new rows and/or closed rows)

## Related
- Full ledger operating rules: `research/SKILL.md`
- Skill: `skills/research-ledger.md`
- Procedure: `procedures/add-adr.md` (research often leads to an ADR)
- Procedure: `procedures/add-spec.md` (research informs specs)
