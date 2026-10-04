# Phase 8 — Parity, proven; old paths retired

**Completion: 40% (2 / 5 tasks).** U-8.1 and U-8.2 DONE 2026-10-03; U-8.5 PARTIAL; U-8.3 and U-8.4 BLOCKED on FEAT-007 §12's own condition — the old paths go only after the replay parity check has passed on translated strategies running on paper, and five producers still write v1.

**Requirement IDs:** SU-PAR-01…05.
**Spec:** [FEAT-007](../../../specs/FEAT-007-unified-strategy-engine.md) §11, §12, §15; ADR-0047.
**Depends on:** Phases 3, 4, 7.
**Blocks:** nothing — this closes the set.
**Migration range:** none.

Sharing code makes parity true; this phase makes it *measured*, and then removes every evaluator that is not the engine.

---

### U-8.1 Replay harness — M
`DONE` 2026-10-03 — `crates/backtest/src/replay.rs`. A decision can only be reproduced from the moment
its engine was built (an EMA remembers every bar since; a `Checkpoint` holds no indicator state), so the
paper runner now journals a `Genesis` on the first record after each start (the warm-up bars, and the
checkpoint when it restored one) and, on every record, the events it consumed without deciding
(`preceding`). `JournalInputs`/`Genesis` are the one definition of the record — the runner writes them,
the replay reads them, and the runner builds its engine through `Genesis::build`, the call a replay
makes. A segment (a genesis and the records after it) is replayed through a plan compiled by
`compile_for_run`, model answers fed back as received; records with no genesis are `unreplayable`, never
a match. **Found by the replay:** serde_json without `float_roundtrip` reads some decimals back one ULP
off, so a journal read back from JSON drifted in an EMA's last digit at decision 18 — the workspace
`serde_json` now enables it, with a 20 000-value round-trip guard. Evidence: `live_runner` tests — a
paper journal of 120 decisions (warm-up genesis, fills consumed without deciding) replays identically;
a journal across a restart replays segment by segment; recorded model answers (one answered, one timed
out) replay as received; an engine compiled with `n: 22` for `n: 21` diverges at record 1 naming
`values.s…`; `replay` unit tests 3/3.

Feed a paper journal's recorded inputs — bars, portfolio views, model responses — through the engine as the backtest environment builds it; compare decisions byte for byte; report the first differing step.

**Files:** `crates/backtest/src/replay.rs`.
**Acceptance:** a journal replays identically; a deliberately perturbed engine fails naming the step and the field.
**Traces:** ADR-0047 decision 2; QA-1.

### U-8.2 Nightly parity job — S
`DONE` 2026-10-03 — `apps/platform/src/parity_jobs.rs`, spawned beside `feature_jobs` in `main.rs`.
Each night every deployment that decided in the previous UTC day has its journal replayed (the
deployment's recorded graph, compiled against the registry as a backtest compiles it) and every
segment's verdict kept append-only in `strategy_parity_check` (migration 0149: a divergence must name
its record and field, an identical verdict its decision count, an unreplayable one its reason; granted
to `app_role`). A divergence is raised at `error` as a P1 — the severity `feature_jobs` gives a code
drift; an unreplayable segment is a warning. Evidence: `parity_jobs` tests 7/7 incl.
`the_parity_job_is_spawned_at_boot` (source scan), `a_seeded_difference_is_a_p1_naming_the_record_and_field`,
a perturbed engine, a graph that no longer compiles, segments outside the day; storage
`parity_verdicts_are_recorded_append_only_and_by_the_platform_role` on a scratch PostgreSQL 17
(checks, append-only, `SET LOCAL ROLE app_role`). *Not here:* the verdicts are not yet a row in the
self-monitor's §16.2 signal list, which is a spec contract of its own.

Run the harness nightly over the previous day's paper journals; a difference raises the same severity as a feature-consistency code drift.

**Files:** `apps/platform/src/parity_jobs.rs`.
**Acceptance:** the job is registered at boot (source-scan guard, the pattern of `fill_stream_tests`); a seeded difference produces an alert.
**Traces:** ADR-0047.

### U-8.3 Retire the old evaluators — M
`BLOCKED:UPSTREAM` 2026-10-03 — on FEAT-007 §12's own condition: "old paths run until the replay
parity check passes on translated strategies, then are deleted". No translated strategy has run on paper
yet (the platform database records 0 deployments), so the check has nothing to pass on. And deleting
now would strand what still produces or runs v1 (mapped 2026-10-03): hot-path stage 3 fed by
`/api/strategies/{id}/start`, automations arm/resume and `deploy_paper` approval; research sweeps (v1
parse and `SearchSpace`); `GET /api/strategies/{id}/config`, `apply_list`, lifecycle simplification;
the SLv2 MCP tools; `StrategyBuilderPage` (posts v1) and `ScannerPanel` (evaluates v1 in the browser).
Each needs a v3 arm before the evaluators can go.

Delete `sim.rs`'s evaluation loop, `preview.rs`'s loop, `interpreter.rs` and hot-path stage 3's `StrategyInstance` once every stored strategy runs on v3. A source-scan guard fails the build if any reappears.

**Files:** `crates/backtest/src/`, `crates/strategy-runtime/`, `apps/platform/src/hot_path.rs`.
**Acceptance:** `strategy-engine` is the only crate that evaluates a strategy (guard); the workspace builds and every suite passes without the retired code.
**Traces:** ADR-0039; FEAT-007 §15 criterion 1.

### U-8.4 Retire the duplicate grammars — M
`BLOCKED:UPSTREAM` 2026-10-03 — behind U-8.3: `strategy-validator`'s grammar validates every v1
write (`create_strategy`, the MCP builder) and the bytecode compiler runs stage 3, both still live.

Remove `strategy-validator`'s expression parser and the runtime bytecode compiler as grammar implementations; the domain EBNF becomes documentation of v1 only; DATA-004, FEAT-001 and FEAT-005 are updated as-built.

**Files:** `crates/strategy-validator/`, `crates/strategy-runtime/src/bytecode.rs`, `docs/specs/`.
**Acceptance:** exactly one parser for strategy expressions exists in the workspace (guard); the as-built specs describe the engine that runs.
**Traces:** ADR-0045.

### U-8.5 Migrate stored strategies — M
`PARTIAL` 2026-10-03 — `crates/api/src/bin/migrate_strategies_v3.rs` translates every stored v1 head, compiles
it as a backtest would, re-runs its latest completed backtest on the engine (same instrument, window
and balance) and publishes old against new with each difference attributed or marked unattributed;
`--apply` saves the translation as the head through the platform's one revision write
(`append_revision`, split out of `write_revision`, authored `scheduler`). **Dry run against the
platform database, 2026-10-04:** [report](../../../reports/strategy-v3-migration-2026-10-03.md) — of 14
stored strategies, 9 translate (1 re-run; 1 cannot re-run, `BTC-USDT` has no instrument row; 7 have no
backtest on record), 2 translate but do not compile (`btc_forecaster@production` is not in the
registry; `model_signal` is no indicator), 3 are refused by name (two parameterised-feature smoke
tests, one reading another instrument). **Found by the re-run and fixed:** v1 places an order on a
signal's *rising edge* and the translation placed one on every bar the condition held (10 079 orders
for v1's 333) — entries now translate as `c and not prev(c, 1)`; v1 multiplies conditions as 1/0 and the
translation now writes `if(x, 1, 0)`; the compiler reported a mistake inside a nested expression twice;
**`storage::postgres::instruments::fetch_by_id` has selected columns that do not exist (`id`,
`is_active`) and decoded an INT4 as i64 since June**, so every v3 backtest, preview and series run
failed on a real database (`tests/pg_instruments.rs` now holds it); and the attribution had credited an
order count to fill timing — counts are now attributed only to what changes a count, and a different
number of bars seen (`bars_differ`: 6 022 then, 10 079 now for the same week, orders per bar 0.055 vs
0.056) is measured, not assumed. **Not met:** `--apply` was not run. Making v3 the head would break
those strategies in every consumer that still reads only v1 (automations arm/resume, research sweeps,
`GET /api/strategies/{id}/config`, `StrategyBuilderPage`, `ScannerPanel` — U-8.3's list), and no
stored strategy is deployed. Apply once those consumers have a v3 arm.


Translate every stored strategy, re-run its latest backtest on the engine, and publish old-versus-new with the translator's behaviour-change list, so no result changes silently.

**Files:** `crates/api/src/bin/migrate_strategies_v3.rs`, `docs/reports/`.
**Acceptance:** every stored strategy has a v3 revision or a named refusal; every re-run's difference is attributed to a listed behaviour change.
**Traces:** FEAT-007 §12.
