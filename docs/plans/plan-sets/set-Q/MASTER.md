# Research standard — Set Q

**Completion: 33% left (11 / 20 tasks). 11 done, 5 partial, 1 not started, 3 blocked upstream, 0 blocked on a human.** Re-counted 2026-10-05 (track VERIFY2) from the per-task markers; the line read 5 / 20 before.

**Status:** PLANNED (2026-10-04, track F-RESERVED) — this file was the reserved letter in [ROADMAP §5](../../ROADMAP.md#5-set-succession-what-the-reserved-letters-still-mean) until today.
**Stage:** [2](../../ROADMAP.md#4-the-stages) — complete the evidence surface
**Derived from:** [BS-007 11_EVALUATION_STANDARD](../../../BRAINSTORM/BS-007_QUANT_RESEARCH_AGENT/11_EVALUATION_STANDARD.MD) §6–§9, ROADMAP §5, OPEN-TASKS Part C3, Set S Phase 3 (the catalogue), ADR-S3-06
**Requirement IDs:** EV-02, EV-03, EV-06, EV-07, EV-08, and the T-rules T9, T12, T15, T18.
**Migration range:** none planned (EV- extensions are tools, gates and reports); 0210 is available if a table is needed.
**Scope class:** End-state architecture.

---

## 1. Why this set exists

The research standard is mostly built: the Null Library, the funnel, deflated Sharpe, PBO, the family-wise test, the leakage battery and a dossier. What the standard
still asks for is a handful of tools an agent needs *before* it spends trials (can this edge survive size at all, what does a dumb rule earn on the same data, how much should this
single result be shrunk toward its family) and G3 rules that consume them. This set is deliberately small.

## 2. Scope

| Area | Today | This set delivers |
|---|---|---|
| §8 evaluation tools | three of twelve exist under another name | the rest: posterior alpha, capacity, benchmarks, decay, the alpha-chain audit, the robustness suite, decomposition, collider check |
| G3 | DSR, PBO, null p-value | plus the theme posterior and the campaign family test (EV-07) |
| Dossier | built | carries a specification count, expected haircut and posterior |

### Out of scope

- The honesty arithmetic, the leakage battery, the dossier shell: Set S Phases 3 and 8.
- The forward paper gate (EV-11): Set T.
- The shadow random arm (EV-09): Set R-b.

### Excluded because something else already owns it

| What | Owner | Evidence |
|---|---|---|
| `family_test` | Set S | S-3.9 `family_wise_test` |
| `compare_candidates` | Set S | S-10.10 |
| `factor_attribution` | Set S | `factor_spanning_test` |
| EV-01, EV-04, EV-05 leakage battery | Set S | Phases 3, 8 |
| EV-09 random arm | Set R-b | RB-3.1 |

## 3. Decisions made while planning (ADR rows)

Each row records the alternative that was rejected. They are this set's own ADR index; promote one to `docs/adr/` when a task relies on it.

| ID | Decision | Rejected alternative | Why |
|---|---|---|---|
| ADR-Q-01 | The §8 table has **eight** High tools and four Medium; this set plans those twelve. | Plan 'the 11 High tools' as ROADMAP §5 and OPEN-TASKS say. | The table in 11 §8 was counted on 2026-10-04: alpha_chain_audit, compare_to_random, family_test, capacity_report, posterior_alpha, run_robustness_suite, compare_candidates, benchmark_run are High. |
| ADR-Q-02 | `family_test` is `family_wise_test` and offers Romano-Wolf only. | Add SPA and stepwise SPA behind a `method` argument. | ADR-S3-06: three procedures behind one question is a menu to shop between. |
| ADR-Q-03 | `capacity_report` is first a template over bars and a declared edge; the experiment-reading variant is a follow-up. | Wait for an API route that reads trades. | The pre-trial question ('can this edge survive size at all') is answerable now and is the one a hypothesis registration needs; Gate 4's measured ladder already exists. |
| ADR-Q-04 | New tools are templates where the shape allows; workflow batteries only where a tool must dispatch runs or read verdicts. | Hand-written Rust handlers for each. | A template is declaration and implementation in one file and is covered by the existing determinism test; a hand-written handler has a second place to forget. |
| ADR-Q-05 | A stage or rule with no evidence is `unknown`/`NotComputable`, never `pass`. | Default to pass when a row is absent. | The standing rule for every gate. |
| ADR-Q-06 | `decompose_strategy_returns` takes an optional `exposure` array (one value per bar) and, when it is omitted, decomposes a labelled reference rule (`long` or `vol_target`). The three components are exact (drift + vol-timing + selection = total, because the regression has an intercept). | Require `exposure` (the reference-dataset suite cannot size an array by name). Decompose by a Brinson-style split on realised weights (needs positions per bar the platform does not store for graph runs). | The identity is an algebraic fact, so the test can demand it to rounding. Labelling the reference rule in the output keeps it from being read as a strategy's own. |
| ADR-Q-07 | Q-2.2 is closed by Gate 14, which is Romano-Wolf over the whole recorded candidate family of the experiment (a campaign's candidates are its experiment's runs). A solo experiment has no family clause: Gate 14 is `inconclusive`, a hole, never a failure and never a pass. | A second family test inside Gate 3. | ADR-Q-02: one procedure behind one question. A second implementation would have to be kept in step by hand. |
| ADR-Q-08 | The alpha chain maps its five stages to gates: PIT -> 2; stable signal -> 7, 8, 13, 14; feasible positions -> 12; executable orders -> 15; net alpha after costs and capacity -> 3, 4. A failure **with** a statistic is `fail`; a failure **without** one is the stack's record of an inconclusive gate, so `unknown`. | Add an `inconclusive` column to `mlops.gate_verdict`. | The spec names stages and gates but not which answers which. The statistic test needs no migration and matches how `to_record` writes an inconclusive gate. |
| ADR-Q-09 | The robustness suite's "perturbed paths" member is a `synthetic_paths` study; "passes" means: most pessimistic cost rung > 0, one-bar-late metric > 0, and the study median > 0 for the other four. A member not supplied is `not_run`; the suite is complete only when all six ran. | Let the caller define pass. | A bar the caller sets is a bar it can set until the strategy passes. |
| ADR-Q-10 | `allocate_book` defaults to the Ledoit-Wolf constant-correlation shrunk covariance and says so; the sample covariance is reachable only by naming it. The estimator is **linear** shrinkage, named exactly (not "nonlinear"). | Implement analytic nonlinear shrinkage (Ledoit-Wolf 2020) and call it T9's. | The estimator already shipped in `shrunk_covariance`; claiming the nonlinear one would be a false description. |
| ADR-Q-11 | The theme-posterior clause (EV-07) applies to every profile, inside Gate 8, reading the same family Gate 14 reads; it can only tighten a gate. | A new profile `strict_v2` with a `posterior_required` threshold. | A rule is code and a threshold is a row; the clause adds no number. The cost is that old and new verdicts under `strict_v1` are not strictly comparable for candidates whose interval includes zero, which the verdict detail states. |

## 4. Phase summary

| Phase | Title | Tasks | Migrations | State |
|---|---|---|---|---|
| 1 | [The §8 evaluation tools](phase-1-the-section-8-tools.md) | 12 | none | 5 / 12 done |
| 2 | [Evidence-standard extensions](phase-2-ev-extensions.md) | 8 | none | 0 / 8 done |

### Territory note

Tools Q-1.2 to Q-1.5 are Python templates under `apps/skill-runner/templates/`, which is inside the F-RESERVED territory and needs no Rust registration: `build.rs` embeds the file.
The existing suite `crates/mcp-server/tests/templates_run.rs` runs every template twice and compares `detail` byte for byte, so the new ones are covered by it. The namespace caps (16) and the
descriptor budget (1,200 bytes) are enforced by `crates/mcp-server/src/lib.rs` tests.
