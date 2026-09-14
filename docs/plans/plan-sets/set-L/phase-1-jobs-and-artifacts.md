# Phase 1 — Durable job service and artifact store

**Completion: core complete (JB-01…JB-12 implemented and verified); Set K C deferred**

**Requirement IDs:** JB-01…JB-12; Set K Phase B (durable stores), Phase C (parallel
execution).
**Spec:** [COMP-005](../../../specs/COMP-005-job-service-and-artifacts.md), ADR-0030.
**Migration:** `0037_jobs_artifacts.sql`.

---

## What was built

| Piece | Where |
|---|---|
| Job domain, kinds, states, error and output caps | `crates/jobs/src/types.rs` |
| RFC 8785 canonical manifests and the idempotency hash | `crates/jobs/src/manifest.rs` |
| Durable store: submit, claim, heartbeat, complete, fail, cancel, reap | `crates/jobs/src/store.rs` |
| Trial counter against Set J's experiments | `store::PgTrialCounter` |
| Content-addressed artifact registry | `crates/jobs/src/artifacts.rs` |
| Worker SDK and bounded pools | `crates/jobs/src/worker.rs` |
| REST surface | `crates/api/src/routes/jobs.rs` |
| First worker (`data_qc`) | `crates/api/src/workers.rs` |
| Pools + lease reaper on boot | `apps/platform/src/main.rs` |

## The three properties that carry the design

**Idempotent submission (JB-02).** A job is identified by
`sha256(canonical(manifest) ‖ sorted(code_hashes) ‖ data_snapshot_id ‖ kind)`.
Canonicalisation matters more than it looks: two callers building the same manifest
with their object keys in a different order must get one job, and the live platform
demonstrates it — submitting `{instrument_id, timeframe}` and then
`{timeframe, instrument_id}` returned the same `job_id` with `deduplicated: true`.

**Trials counted at submission, in the same transaction (JB-03, INV-1).** This closes
the hole Set K flagged and never fixed: the suite's trial counter lived in
`InMemory*Store` and did not survive a restart. Registration now happens inside the
submission transaction, so the job row and the trial commit together or not at all.
`a_refused_trial_rolls_the_job_back` asserts the rollback, because the two failure
modes are both unrecoverable after the fact — a counted trial with no job inflates
every later significance claim, and a job with no counted trial deflates it, which is
the direction that flatters the researcher.

**Leases, not liveness assumptions (JB-04).** A worker renews a lease every 20 s; if
it dies, the reaper re-queues the job. After two infrastructure re-queues the job
fails as `lost_worker` rather than cycling forever.

## Verification

19 integration tests against live Postgres, 28 unit tests. The ones worth naming:

- `identical_submissions_dedupe_and_count_one_trial`
- `a_refused_trial_rolls_the_job_back`
- `terminal_jobs_cannot_be_resurrected` — attempts the `UPDATE` directly and asserts
  the database trigger refuses it
- `the_counter_cannot_be_lowered_even_by_raw_sql`
- `an_expired_lease_returns_the_job_to_the_queue`
- `cancelling_a_parent_cancels_its_children`
- `exploration_is_logged_and_never_counted`

End to end on the running platform: a `data_qc` job submitted over HTTP was claimed,
reported progress, queried ClickHouse and returned a grade — and produced a real
finding, that BTC-USD 1m is **grade D** at 59.7% coverage.

## Four defects that only appeared by running it

1. **A `JobError` code carrying a whole message.** `JobError::infrastructure(format!(…))`
   compiles, reads fine in a log, and destroys the one field callers branch on. Now
   rescued into `fix` by `clamped()`, with a test.
2. **A ClickHouse row-type mismatch.** `toUnixTimestamp` returns UInt32; the struct
   field was i64. The error — "not enough data, probably a row type mismatches a
   database schema" — names no column. Every aggregate is now cast explicitly.
3. **The boot-time bar backfill aborted a healthy platform.** See Phase 0's amendment.
4. **An em dash in a machine-facing summary** crashed a cp1252 console. Machine-facing
   output is ASCII now.

## What is deliberately not done

- **Set K Phase C (parallel execution inside the suite).** The job service *is* the
  bounded scheduler, and the fixed 3-concurrent backtest semaphore is gone, but the
  Set J `SuiteManager` has not been rewritten to submit its Study members as child
  jobs. Until it is, a Study still runs its members in-process.
- **Set K Phase B (Postgres-backed Run/Study/Experiment stores).** The INV-1 hole it
  was meant to close is now closed by counting at job submission, which is the more
  robust fix; the suite's own stores remain in memory and a restart still loses
  in-flight Study state.
- **NATS/SSE event streaming.** `GET /api/jobs/events` is a resumable cursor feed,
  which is what `tbot jobs wait` needs. The SSE lane for the UI is Phase 6.
- **Budget admission at submission** (`409 budget_exhausted`). Estimates are carried
  but not yet checked against the project's compute budget.
