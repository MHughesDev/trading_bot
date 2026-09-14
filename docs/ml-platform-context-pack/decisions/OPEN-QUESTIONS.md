# Open questions

Write here instead of guessing. One entry per question. Keep working on unrelated items while a question is open.

## Template

```
### OQ-nn · <one-line question>
**Blocking:** <which checklist items>
**Context:** <what you found in the code / spec / reference>
**Options:**
  A. <option> — cost: <effort/risk>
  B. <option> — cost: <effort/risk>
**Spec says:** <quote or "silent">
**Reference says:** <file + finding, or "not covered">
**Recommendation:** <yours, and your confidence>
**Status:** open | answered <date> | superseded
```

## Questions carried in from the spec phase

### OQ-01 · Historical `knowledge_time` for already-ingested data
**Blocking:** Phase 0 migration.
**Context:** `knowledge_time` cannot be reconstructed. `CLAUDE.md` §6 prescribes a sentinel (`event_time + declared_vendor_lag`) plus a `BACKFILLED_KNOWLEDGE_TIME` quality flag.
**Needs confirmation:** what vendor lag to declare per source, and whether datasets spanning the sentinel boundary should be blocked rather than flagged.
**Status:** answered 2026-09-14 (decided under Mason's delegation — ADR-P2-24).

**Decision.** (1) The sentinel is `bar close + declared_vendor_lag` — the canonical migration already computes it that way (`canonical.rs`: `v.event_time + lag`, where v2's `event_time` is the close), and it is the only honest arithmetic: a candle cannot be known before it closes. (2) The declared lags stand as seeded in `dataplane.source`: REST candle endpoints 60 s (Coinbase, Kraken, Binance), Alpaca free-tier 900 s, Tradier delayed 900 s, Oanda 5 s, live streams 0. `unknown_legacy` rows carry 60 s **and** the `BACKFILLED_KNOWLEDGE_TIME` flag; no new flag bit — the source id already distinguishes them and the dataset spec records the fraction. (3) **Flag, never block.** Blocking would remove essentially all history the platform has (backfilled since 2026-09), making CPCV and regime coverage impossible, to guard against a ≤ 60 s effect on minute bars. Instead the sentinel becomes *structural*: the dataset's sources' maximum declared lag enters `EmbargoInputs.max_knowledge_lag_ms`, so the computed embargo already exceeds any error the sentinel could introduce. The one place the boundary is a hard line is money: Gate 15's forward-test evidence must come from observed-knowledge rows only.

### OQ-02 · Legacy trials with no propensity
**Context:** `CLAUDE.md` §6 prescribes `policy_id='legacy_unlogged'`, `propensity=NULL`, excluded from off-policy estimators, **but still counted in N_eff**.
**Needs confirmation:** that counting legacy trials in N_eff is acceptable — it makes deflation stricter and some existing strategies may stop passing gates.
**Status:** answered 2026-09-14 (ADR-P2-25). **Yes — count them.** A legacy look happened; excluding it is the direction that flatters results, and the whole product claim (Appendix B) is that this platform is structurally unable to not count. Legacy trials with a stored return series cluster like any other; those without one (training runs, legacy jobs) enter `N_eff` as independent — the conservative direction. They stay excluded from every off-policy estimator by construction (`policy_id = 'legacy_unlogged'`, `propensity = NULL`). The comparison layer shows "`N_eff` includes *k* legacy looks" so the strictness is visible rather than mysterious. The consequence for existing strategies is OQ-04's subject.

### OQ-03 · Options universe gate parameters
**Context:** spec §1.6 defaults to `|moneyness−1| ≤ 0.30`, `dte ∈ [1,400]`, two-sided quote. Drives storage from ~7 TB to ~1.3 TB per 10 years.
**Needs confirmation:** whether the strategies in scope need the tails.
**Status:** answered 2026-09-14 (ADR-P2-26). **Keep the default gate**: `|moneyness−1| ≤ 0.30`, `dte ∈ [1, 400]`, two-sided quote, as `universe_id = 1`, bitemporal from the first row. The strategies in scope (BS-007 T16, volatility-risk-premium on BTC — evidence graded *weak*) are near-the-money and short-dated; nothing in scope needs the tails. Under free-only data (BS-007 D-09) options are forward-collected from delayed sources, so the storage question is moot for now and the gate is a *definition* rather than a filter over existing data. Outside the gate: end-of-day snapshots when a source provides them (reference `08-market-data-arch.md` §345), never deletion. Ingestion is the BS-007 Track P (data reach); the tables from 0.12 are ready.

### OQ-04 · Gate profile for existing production strategies
**Context:** `strict_v1` pass rates are expected in the single digits. Existing deployed strategies were not gated this way.
**Needs decision:** grandfather with a `legacy_ungated` marker, or re-gate and accept that some get pulled.
**Status:** answered 2026-09-14 (ADR-P2-27). **Re-gate. Nobody is grandfathered. Nothing is pulled by the gate — existing exposure is ramped down on a schedule.** Concretely: (1) every currently-running strategy's lineage gets a `legacy_ungated` marker — a fact shown beside its numbers, never a permission and never an input to any threshold; (2) its live/paper record **from today forward** is captured as Gate 15 evidence — the cheapest gate evidence the platform will ever have, and the reason "pull immediately" is the wrong answer; (3) Gate 16 applies in reverse: an ungated strategy may not increase size, may hold its current size for one review period while a `strict_v1` (or `paper_v1`) pass is attempted with that record, and steps down 50 % per period to zero if it fails. Grandfathering was rejected because it creates a permanent two-class registry in which the older class is exactly the one that never faced the multiple-testing haircut. Q-4 (no real-money commitment yet) makes the cost of the ramp-down small; the rule is what matters.

## Questions raised by the CURRENT-STATE.md survey (2026-09-12)

### OQ-05 · Is multi-tenant SaaS still the target, or is this platform single-tenant now?
**Blocking:** whether §7 (tenancy, RLS, feature firewall, INV-23/INV-24) is Phase-0-urgent or speculative.
**Context:** the spec's stated context ("Context locked from Mason") describes multi-tenant SaaS, but the surveyed codebase has real per-user auth (migrations 0013, 0038) with no `tenant_id`/RLS concept at all, and only one operator's capital is routed. `CURRENT-STATE.md` §7 treats every tenancy invariant as vacuously true today.
**Options:**
  A. Multi-tenant SaaS is still the plan — build tenant partitioning/RLS now, ahead of a second tenant, so it's not retrofitted onto live data later. Cost: real Phase-0-scale effort with zero users to validate it against yet.
  B. Single-tenant for now — defer all of §7 to whenever a second tenant is actually onboarded, and spend Phase 0 effort on the invariants that are actively violated today (ledger, surrogate IDs). Cost: §7 becomes a real migration later, but against far less data than exists by then.
**Spec says:** "multi-tenant SaaS where every user is trader, quant and ML engineer" (§0, context line) — describes the target, not necessarily today's deployment state.
**Recommendation:** B.
**Status:** answered 2026-09-13 (decided by me, at Mason's direction to choose and continue) — **B: defer all of §7.** Reasoning: Phase 0 is reserved for work where every day of delay destroys unrecoverable information, and there is no tenant data to lose. Building RLS and the feature firewall against a single tenant would be speculative infrastructure validated by nothing. INV-23/INV-24 and S-2 remain *vacuously* satisfied and are reported as such, never as enforced. **Revisit trigger:** the moment a second tenant is real, before any of their data lands — retrofitting RLS onto populated tenant tables is materially worse than building it ahead of the first row.

**Superseded 2026-09-13 by ADR-P0-16.** The new `mlops` and `dataplane` tables were created empty, which is exactly the revisit trigger above: RLS, FORCE RLS, transaction-local tenant context and a restricted runtime role cost nothing to build before their first row and would have required a migration of every row after it. S-2 is now enforced (AT-37 live test, catalog check), not vacuous. Object-storage prefix isolation (checklist 0.23) remains deferred.

### OQ-06 · The legacy `create_backtest` MCP tool bypasses the ledger entirely — retire it or wire it in?
**Blocking:** Phase 0 item 0.17 (executor refuses jobs without a REGISTERED trial).
**Context:** two parallel backtest systems exist. The Set J Experiment/Study/Run flow (spec-aligned schema, migrations 0026-0030) is real but unwired to a database (in-memory only). A separate, older `backtest_jobs`/`BacktestManager` path (`crates/api/src/routes/backtests.rs`, `crates/backtest/src/manager.rs`) has no ledger involvement at all, and is directly callable by the LLM agent via the `create_backtest` MCP tool (`crates/mcp-server/src/tools/backtests.rs`). This is a live INV-16 violation, not a hypothetical one.
**Options:**
  A. Retire the legacy path and point its UI/tool consumers at the Experiment/Study/Run flow once that's wired to Postgres. Cost: whatever UI/tooling currently depends on the simpler job-tile view needs to be re-pointed; likely the cleaner long-term state.
  B. Keep the legacy path for some declared "quick, unlogged sanity check" use case, but that directly contradicts CLAUDE.md ("no test fixture that writes directly to the executor... if a side door exists, an audit will find it was used") — not a real option under the operating rules as written.
**Spec says:** INV-16, CLAUDE.md §3 — explicit, no ambiguity: this must not exist.
**Recommendation:** A.
**Status:** answered 2026-09-13 — **A, implemented.** `create_backtest` is gone from the catalogue, the taxonomy and the dispatch match arm; dispatching it by remembered name returns `unknown_tool` and runs nothing. Two CI tests keep it closed (`no_tool_profile_can_dispatch_an_unregistered_backtest`, `dispatching_create_backtest_by_name_is_refused`). The *human* half of the same bypass was tracked separately as OQ-08 and is now also closed.

### OQ-07 · Wire the existing Set J ledger schema into Postgres, or redesign it while adding the missing fields?
**Blocking:** Phase 0 item 0.16.
**Context:** `backtest_runs/studies/experiments/nulls/gate_verdicts` (migrations 0026-0030) already encode several spec-aligned ideas (no bare score column, content-addressed immutable runs, mandatory null-override reasoning, staged gate funnel). But the design predates this spec pack and is missing `prev_hash`/`row_hash`, `propensity`, `censoring`, `exploration_flag`, `candidate_set`, and a typed outcome vector — and it has never been connected to a real database (`InMemoryRunStore`, `RwLock<HashMap>` in `SuiteManager`).
**Options:**
  A. Extend the existing schema additively (new columns/tables on top of `backtest_runs` etc.) and wire it to Postgres for the first time. Cost: lower — reuses validated design decisions (the null library, the gate funnel) rather than re-deriving them.
  B. Treat this as building the spec's `trial`/`decision` tables fresh, migrating `backtest_runs`/`experiments`/etc. data (there may be none, if nothing has been written) into the new shape. Cost: higher, but avoids carrying forward any naming/shape mismatch between "Run" and the spec's "trial."
**Spec says:** §4.1 gives the `trial` table shape; silent on how to reconcile it with a pre-existing partial implementation.
**Recommendation:** A, with the naming kept as-is (`backtest_experiments`/`backtest_runs`) rather than renamed to match the spec's `campaign`/`trial` vocabulary, unless Mason wants the rename. Since nothing is being written today (in-memory only), there is likely no real data to lose either way — worth confirming before treating this as a migration at all.
**Status:** answered 2026-09-13 — implemented as A. `backtest_trials` is a *new* table alongside `backtest_runs` rather than columns on it (see ADR-P0-01); the Set J schema was kept and extended, not redesigned.

**Superseded 2026-09-13 by ADR-P0-12.** `backtest_trials` was replaced by the `mlops` ledger (`trial` + append-only chained `trial_event`, typed outcome vector) in its own `ledger` crate, because keeping lifecycle state on the trial row left INV-19 only partially enforceable.

### OQ-08 · The legacy `POST /api/backtests` UI route still dispatches compute with no trial row
**Blocking:** completing Phase 0 item 0.17 / INV-16. **This is the one invariant knowingly left partially enforced.**
**Context:** the agent-facing half of this bypass is closed — the `create_backtest` MCP tool is removed from every profile, its dispatch arm refuses the name, and `no_tool_profile_can_dispatch_an_unregistered_backtest` keeps it closed. The *human* half is still open: `POST /api/backtests` → `BacktestManager::create` → `backtest_jobs` runs a real simulation with no Experiment, no `delta_practical`, no propensity and no ledger row. The "Back Testing" tile UI depends on it.
**Why I did not just fix it:** the legacy path structurally cannot pre-register, because the user never declared a `delta_practical` — that declaration is the whole point of pre-registration, and inventing one after the fact is exactly the post-hoc rationalization §4.1 exists to prevent. CLAUDE.md §7 also says not to guess on anything involving gates or trial accounting, and this decides whether a class of historical runs counts toward `N_eff`.
**Options:**
  A. **Retire the route** and migrate the Back Testing UI onto the Experiment→Study path. Cost: real frontend work; the "quick one-off backtest" interaction disappears or becomes "create an experiment first," which is heavier than what the UI does today. Cleanest end state, and the one the spec clearly wants.
  B. **Register legacy dispatches under the sanctioned legacy marker** — `policy_id='legacy_unlogged'`, `propensity=NULL`, `delta_practical=0.0`, counted in `N_eff` but excluded from every off-policy estimator. This is the pattern CLAUDE.md §6 already prescribes for pre-existing history. Cost: `delta_practical=0.0` reads as "any improvement counts," which is arguably the honest encoding of *no declared threshold* but makes these trials look trivially passable if anyone reads the column without the marker. Keeps the UI working today and closes the hole.
  C. Leave as-is. Not really an option under the operating rules, but named so the cost of deferring is explicit: every one-off UI backtest is a look that `N_eff` does not see, which makes deflation *too lenient* — the direction that flatters results.
**Spec says:** INV-16, §4.2·1 — no bypass at any permission level. CLAUDE.md §6 covers the legacy-marker pattern for existing history but does not address an *ongoing* unlogged path.
**Recommendation:** B now, A when the frontend is next touched.
**Status:** answered 2026-09-13 (decided by me, at Mason's direction to choose and continue) — **B, implemented**, with one correction to the option as originally written.

`delta_practical` is now **NULL** for legacy rows, not `0.0`. Writing `0.0` would encode "any improvement counts" — a claim nobody made — whereas NULL encodes "never declared", which is the truth. The schema enforces the pairing exactly as it does for propensity: `CHECK (delta_practical IS NOT NULL OR policy_id = 'legacy_unlogged')`. So there are now exactly two ways to register, and no third: declare your effect size, or be explicitly marked legacy.

`BacktestManager::create` registers under `DispatchContext::legacy(...)` **before** the job is spawned, and `settle_trial` closes it on every terminal path (`Cancelled` → `right_cancel`, anything else non-completed → `failed`). A refused registration returns an error and dispatches nothing.

**INV-16 is now enforced across every dispatch path in the system.** Option A (retiring the route and moving the Back Testing UI onto the Experiment path) remains the better end state and is still worth doing when the frontend is next touched — B closes the correctness hole but leaves a second, weaker door open by design.

## Questions raised during the Phase 1–2 build (2026-09-14) — all answered under delegation

### OQ-09 · Adopt Temporal (ADR-001/002, §8, §10) on a stack that does not run it?
**Blocking:** 2.1, 2.2, 2.5.
**Context:** ADR-001/002 chose Temporal; nothing here runs it, or Kubernetes, or Ray. `crates/jobs` (ADR-0030) is a Postgres-backed durable job service with leases, manifest-hash idempotency, NATS→SSE events, fair-share queues, budgets and in-transaction trial registration. `mlops.campaign_event` already holds §10's phase states.
**Options:** A. Adopt Temporal now — cost: a new control plane, a worker fleet and a determinism discipline for ~200 lines of glue, on one machine. B. Event-source the campaign on `campaign_event` and drive it with a job-service job whose phase work is idempotent child jobs — cost: writing the driver; no new infrastructure. C. Hand-roll a workflow engine — the worst of both.
**Spec says:** "A campaign is a durable Temporal workflow" (§10). **Reference says:** `01-mlops-infra.md` §1.1(b) — a task queue plus explicit checkpoints is sufficient when steps are coarse.
**Decision:** **B** (ADR-P2-04). The three properties durable execution buys — crash recovery, exactly-once side effects, deterministic ordering — hold at phase granularity through the event fold and idempotent child jobs. Principal attribution already holds (the job service stamps `SubmittedBy` from the token). Spec §8/§10 amended (Appendix C). Revisit trigger: multi-machine drivers, sub-second sagas, or Kubernetes.
**Status:** answered 2026-09-14.

### OQ-10 · Ray + Kueue (2.3): what survives on one box?
**Decision:** N/A as systems (ADR-P2-05). The requirement that survives is the mandatory, default-less `max_gpu_hours` — now a REQUIRED manifest field for `Trainer`-class jobs, enforced by refusal at submission and a hard kill in the worker. Quota, priority and fairness are the job service's existing caps and queues.
**Status:** answered 2026-09-14.

### OQ-11 · Sample weighting declared `none` because none is applied (ADR-P1-01) — make it true, or leave it flagged?
**Decision:** make it true (ADR-P2-22, plan item 2.19). `uniqueness_weights` exists; emit `sample_weight` in the frame contract, consume it in every trainer adapter, then declare `uniqueness`. Leaving a truthful-but-permanent flag on every trial would train everyone to ignore the flag.
**Status:** answered 2026-09-14.

### OQ-12 · Overlapping-label leakage has no static detector — should one be built, or is the CV/WF flag the honest answer?
**Context:** the structural precondition (overlapping labels + a purge shorter than the horizon) is already refused by the `split_spec` CHECKs and the fold generator's `purge ≥ horizon`. What the flag catches is the *symptom* when the structure is not visible to the split — for example when a label's effective horizon is longer than its declared one.
**Decision:** the current position stands (ADR-P2-22 note). A static check that duplicated the CHECKs would give false comfort; the CV/WF gap is the correct detector for the undeclared case, and `inject::OverlappingLabelBlocks` stays as M10's labelled example so the learned tier can eventually catch what the rule cannot. The test that asserts "no static check catches this" stays, so the claim is kept honest.
**Status:** answered 2026-09-14.

### OQ-13 · `strict_v1` requires five years of history; the platform has months. Nothing can promote for years — is that the intent?
**Context:** CLAUDE.md §7 forbids guessing on gates; Mason has delegated the decision. P-01 is unambiguous that short backtests plus many trials cannot be distinguished from noise, and Gate 9 is the mechanism.
**Options:** A. Leave it — nothing reaches paper, so nothing accumulates the years honestly. B. Lower `strict_v1` — forbidden (immutable) and wrong. C. A second profile for paper deployment only.
**Decision:** **C** (ADR-P2-14): `paper_v1` = `strict_v1` minus the five-year calendar floor (MinBTL(N_eff) and ≥ 300 events kept) and minus Gate 16 (it cannot authorise capital). Live capital requires `strict_v1`, unchanged. The two are non-comparable by mechanism (built, AT-29). This is the spec's own remedy — "changing thresholds creates a new profile" — applied to a *purpose*, not a relaxation.
**Status:** answered 2026-09-14.
