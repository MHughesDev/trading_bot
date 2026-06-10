# Research Ledger Discipline

Use this skill whenever researching architecture decisions for this project or updating the research ledger files:

- `research/CONCLUSIONS.md`
- `research/OPEN_QUESTIONS.md`

The goal is to preserve the line of thought for future agents. Conclusions should explain what is now decided. Open questions should capture anything discovered during research that is still unresolved, blocked, risky, or dependent on a later implementation choice.

## Core Rules

1. Keep `OPEN_QUESTIONS.md` active while researching.
   - Before starting a research batch, read the target open-question rows.
   - While researching, if a new uncertainty, dependency, contradiction, or follow-up is found, add it to `OPEN_QUESTIONS.md`.
   - Do not leave important uncertainty only in chat, notes, or a conclusion rationale.
   - If the research closes one question but reveals another, close the original and add the new question as a new row.
   - When an open question has been dealt with, update its row in `OPEN_QUESTIONS.md` immediately before moving on to the next question.
   - Before moving on, also add any new open questions triggered by the research that just took place.
   - Do not batch OQ cleanup for later. The ledger must reflect the current state after each resolved question or tightly related mini-batch.

2. Conclusions must always have sources.
   - Every new row in `CONCLUSIONS.md` must cite the basis for the decision in the `Rationale` column.
   - Prefer direct primary sources for current external facts: official API docs, vendor docs, standards, regulatory notices, source code, ADRs, specs, or repo files.
   - If the conclusion is based on user direction or an existing repo decision, say that explicitly and reference the relevant conclusion, ADR, spec, file, or conversation instruction.
   - For web research, include enough source detail in the rationale for a future agent to verify the claim later. Do not write vague rationales like "research showed this."

3. Research is append-only unless correcting status.
   - Add new conclusion rows; do not delete old conclusion rows.
   - Add new open-question rows; do not delete old open-question rows.
   - It is acceptable to update an open question's `Status`, `Options / Sub-questions`, and `Notes` when closing or refining it.

4. Keep IDs trustworthy.
   - Before adding a row, scan the file for the highest existing ID and the target topic.
   - New rows must use the next numeric ID after the highest existing ID in that file, not merely the next ID in the local section being edited.
   - Do not reuse an existing ID, even if older rows already contain historical duplicates.
   - If duplicate IDs already exist, do not make the problem worse. Add the next unique ID and mention the duplicate only if relevant.

## Adding A Conclusion Row

Use the table format already present in `research/CONCLUSIONS.md`:

```markdown
| C-XXX | Topic | **Decision summary.** Supporting decision details. | Source-backed rationale. Include source names, repo references, URLs, or conclusion IDs. | YYYY-MM-DD |
```

Procedure:

1. Read nearby recent conclusions to match tone and granularity.
2. Pick the next `C-XXX` after the highest existing conclusion ID.
3. Write a decision, not a question.
4. Include the affected open-question ID in the conclusion text when it closes one, for example: `Closes OQ-042.`
5. In `Rationale`, include sources:
   - Web source names and URLs when using current external facts.
   - Repo files, ADR/spec IDs, or prior conclusion IDs when using internal architecture context.
   - User direction when the decision is explicitly user-defined.
6. Keep the row self-contained enough that a future agent can understand the decision without reading the whole chat.

Checklist before saving:

- The conclusion has a unique `C-XXX`.
- The conclusion states a concrete decision.
- The rationale names the source basis.
- Any closed OQ is explicitly referenced.
- Any new uncertainty discovered was added to `OPEN_QUESTIONS.md`.

## Closing An Open Question Row

Use the table format already present in `research/OPEN_QUESTIONS.md`.

When a conclusion closes an open question:

1. Change `Status` from `Open` or `In Progress` to `Closed (see C-XXX)`.
2. Replace the old options text with the resolved answer in compact form.
3. Update `Notes` with a closure date and short research note.
4. Leave the `Blocking` column as the affected area or `—` if no longer relevant.
5. Do not delete the row.

Example:

```markdown
| OQ-042 | Paper trading fill simulator — exact design per asset class | Closed (see C-086) | Resolved answer here. | Closed 2026-06-10. Sources: venue docs + C-070. | Paper trading |
```

## Adding A New Open Question Row

Add a new row whenever research uncovers unresolved work that future agents need to pick up.

Use this format:

```markdown
| OQ-XXX | Topic | Open | Specific sub-questions and candidate options. | Why this remains unresolved, plus any source/context that triggered it. | Blocking area |
```

Procedure:

1. Pick the next `OQ-XXX` after the highest existing open-question ID.
2. Make the topic short and searchable.
3. Write the `Options / Sub-questions` field as concrete questions, not vague concern.
4. In `Notes`, explain what triggered the question: source conflict, missing API field, repo gap, user decision needed, implementation unknown, or external verification needed.
5. Set `Blocking` to the most relevant system area: Dashboard, Strategy system, Collector design, Paper trading, Venue onboarding, UI layout, Infrastructure, etc.

Good open questions are specific enough that a later agent can research or decide them without reconstructing the whole prior thread.

## Research Batch Workflow

For each research batch:

1. Identify the OQ IDs in scope.
2. Read existing conclusions that constrain the batch.
3. Research with source discipline:
   - Use primary sources for current external facts.
   - Use repo files/ADRs/specs for internal facts.
   - Note source dates when the fact may change.
4. While researching, immediately add newly discovered unresolved items to `OPEN_QUESTIONS.md`.
5. Append one or more source-backed conclusions to `CONCLUSIONS.md`.
6. Immediately mark the resolved OQ rows as `Closed (see C-XXX)` before starting the next OQ or mini-batch.
7. Immediately add any new OQ rows that were triggered by the just-completed research before moving on.
8. Re-scan both files for:
   - Duplicate IDs accidentally introduced.
   - Closed OQs pointing at the wrong conclusion.
   - Conclusions without source-backed rationale.
   - Research findings that should have become open questions.
9. Commit only the relevant research files unless the user asked for broader changes.

## Source Quality Bar

Use the strongest available source:

1. Official vendor/API documentation.
2. Regulatory or standards body source.
3. Repo code, ADRs, specs, or tests.
4. Reputable technical documentation or engineering articles.
5. Secondary summaries only when clearly marked as secondary.

If sources disagree, do not force a conclusion. Add or keep an open question describing the conflict and what must be verified.

## Anti-Patterns

- Closing an OQ without adding a corresponding conclusion.
- Adding a conclusion with no source basis.
- Hiding unresolved concerns in a conclusion rationale.
- Reusing an existing ID.
- Treating a stale open question as closed without updating its status.
- Mixing implementation changes into a research-only batch unless explicitly requested.
