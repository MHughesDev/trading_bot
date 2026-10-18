---
name: skill-authoring
description: Use when a procedure you ran is worth keeping. How to write a skill the platform will admit, and what it will refuse.
---
# Skill authoring

A skill is a procedure, not a result.

## Write one only when

- You ran the same procedure three times, or
- it was hard-won (many tool calls, or an error and then a fix) and the platform verified the
  outcome, or
- a platform rejection taught you a pitfall worth naming.

## Shape

- `SKILL.md` with `name` (equal to the directory name) and `description`. The description is
  the trigger: at most 100 tokens, say when to use it.
- Body at most 500 lines.
- No symbols, dates, findings or fitted constants. A skill that names `BTC-USD` or `0.73` is a
  finding in disguise and will be refused.
- No `allowed-tools`, no hooks, no shell substitution lines. Core skills are instructions only.

## What admission checks

Static lint, unit tests, a replay on synthetic data with and without the skill, and that it
does not steal another skill's trigger. A skill that does not change the outcome is not admitted.

## Procedure skills

A *procedure* is the usual kind: instructions another session follows, not code. Declare it
with `kind: procedure` in the frontmatter and use `procedure-template.md` (in this directory)
as the shape. A procedure has exactly three sections:

- `## Steps`: ordered. Name a tool as `[tool:name]`. Only tools in the platform catalogue are
  admitted, and none that reaches the network.
- `## Evidence`: what the procedure produces that a reviewer can check (an id, a number, a file).
- `## Failure`: the mistake it was written to prevent. If you cannot name one, it is not worth
  keeping.

A procedure grants nothing. `shell`, `network`, `permissions`, `env`, `allowed-tools` and `hooks`
in the frontmatter are each refused, and the refusal cannot be suppressed.

## How to propose

Search first: `tbot skills search "<what it is for>"`. A close match becomes a patch of the
existing skill, so say what is new. Then
`tbot skills propose <dir> --project <id> --name <name> --category <c> --reason T2 --source-ref run:<id>`.
You must cite the run it came from. Check the outcome with `tbot skills proposals`: a rejection
names the step that failed. You cannot admit a skill; the platform does that after verification.
