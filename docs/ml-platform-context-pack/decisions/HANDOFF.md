# Agent handoff — ML platform context pack build

Rewritten 2026-09-14 by the third agent (Phases 2–5). Read this first, then `CLAUDE.md` and `INVARIANTS.md` in this pack.

## 1. The job

Bring the codebase up to the normative spec pack in `docs/ml-platform-context-pack/`. Work the checklist (`backlog/IMPLEMENTATION-CHECKLIST.md`) in phase order, Phases 0–5. Phase 6 is deferred by decision.

Mason's standing instructions:
- "you chose and continue": open decisions are delegated. Make the call and record it in `ADR-INDEX.md`.
- "quit stopping every time you reach a milestone … I'm looking for the finish line": do not end turns at milestones. Keep working until the whole job is done.

Non-negotiable rules (from `CLAUDE.md`):
- Authority order: `INVARIANTS.md` > `spec/SPEC.md` > `reference/*` > your judgment.
- REQUIRED / no default / never / enforced by → a schema constraint, GRANT, type or CI test, never a comment.
- Never add a non-PIT read path, a way to run a trial without a ledger row, or a flag that disables either. That applies in tests too.
- If the spec looks wrong, write it into `OPEN-QUESTIONS.md` and don't implement either version.
- Every uncovered judgment call gets an entry in `ADR-INDEX.md`. The last one written is **ADR-P3-07**; the index is not in numeric order across phases, so check it rather than guessing the next id.
- Never report a partially enforced invariant as enforced (`PROGRESS.md`).

## 2. Where things stand

Authoritative detail: `PROGRESS.md`, `IMPLEMENTATION-CHECKLIST.md`, `ADR-INDEX.md` (110 entries; P0, P1, P2-01…P2-33, P3-01…P3-07, P4, P5).

**Checklist: 78 done, 19 partial, 5 open.** The five open items are the whole of Phase 6, deferred by decision with a written revisit trigger each. **Phases 0–5 have no unstarted item left.** Every partial says in its own entry exactly what is missing.

Phase 0 and 1: done. Phase 2: every item done except 2.1, 2.14 and 2.16, which are partial and reported as partial. Phase 3: done except the learned tiers and the jobs that would *write* `asset_embedding` and the tensor. Phase 4: the ladder and every rule tier are built; no learned tier is fitted, which is ADR-P4-01's decision rather than a gap. Phase 5: all seven built surfaces; 5.8 stays deferred.

## 3. Open failures

- **`lint-no-json-hotpath`** fails on one pre-existing line (`crates/collectors/src/crypto/kraken.rs:20`, a `RawValue` import for zero-copy parsing). Not a CI job; predates this work.
- **Four `storage` live suites fail, and were failing before this work.** `ledger_append`, `ledger_writer` and `pnl_schema` need a `ledger_events` table **no migration in this repository creates**; `registry_seed` expects 8 asset classes where migration 0006 seeds 6. All four files are unchanged from `HEAD`. They belong to the execution subsystem rather than this pack, and fixing them means deciding whether the test or the migration is right.
- ClickHouse's container healthcheck reports unhealthy while the server answers queries normally. Pre-existing.

## 4. Known violations still in code (reported, not fixed)

- **The external MCP front door writes no audit trail.** `apps/mcp-server` is a separate process; the `pre`/`post` records cover the two internal agent dispatch paths only. AT-64 enumerates every `dispatch_tool` caller, so this is a named exemption rather than an oversight.
- **The older SDK driver refuses gated actions rather than pausing on them.** Its loop cannot hold a call across a human answer. The GOVERNOR loop (`local_driver`) pauses and resumes.
- **The trainer's failure classification is not total.** An exception type nobody has classified maps to `dependency_failure`.
- **Overlapping-label leakage has no static detector** (OQ-12, deliberate). Visible only as the 1.10 CV/WF gap, and the test asserts that.
- **Nothing consumes a `Comparison`.** Built, sealed, tested; the COMPARE worker that would call it is 2.1's gap.
- **Five of the sixteen gates have no evidence source** — 3, 4, 10, 12, 15. Eleven judge on real numbers read from the ledger.
- **No framework declares `resume_support: bit_identical`.** `BIT_IDENTICAL_FRAMEWORKS` is empty until AT-61's eval task has demonstrated it, so every resume currently restarts under the same trial.

## 5. Next work, in order

1. **AT-61's eval task.** Train `n` steps, resume from `n−2`, assert bit-identical weights and metrics per framework. It is the one build-blocking acceptance test still unwritten, and passing it is what lets a name enter `BIT_IDENTICAL_FRAMEWORKS`.
2. **2.1's child workers** — `Study`, `EvalTask`, `ResearchRun`. `GateAdvance` exists. Until the rest do, a campaign fails at its first dispatching phase with `no_worker`. The COMPARE worker is also what would finally call `ComparisonPlan::judge`, and with it 2.1's `DIMINISHING_RETURNS` and `CONVERGED` stop reading `NotComputable`.
3. **The gate stack's last five inputs** — the cost sweep's breakeven multiple (3) is one `record_statistic` call from a `CostSweep` study the funnel does not currently run; capacity (4) needs ~5–8 counted runs at AUM multiples; `per_instrument_pnl` (12, also 5.2) and the factor-return series (10, also 3.1) are new artifacts; forward-test observations (15) need the reconciliation crate wired.
4. **Phase 3's writers** — the `InstrumentProfile` job that calls `fingerprint.compute` and writes `knowledge.asset_embedding`, and a timer for `TensorProjection::rebuild`. Both modules exist and are tested; neither is scheduled.
5. **Keep the live suites in CI.** They pass today against a real database; the four listed in §3 do not, and they were already failing.

## 5b. Database state

The developer database **is at migration 57** and the Postgres container **is**
`pgvector/pgvector:pg16`. Applying 0043–57 to it found two bugs that a fresh
database could not have shown (see `PROGRESS.md`), and the old alpine volume had
its indexes rebuilt with `REINDEX DATABASE` because musl and glibc sort text
differently and alpine records no collation version to compare against. A dump
was taken first.

To bring any other database up:

```
DATABASE_URL=postgres://trading:trading@localhost:5432/trading \
  cargo run -p storage --example migrate
```

It refuses to run without `DATABASE_URL`, prints the before and after version,
and is idempotent.

## 6. Environment and how to verify

```
cargo test --workspace -j 2          # -j 2 is not optional: full parallelism OOMs the page file
cargo clippy --workspace --all-targets -j 2 -- -D warnings
cargo run -q -p xtask -- check-money-f64
cargo run -q -p xtask -- check-bars-v1-frozen
cargo run -q -p xtask -- lint-no-json-hotpath   # one known pre-existing failure
python -m pytest apps/model-trainer/tests -q    # the post-hoc pipeline, AT-35
cd frontend && npx tsc --noEmit
```

Live-Postgres suites are `#[ignore]` by default. All of these pass:

```
export DATABASE_URL=postgres://trading:trading@localhost:5432/trading
export CONSISTENCY_E2E_CLICKHOUSE_URL=http://trading:trading@localhost:8123
cargo test -p invariants --test db_invariants     -- --ignored --test-threads=1
cargo test -p ledger     --test pg_campaign       -- --ignored --test-threads=1
cargo test -p ledger     --test pg_ledger         -- --ignored --test-threads=1
cargo test -p ledger     --test pg_neff           -- --ignored --test-threads=1
cargo test -p ledger     --test pg_tenancy        -- --ignored --test-threads=1
cargo test -p ledger     --test pg_holdout        -- --ignored --test-threads=1
cargo test -p ledger     --test pg_anchor         -- --ignored --test-threads=1
cargo test -p ledger     --test pg_trajectory     -- --ignored --test-threads=1
cargo test -p backtest   --test pg_gate_profile   -- --ignored --test-threads=1
cargo test -p api        --test pg_self_monitor   -- --ignored --test-threads=1
cargo test -p api        --test pg_feature_consistency -- --ignored --test-threads=1
```

The job-store suite is gated on its own variable and skips silently without it,
which is why it is easy to believe it ran when it did not:

```
JOBS_TEST_DATABASE_URL=postgres://trading:trading@localhost:5432/jobs_test \
  cargo test -p jobs --test job_store -- --test-threads=1
```

Never pipe cargo through anything that swallows its exit code.

## 7. Memory notes worth reading

In `C:\Users\Mason\.claude\projects\C--Users-Mason-Desktop-coding-projects-trading-bot\memory\`:
- `ml-platform-context-pack.md`
- `trial-ledger-architecture.md` — why the sealed types and the append-only design must not be "simplified"
- `feedback-work-to-finish-line.md`
- `build-env-gotchas.md`
- `docker-phantom-socket-crashloop.md`
