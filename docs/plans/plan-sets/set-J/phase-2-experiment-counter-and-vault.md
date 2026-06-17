# Phase 2 — Experiment, trial counter & holdout vault

**Completion: 0% (0 / 9 tasks)**

**Goal:** Build the container for the whole investigation of one strategy idea —
the **Experiment** — and the two things that make the suite honest: the **global
trial counter** (automatic, monotonic, irreversible) and the **holdout vault**
(one logged access, self-sealing). Add the one-directional lifecycle state
machine that gates which operations are allowed, and the up-front `primary_test`
declaration that Gate 3 will read.

**Depends on:** Phase 1 (`Study`, `trial_delta`), Phase 0 (`RunStore`, `unsafe`).
**Blocks:** Phase 4 (the funnel reads lifecycle state, the counter, and the vault),
Phase 5 (the workbench renders the counter + lifecycle).

---

## Design notes

**The counter is the point.** Most backtesters fail not because they lack tests
but because the user runs 500 variations, remembers the 3 that worked, and
reports those as if they were the only attempts. The counter removes the human
from the counting loop: Studies increment it automatically, at the Study level,
*before* the user ever sees a result. You physically cannot under-report trials.

**Contract (frozen shape, `crates/backtest/src/experiment/mod.rs`):**

```rust
pub struct Experiment {
    pub experiment_id: ExperimentId,
    pub strategy_family: StrategyFamilyRef,  // version-agnostic root idea
    pub state: ExperimentState,              // candidate|validated|live|decaying|retired
    pub studies: Vec<StudyRef>,
    pub trial_counter: i64,                  // AUTOMATIC, MONOTONIC, IRREVERSIBLE
    pub holdout: Holdout,                     // locked tail + access_log + spent
    pub primary_test: NullRef,                // the ONE designated significance test (up front)
    pub verdict: Option<ExperimentVerdict>,
    pub created: DateTime<Utc>,
    pub updated: DateTime<Utc>,
}

pub struct Holdout {
    pub slice: DataSlice,                     // the locked tail of data
    pub access_log: Vec<VaultAccess>,         // { when, run_id, by } — every touch, forever
    pub spent: bool,                          // true after the single permitted vault run
}
```

**Lifecycle (one-directional through validation):**

| State | Meaning | Allowed operations |
|---|---|---|
| `candidate` | under active research | all Studies except vault; counter runs hot |
| `validated` | passed all gates, vault spent | no research Studies; may promote to live |
| `live` | deployed | only reconciliation Studies (Phase 5) |
| `decaying` | live perf below backtest distribution | reconciliation + diagnostic Studies; flagged |
| `retired` | pulled | read-only |

Once you spend the vault and reach `validated`, you **cannot** drop back to
`candidate` and keep researching against the same holdout. More research requires
a fresh holdout (new data) and a fresh Experiment.

---

## Tasks

### ☐ J-2.1 `Experiment` aggregate + persistence — M
Add `crates/backtest/src/experiment/mod.rs` with the design-notes shape and
Postgres migration **0028** (`backtest_experiments`: id, strategy_family, state,
trial_counter, primary_test, holdout JSON, verdict, timestamps;
`backtest_vault_accesses`: append-only log). An Experiment is created in
`candidate` with `trial_counter = 0`, `holdout.spent = false`, and a declared
`primary_test`.
**Acceptance:** create→load round-trip; a new Experiment starts `candidate`,
counter 0, vault unspent; `primary_test` is mandatory at creation.

### ☐ J-2.2 Monotonic, irreversible trial counter — M
`Experiment::record_study(study_result)` increments `trial_counter +=
study_result.trial_delta` and appends the `StudyRef`. There is **no** decrement,
reset, or setter. The increment is the *only* way the counter changes, and it is
called automatically by the study-run path (J-2.3), never by user API.
**Acceptance:** the counter only goes up; a test asserts no public method
decreases it; recording three studies of deltas 200/120/1000 yields 1320.

### ☐ J-2.3 Auto-increment wiring (no human in the loop) — M
Wire `StudyEngine::run` (Phase 1) so that running a Study **within an
Experiment** automatically calls `record_study` *before* the `StudyResult` is
returned to the caller. A Study cannot be run "off the books": the only public
entry to run a Study attached to an Experiment routes through the counter.
**Acceptance:** an integration test runs a Study and observes the counter
incremented before it can read the result; there is no code path that returns a
`StudyResult` for an Experiment without having incremented.

### ☐ J-2.4 Lifecycle state machine — M
Add `ExperimentState` + `Experiment::transition(to)` enforcing the legal,
one-directional graph: `candidate→validated→live→{decaying↔live}→retired`, with
no edge back to `candidate` after `validated`. Each state exposes
`allows(operation) -> bool` (e.g. `validated` forbids research Studies). Illegal
transitions return an error.
**Acceptance:** every legal edge succeeds; `validated→candidate` is rejected;
running a research Study in `validated`/`live` is rejected with a clear error.

### ☐ J-2.5 Holdout vault definition + lock — M
On Experiment creation, carve the **locked tail** (`holdout.slice`, typically the
most-recent data window) out of the addressable range. While `candidate` or
`validated`, **no Study's `data_slice` may intersect `holdout.slice`** — enforced
at `StudyConfig` validation against the owning Experiment. A study touching the
vault range is refused.
**Acceptance:** a Study whose `data_slice` overlaps the holdout is rejected in
`candidate`; the holdout range is excluded from all research-Study data loads
(test).

### ☐ J-2.6 One-shot vault access + self-seal — L
Add `Experiment::run_vault(run_config) -> Result<RunResult>` callable **only**
from a Gate-3-passed Experiment (the gate dependency lands in Phase 4; here,
expose a `gate3_passed` precondition flag). It: (1) refuses if `holdout.spent`;
(2) executes exactly one Run over `holdout.slice`; (3) appends a `VaultAccess
{ when, run_id, by }`; (4) flips `holdout.spent = true` **before** returning. A
second call is refused at the API level. `spent` never flips back.
**Acceptance:** the first vault run succeeds and logs access; the second is
refused; `spent` cannot be reset (no setter); the access log is append-only and
survives a round-trip.

### ☐ J-2.7 `primary_test` declared up front — S
`primary_test: NullRef` is set at Experiment creation and is **immutable**
thereafter (changing the significance test after seeing results is p-hacking).
Gate 3 (Phase 4) reads exactly this null. The null object itself lands in
Phase 3; here, store and freeze the reference.
**Acceptance:** `primary_test` cannot be changed after creation (test); a missing
`primary_test` blocks creation.

### ☐ J-2.8 `unsafe` propagation to Experiment — S
If any Study or Run within an Experiment is `unsafe` (INV-1, Phase 0), the
Experiment is permanently flagged `unsafe = true`, surfaced in its verdict and
the workbench, and **barred from the vault** (you cannot validate an idea whose
research disabled protections). The flag never clears.
**Acceptance:** an Experiment containing one `unsafe` run is flagged and its
`run_vault` is refused; the flag survives round-trip and cannot be cleared.

### ☐ J-2.9 No-laundering test (rename ≠ fresh start) — S
Add a test asserting that "starting over" is structurally a *new* Experiment with
its own zeroed counter and fresh holdout — there is no operation that resets an
existing Experiment's counter or re-locks a spent vault. Document the
strategy-family vs experiment-id distinction (the family is the idea; each
Experiment is one honest investigation of it).
**Acceptance:** no API resets the counter or un-spends the vault; a test
documents that a renamed/cloned Experiment cannot inherit a lower trial count.

---

## Exit criteria

- An Experiment owns a trial counter that only increments — automatically, via
  Studies — and a holdout vault that grants exactly one logged, self-sealing
  access.
- The lifecycle is one-directional through validation; research Studies are
  barred once `validated`, and the holdout range is unreachable while researching.
- `primary_test` is declared up front and frozen; `unsafe` propagates and bars
  the vault.
- Trial count cannot be laundered by renaming. `cargo test -p backtest` green.
