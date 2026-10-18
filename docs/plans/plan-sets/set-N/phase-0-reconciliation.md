# Phase 0 — Reconciliation: two DatasetSpecs, a stale Set H, a GPU question

**Completion: 100% (3 / 3 tasks).**

**Requirement IDs:** MD-01, MD-07
**Spec:** [FEAT-006](../../../specs/FEAT-006-model-platform-v2.md) §3, §11 and the amendment note
**Depends on:** nothing inside this set.
**Migration range:** none.

FEAT-006 was written on 2026-09-11 before the ML platform pack landed. Three things in it collide with what is now in the tree, and a builder who
does not see them will build a parallel path.

---

### N-0.1 Reconcile the declaration with `dataplane::dataset::DatasetSpec` — S
`DONE` 2026-10-04 — decided from reading `crates/dataplane/src/dataset.rs` and `crates/model-registry/src/datasets.rs`.

There are two things called DatasetSpec. `dataplane::dataset::DatasetSpec` is the **resolved** spec the ML pack hashes into a `dataset_id` (surrogate instrument
ids, PIT anchor, quality mask, calendar versions, adjustment policy, feature-DAG hash, label and split specs, image digest). FEAT-006 §3's `DatasetSpec` is what an
agent **declares** (universe, features, transforms, sampling, target, weights, sequence, cv). Decision ADR-N-01: the declaration is a new type,
`DatasetDeclaration`, that is validated, hashed canonically, and *lowers* to a `DatasetRequest`; `DatasetManager::plan` then resolves it into the existing spec.
Nothing re-implements the resolved spec or the `dataset_id`.

**Files:** this file §3, ADR-N-01
**Acceptance:** ADR-N-01 names the rejected alternative; N-1.1 and N-1.2 build only the declaration and its lowering.
**Traces:** FEAT-006 §3; ML pack SPEC §3.1, INV-12

### N-0.2 Re-derive Set H Phases 4–5 against rings, `mlops` and Meridian — M
`DONE` **R-N2 2026-10-05:** every H-4.5 to H-5.9 row has a verdict (table below); none is left as "verify". H-4.5, H-4.7 and H-4.8 are built (tests in `crates/model-registry/tests/pg_model_ops.rs` and `rate_limit.rs`); H-4.6 is built minus the two items dropped by ADR-N-R2-05; H-5.1 to H-5.9 are re-cut as N-5.1 to N-5.9 one for one (all built `PARTIAL` there). OPEN-TASKS Part B still carries the old verdict text: the orchestrator's edit is in the R-N2 report.

| Row | Verdict | Where / evidence |
|---|---|---|
| H-4.1 to H-4.4 | replaced | Set U U-5 (registry/inference foundations) |
| H-4.5 used-by | built | `ModelManager::get_used_by` + `GET /api/models/{id}/used-by`; pg test `used_by_lists_the_strategies_that_reference_the_model_and_only_those` (a strategy referencing another model is not listed) |
| H-4.6 traces and rollups | built, two items dropped | `GET .../traces`, `GET .../traces/rollup` (now `InferenceGateway::trace_rollup`, one SQL shared by the route and the test); pg test `the_trace_rollup_counts_outcomes_and_reports_latency`. The ClickHouse trace row and a cost field are dropped: ADR-N-R2-05 |
| H-4.7 rate limits and permissions | built | per-model sliding-window limit in `InferenceGateway::forecast` (`rate_limit::SlidingWindow`, default 1200 per minute, `MODEL_INFERENCE_MAX_PER_MIN`), traced as `rate_limited`; permissions: `require_human` on every alias-moving route (N-4.3) and `every_mounted_route_refuses_an_anonymous_caller`; tests `rate_limit::tests` (4) and pg `a_model_past_its_rate_limit_is_refused_and_the_refusal_is_traced` |
| H-4.8 nightly retrain | built, chain re-cut | `RetrainScheduler` spawned from `main.rs` (24 h, `TBOT_RETRAIN_PERIOD_HOURS`); the due list is `RetrainScheduler::due_models` with pg test `only_an_auto_retrain_model_with_no_run_in_flight_is_due`. Train, then eval, then gate is **not** scheduled: re-cut as the deferred item N-4.5b below |
| H-5.1 to H-5.9 | re-cut | N-5.1 to N-5.9 (routing and data layer, Command Center, Create wizard, Cockpit, Training console, Test lab, Evaluations arena, Version timeline and promotion gate, Lineage graph) |

**N-4.5b (deferred, not a task in the count):** schedule evaluation, and the promotion gate's check, after an auto-retrain run lands, so a nightly candidate arrives with its scorecard. It needs an eval-dispatch caller in `scheduler.rs`; nothing here promotes (N-4.3).

Set H's remaining 17 tasks were written against the pre-`mlops` registry and a Streamlit-era UI. `promote_model_version` is a ring-1 write reachable only
through a `Proposal` a human decides (AGENT-013); the frontend was torn down to the Meridian design system on 2026-09-12. Each H-4.x and H-5.x row in OPEN-TASKS Part B
gets one of: built (with evidence), replaced (by which U task), or re-cut here as an N task.

**Files:** this set's Phase 4 and Phase 5; OPEN-TASKS Part B (orchestrator edits it)
**Acceptance:** every H-4.5…H-5.9 row has a verdict; none is left as 'verify'.
**Traces:** FEAT-006 amendment note; OPEN-TASKS Part B

### N-0.3 Decide the GPU question: `sequence` is SHOULD — S
`DONE`

FEAT-006 open question 1. The local trainer host is a Pascal GTX 1080 Ti (see `local-agent-on-pascal` in the programme notes): it can train small sequence models,
slowly. Decision ADR-N-02: the `sequence` family is SHOULD, gated on the `max_gpu_hours` manifest field, and MD-07 (real windows) is MUST because the seq-len-1 bug is a correctness defect
regardless of speed.

**Files:** this file §3, ADR-N-02
**Acceptance:** ADR-N-02 is recorded; N-2.5 is MUST and N-2.6 is SHOULD.
**Traces:** FEAT-006 §11; MD-06, MD-07
