# ADR-0028: Skill registry, platform-side admission, and the glossary as a registry view

**Status:** Proposed
**Date:** 2026-09-11
**Deciders:** Mason Hughes (with Claude)

## Context

The user wants the agent to create its own skills, and wants skills used for
every user. Every new skill must appear in the glossary of skills (BS-007 D-06).
The research is clear on three points:
- skills the agent writes speculatively make it *worse* (SkillsBench: −8 to −11.5
  points);
- skills distilled from verified experience and checked by an outside verifier
  beat human-written ones;
- large libraries of look-alike skills degrade selection, and shared skill
  libraries are a real prompt-injection surface.

## Decision

1. **The agent proposes; the platform admits.** Drafts live in the workspace.
   Verification (`skill_verify` job):
   - static scan and unit tests;
   - replay on synthetic instruments, with vs without the skill, which must show
     equal success at lower cost;
   - trigger and coexistence evals;
   - quant tests: PIT truncation, noise-null, leakage trap;
   - an isolated Reviewer sub-agent.

   Admission is a platform-side write. `skills.admit` is never granted to agent
   tokens.
2. **The glossary is a view over the registry.** Admission, patch, deprecation and
   quarantine write the registry and glossary rows in one transaction. An admitted
   skill can't be absent from the glossary.
3. **Materialisation at every session start, for every user:**
   - core and pinned global skills always;
   - up to about 40 descriptions in total, in deterministic order, frozen per
     project version;
   - mounted read-only.
   The rest is searchable.
4. **Scopes:** core → global → user → project. Promotion to global requires:
   - evals on other projects' tasks;
   - sanitisation;
   - owner consent;
   - a security scan;
   - a human reviewer who isn't the author;
   - no untrusted-input taint.
5. **Skills hold procedures, never findings.** No symbols, dates or tuned values;
   no capability grants.

## Rationale

This design matches the evidence on when self-made skills help, and it enforces
the user's two requirements mechanically rather than by prompt. Keeping skill files
byte-stable (statistics live in the registry) also protects prompt caching.

## Consequences

- New tables `skills`, `skill_versions`, `skill_runs`, `skill_evals`,
  `skill_votes`; a `skill_verify` job kind; a Skills UI page with a promotion queue.
- Verification and A/B evaluation cost real tokens. They are bounded by per-skill
  budgets and sequential paired designs.
- Core skills are seeded from the BS-007 quant knowledge and research standard.

## Alternatives Considered

- **Agent writes directly to `.claude/skills/`.** No verification; drifts; unsafe.
- **Human-curated skills only.** Loses the measured gains of verified,
  experience-distilled skills.
- **Load every skill.** Selection accuracy collapses past about 90 skills, and
  tokens double.

## References

- [AGENT-003](../specs/AGENT-003-knowledge-memory-skills.md)
- BS-007 [13_SKILLS](../BRAINSTORM/BS-007_QUANT_RESEARCH_AGENT/13_SKILLS.MD)
