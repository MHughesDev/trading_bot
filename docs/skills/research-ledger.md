# Skill: Research Ledger

## Purpose
Maintain the project research ledger — recording decided conclusions in `research/CONCLUSIONS.md`
and tracking unresolved questions in `research/OPEN_QUESTIONS.md`. These two files are the
authoritative source of design intent for this project.

## When to Use
- A design question has been answered and the decision needs to be recorded
- Research surfaces a new unresolved question, conflict, or dependency
- An open question is resolved and its row needs to be closed
- Before writing a spec, ADR, or plan — verify that the relevant questions are already concluded
  (if not, research first and record conclusions before designing)
- Any time the user asks "what do we know about X?" or "is X decided?"

---

## Procedures Used
- `procedures/research-ledger.md` — atomic steps for adding conclusions, adding open questions,
  and closing open questions
- Full operating rules (source discipline, append-only policy, ID management, anti-patterns):
  `research/SKILL.md`

---

## Workflow

### 1. Read before writing
Before adding any conclusion or closing any question:
- Read `research/CONCLUSIONS.md` for the highest C-NNN and any existing conclusions on the topic.
- Read `research/OPEN_QUESTIONS.md` for the highest OQ-NNN and the OQ rows in scope.
- Check for duplicate IDs accidentally introduced in earlier sessions.

### 2. Research with source discipline
For any conclusion that depends on external facts:
- Use primary sources (official vendor/API docs, standards bodies, regulatory notices, source code).
- Use repo files, ADRs, specs, or prior conclusions for internal architecture facts.
- Note source names and URLs — a future agent must be able to verify the claim.
- If sources conflict, do not force a conclusion. Add an open question describing the conflict.

### 3. Record conclusions
Apply `procedures/research-ledger.md` Sub-Procedure A.
- State a decision, not a question.
- Include `Closes OQ-NNN.` when the conclusion resolves an open question.

### 4. Immediately close resolved OQ rows
Apply `procedures/research-ledger.md` Sub-Procedure C.
- Do not batch closure for later — the ledger must reflect current state after each question.

### 5. Immediately add newly discovered open questions
Apply `procedures/research-ledger.md` Sub-Procedure B.
- If research resolves one question but reveals another, close the original and add the new one.
- Do not leave uncertainty only in chat or in a conclusion's rationale.

### 6. Scan for hygiene before finishing
- Duplicate IDs accidentally introduced.
- Closed OQs pointing at the wrong conclusion.
- Conclusions without source-backed rationale.
- Research findings that should have become open questions but didn't.

---

## Tips

- The ledger is append-only: do not delete rows, do not renumber IDs.
- A conclusion without a source is just an opinion — include the basis or do not add it.
- "It depends" is not a conclusion — either decide or open a question.
- Research done for a spec or ADR should also produce ledger rows; one does not replace the other.
- When writing a spec or ADR and a required conclusion is missing, pause and add it to the ledger
  first. Specs written ahead of their conclusions are hard to verify later.
