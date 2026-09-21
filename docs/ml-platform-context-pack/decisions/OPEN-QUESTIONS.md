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

### OQ-14 · `jobs::manifest::canonical` deviates from RFC 8785 on integral floats, and its output is already hashed. Fix it, or pin the deviation?
**Context:** found 2026-09-15 by `canonical_json_agrees`, the reconciliation test added with S-1.19 — on its first run, which is what the test is for. RFC 8785 renders numbers by ECMAScript `Number::toString`, under which `1.0` is `"1"`, so `{"n": 1}` and `{"n": 1.0}` are one JSON value and must canonicalise identically. `serde_json` renders them `"1"` and `"1.0"`, and `jobs` takes that as-is.

**What it costs:** two callers that spell the same number differently get two different hashes for the same object. For a job manifest that means a duplicate job; for an idempotency key (S-1.19) it means a write that happens twice. Both are narrow — a schema types its fields, so a field is an integer or it is not, and nothing currently round-trips a number through a float and back — but neither is impossible.

**Options.**
**A. Fix `jobs`.** Correct by the RFC. It changes the canonical form of every job manifest, so every recorded manifest hash stops matching — the same failure shape as the sqlx migration-checksum divergence that produced `.gitattributes`, and that one cost a day.
**B. Pin the deviation and mirror it.** `crates/mcp-server`'s copy renders numbers exactly as `jobs` does, the reconciliation test holds them equal, and the deviation is documented at `idempotency::canonical_number` with a test that asserts `1` and `1.0` differ — so a change to either side is loud.
**C. Fix `jobs` behind a version field.** Manifests gain a canonicalisation version; old hashes verify under v1, new ones under v2. Correct and costly: a column, a migration, and a branch in the verifier for a defect nothing has yet hit.

**Decision:** **B for now**, taken because a mirror that disagrees with its original is worse than either version of the original, and because A's blast radius is every stored manifest. Recorded rather than closed: C is the right answer if a number ever does arrive both ways, and the test that asserts the deviation is what will say so.

**Status:** open — decision B in force, revisit if a duplicate key is ever observed.

### OQ-15 · The EDGE spread estimator is specified by AGENT-006 §5.2 and is not implemented — implement it from the paper, or drop it?
**Context:** found 2026-09-16 while building `analysis.estimate_liquidity_cost` (S-3.4). The spec names five cost estimators and EDGE (Ardia, Guidotti & Kroencke, 2024) is one of them. The first implementation here was written from memory of the method rather than from the paper, and it returned a *negative* spread variance against a reference series whose spread is known exactly (60 bps, set by `apps/skill-runner/reference_dataset.py`). A negative variance is not a hard-data refusal; it means the moment conditions were wrong.

**What it costs to get wrong:** a cost estimator that is biased low makes every strategy look profitable, and the error is invisible — the number is plausible, it has a citation behind it, and nothing downstream can tell it is too small. This is the same failure shape as a zero spread from a clamped Roll estimator, which the tool now refuses to produce.

**Options.**
**A. Implement it from the paper** and check it against the fixture, which now measures exactly this (`the_cost_estimators_recover_the_fixtures_known_spread`). EDGE is the current best single choice on both daily and intraday bars, so this is the only option that delivers what §5.2 asked for.
**B. Drop it from the spec.** The three that remain — Roll, Corwin-Schultz, Abdi-Ranaldo — recover the fixture's known spread to 58.4, 45.0 and 59.6 bps, and the consensus is a median, so a fourth estimator is an improvement rather than a necessity.
**C. Ship the approximation with a caveat.** Refused outright: a caveat in prose does not travel with the number into a cost model.

**Decision:** **A, deferred.** The tool ships with three verified estimators and its description says three; the docstring names EDGE and says why it is absent. Nothing is blocked on it — the consensus is a median of estimators that agree — and the calibration test means whoever implements it will know immediately whether they got it right, which is not true of the version that was nearly committed.

**Status:** open — `estimate_liquidity_cost` is correct and incomplete, and says so in its own description.

### OQ-16 · `max_usd_per_task` and `ask_above_usd` are declared in every profile and there is no price table anywhere in the platform. What are the prices, and who owns them?
**Context:** found 2026-09-16 building `estimate_cost` (S-3.19). `harness::profile::Budgets` carries `max_usd_per_task` (0.5 on the local profile, 25.0 on the frontier one) and `ask_above_usd`; `policy::decide` has a cost axis that fires when an estimate exceeds the threshold. The axis is correct and has never fired, because `drive.rs` passes `estimated_usd: None` — and it says so in a comment, honestly: *"No estimator yet (S-3.19) … `None` is 'not estimated', never 'free'."*

S-3.19's `estimate_cost` is supposed to supply that number. It cannot supply the dollars. A grep across the workspace finds **no price per token, per second, per GPU-hour or per job** — `jobs::models::estimate_cost` answers in *seconds* with a factor-of-three band, and nothing converts seconds or tokens into money.

**What it costs to get wrong:** a made-up rate is worse than none. `ask_above_usd` is a gate that pauses for a human, so a rate that is too low never pauses and a rate that is too high pauses on everything until somebody raises the threshold — and both failures look like the gate working. The same argument the code comment already makes about `None` versus zero applies one level up.

**Options.**
**A. A static price table in config**, per provider model (input/output per million tokens) and per hardware class (per second). Prices are public and change a few times a year; a stale entry is visible and correctable, and the table is one file to review.
**B. Read prices from the provider.** Correct for LLM tokens and unavailable for the local GPU, which has no invoice. It also makes a budget depend on a network call.
**C. Budget in tokens and seconds only**, and delete the dollar fields. Honest, and it loses the one unit a human actually reasons about when approving a spend.

**Decision:** none yet — this is a pricing question the platform owner should answer, not one to guess. Until then `estimate_cost` returns `est_seconds`, `est_rows` and `trials` for real, and `est_usd: null` beside a stated reason, so the tool is correct and incomplete rather than confident and wrong. `ask_above_usd` stays unfired and `policy.cost_above_threshold` stays unreachable, which is recorded here so it is not reported as enforced.

**Status:** open — needs a decision on where prices come from.

### OQ-17 · `get_venue_constraints` carries the venues' qualitative rules and none of their numeric limits. Who verifies the numbers?
**Context:** found 2026-09-16 building S-3.14. The platform's own research brief records, as verified on 2026-09-14, that Coinbase matches price-time with four self-trade-prevention modes, that Kraken's order rate limit *decays* and cancelling a younger order costs more, and that Alpaca's 2026 margin framework replaced the PDT rule with a minimum equity of 2,000 USD rather than 25,000. Those are in the table.

What is not in the brief, and not anywhere else in this repository, is a **number**: Kraken's counter maximum, its decay rate, the penalty schedule by order age, Coinbase's request budget, the four STP mode identifiers.

**What it costs:** §5.6's acceptance is "a requote rate exceeding Kraken's limit is refused before paper testing validates it", and that refusal needs a threshold. A threshold guessed too high lets a strategy be rate-limited live, having passed the check; one guessed too low refuses a strategy that would have worked. Both read exactly like a verified threshold to whoever calls the tool — which is the property that makes a guess worse than a stated gap.

**Options.**
**A. Verify them from the venues' current documentation** and record each with its own `verified_at`, which is what the table is already shaped for. The staleness machinery, the per-fact `verified` flag and the coverage note all exist; only the values are missing.
**B. Read them from the venues' APIs** where exposed. Kraken returns rate-limit headers; that makes the table self-updating for one venue and leaves the others.
**C. Drop the numeric acceptance criterion** and keep the tool qualitative — it still catches the failure that matters most, which is a market-making backtest that assumes cancels are free.

**Decision:** none yet. The table ships with the mechanism (versioned facts, a 90-day staleness bound that reports itself in the summary) and every numeric field flagged `verified: false`, asserted by `no_numeric_limit_claims_to_be_verified` so a later edit cannot quietly promote a guess. A is the right answer and it needs someone to read three sets of exchange docs, which is not a thing to infer.

**Status:** open — `get_venue_constraints` is correct, useful and incomplete, and says so in its own output.

### OQ-18 · A verified finding has no data window, and `data_window` is mandatory. Where does it come from?
**Context:** found 2026-09-16 wiring `record_finding` to the store built for S-4.3. The loop already does the hard half: `memory::verify_finding` checks every cited id against what tools actually returned and labels the claim `established` or `suggestive` (S-4.2, tested and passing). That verdict then lives for exactly one run — `WorkingState` carries it, the turn ends, and the platform forgets what it checked.

Persisting it needs an `Entry`, and `Entry::validate` refuses one without a data window, correctly: AGENT-007 §2.2 makes `data_window {start, end}` a shared field of every kind, and §2.2.1's near-duplicate check is "same subject, same measured quantity, **overlapping window**". The window is not decoration; it is one of the three keys the conflict machinery joins on.

**The loop does not have one.** `drive.rs` tracks no window, `Task` has no window, and `WorkingState` has none. Neither does the loop have a `run_id` — the driver owns that — so `provenance` is half-absent too, though that half is mechanical to pass down.

**What it costs:** a wrong window is worse than a missing one. Two findings about different periods that both claim the same window will be offered to the near-duplicate check as candidates and to §2.2.1 as a possible contradiction; two findings about the same period carrying different windows will never be compared at all. Either way the conflict surface — the thing P5 says must be surfaced and never merged — silently stops working, and nothing about the stored row looks wrong.

**Options.**
**A. Derive it from the cited evidence.** The results in `results_by_id` often carry a window (`get_bars`, a backtest, every `analysis` template stamps one in `detail.window`). Take the union, or the intersection, of the windows of the cited results. This is the most faithful — the claim rests on those results and on nothing else — and it needs a rule for the case the spec does not cover: three cited results with three different windows, and a claim that is about all of them or about one.
**B. Make the model state it.** Add a required `data_window` argument to `record_finding`. Honest about provenance and it moves a factual matter into the model's prose, which is the class of thing AGENT-011 keeps pulling *out* of free-form strings — and an unverifiable window is exactly the field a model under pressure will fill with the widest range it can.
**C. Carry the run's window.** Give the run a declared research window at admission and stamp every finding with it. Simple, always correct at run granularity, and wrong at claim granularity the moment one run looks at two periods — which is what a comparison workflow does by definition.

**Decision:** none yet, and nothing is implemented. An `Effect::Remember` variant was written and then backed out rather than left emitting entries stamped with a guessed window: a stored finding with a plausible wrong window is undetectable afterwards, and it degrades the conflict check rather than failing it. A is most likely right and its tie-breaking rule is the part that needs deciding.

**What holds meanwhile:** `MemoryStore` and `/api/memory/{bucket}` are built and tested, `verify_finding` keeps labelling, and findings still reach the next turn through `WorkingState`. What does not happen is persistence across runs — so phase 4's "the harness computes and nothing persists" is now "the harness computes, the store exists, and the last wire is a decision away".

**Narrowed 2026-09-16.** It is a question about **findings**, not about every kind. `user_preference` has a natural answer and is now built on it: the claim is "the user said this", which is true as of the moment they said it, so the window is that day to that day — a fact rather than a guess (ADR-S4-01). The same reasoning covers `platform_defect`, whose claim is about the platform on the day it was observed. What has no answer is the kind whose claim is *about a period of market data*: a finding, a negative result, an instrument profile.

**Also narrowed by what shipped around it.** `memory_retrievals.cited` no longer waits on this — S-4.5's log is built and verified, because a retrieval row names an entry id and needs no window of its own. What still waits is `record_finding` writing through, which is the one path where the window is the claim's own subject.

**Resolved 2026-09-16 as ADR-S4-04**, by option A with its tie-break settled: the window is the **union** of the windows of the cited results. The union rather than the intersection because the intersection of disjoint windows is empty — leaving the claim most in need of a window carrying none — and because the union is the wider bound, so more pairs reach §2.2.1's conflict check, and P5 says conflicts are surfaced rather than merged.

One thing found while implementing it, and it is the reason this is not yet a refusal: **`run_backtest` does not stamp a window**, while the analysis templates, `get_bars` and `preview_signals` all do. A backtest is the most important evidence a finding can cite, so refusing an underivable window today would reject the central case. The finding is therefore recorded for the run exactly as before and says out loud that it did not persist. Making every evidence-producing tool stamp a window turns this into a refusal in one line.

**The follow-up is done too.** `shape_snapshot` now stamps `window: {start, end}` on every backtest result, in the shape every other producer uses. The snapshot had carried `start` and `end` at the top level all along and that is not the same thing — a bare `start` among twenty fields is not a period a reader may assume, and the derivation deliberately refuses to read one, because it could as easily belong to a lease or a schedule. The requested ends are used and `coverage_warning` keeps flagging a run that covered less than it asked for: the window is what the claim is about, the coverage is how well it was measured.

So every evidence-producing tool now stamps a window, and making `None` a refusal is a small change rather than a breaking one. It is left as a decision rather than taken silently.

**Status:** resolved.

### OQ-19 · `preregistration_required` is a profile value in the build plan and a contradiction of AGENT-013 §11's own table. Which is meant?

**Raised 2026-09-17**, implementing S-10.3's remaining half. Nothing is built for
it, in either direction.

**The plan's sentence.** S-10.3 says the ticket path "refuses a request carrying
no `prereg_id` for the run with `preregistration_required`", which reads as a
profile value: some configurations require a run-level pre-registration and some
do not.

**§11 does not contemplate a tier that skips it.** The per-tier table gives every
tier a pre-registration *authoring mode* — `free` on frontier and `local_high`,
a grammar-filled skeleton on `local_mid`, a skeleton derived from the question's
constraints with the model confirming it on `local_small` and `degraded`. Three
ways of writing one, and no row saying "none". A profile value that switched
pre-registration off would therefore be a fourth mode the table does not have,
available at every tier including frontier.

**What is already built is stricter than either.** `sealed_registration` in
`local_driver` gives `Prereg::NotRequired` to a `TaskDemand::SingleLookup` and
`Prereg::Missing` to everything else, so a Chat lookup needs none and every
Research run does. That is safe, it satisfies INV-16, and it is the behaviour
today.

**Why this is not a small call.** Pre-registration is where the falsifier is
written, and the falsifier is the field `agent_lifecycle.preregistration` exists
for. A configuration that skipped it would spend trials against a hypothesis
that could only ever be confirmed — which is the specific failure §7.2 was
written to close. INV-16 does not stop it: `mlops.trial.prereg_hash` predates
AGENT-013 and a per-Experiment hash would satisfy the constraint while carrying
no falsifier at all. So the flag would be legal and would still remove the thing
that matters.

**Options.**

**A. There is no such profile value; the requirement is the task demand.** A
lookup does not pre-register, everything else does, at every tier. This is what
is built. It makes the plan's sentence a description of the *demand* check
rather than of a profile field, and costs nothing — no tier loses a workflow,
because §11 already gives each one a way to author a registration it can manage.

**B. It exists and defaults to `true`.** Honours the plan's literal wording. The
cost is that the value's only purpose is to be set to `false`, and the first
configuration that does gets a tier spending trials with no falsifier. A default
nobody changes is a field that does nothing; a default somebody changes is the
door §7.2 closed.

**C. It exists and is not a boolean but the §11 authoring mode.** The field the
table actually implies —
`preregistration: free | skeleton_grammar | skeleton_confirm` — with no variant
meaning "none". This is S-10.14's remaining item and is being built regardless,
which is part of why the boolean looks like a restatement of it rather than a
second decision.

**Leaning: A, with C built beside it**, because C is what §11 asks for by name
and A is what the gate already does. But B is the literal reading of the plan,
and a profile field is not something to add or omit on inference — the whole
point of the field would be that somebody can set it.

**What holds meanwhile:** every Research run must have sealed a registration
before any trial-spending tool is admitted, and a read that fails is treated as
`Missing` rather than `NotRequired` so an outage cannot open the gate.

**Status:** open. `max_prereg_amendments` — the other profile value S-10.3 names
— has no such tension and is built, because a cap on amendments has a
conservative direction and this does not.

### OQ-20 · The trial gate registers at the tool call; `mlops.trial` describes compute. The route between them was never built, and every trial-spending tool is refused.

**Found 2026-09-17**, tracing `TrialRequest.prereg_hash` while wiring the workflow
onto a ticket. Nothing is implemented in either direction.

**The failure, which is live.** `gate::ApiTrialRegistrar::register` posts to
`POST /api/ledger/trials`. **That route does not exist** — the only occurrence of
the string in the repository is the `post` call itself. A 404 is an `Err`, which
becomes `registration_failed`, which `CallGate::admit` returns before
`dispatch_untraced` is reached. So every tool declaring a trial is refused at the
gate, on every path: the internal agent goes through `bridge::execute` →
`dispatch_tool` → `admit`, and so does the MCP front door.

That is all seven of them — `backtest_strategy`, `rerun_backtest`, `run_sweep`,
`run_study`, `advance_gate`, `report_kill`, `report_evaluator_defect` — which is
the entire research loop. The platform can plan, author and pre-register, and
cannot run anything that counts.

`affbe59` built the gate, its three adversarial tests and the structural test that
the dispatch table has exactly one call site behind it. All of them pass, because
they test the refusal path and a registrar that refuses is exactly what they
assert. The caller was built and the callee never was — this repository's dominant
defect shape, inverted.

**Why the obvious repair does not work.** `mlops.trial`'s required columns are
`config_hash`, `config`, `dataset_id`, `code_hash`, `image_digest` and `seed_set`.
Every one is a fact about **compute**: which configuration ran, over which data,
from which image, under which seeds. A tool call has not chosen any of them —
`run_sweep` is dispatched with arguments, and the sweep decides the configs and
the data slice afterwards. Writing the route means inventing a `dataset_id` and a
`config_hash` for a look that has not looked at anything yet, and a
`dataset_id` of `"pending:run_sweep"` is a placeholder that reads as data
everywhere downstream.

**And the compute site already registers.** `backtest::run` calls
`ledger.register` per Run (`run/mod.rs:174`), with a real subject from
`trial_subject(cfg)`. INV-16 — "no compute is dispatched without a trial row in
state REGISTERED" — is satisfied there, at the only place the required facts
exist.

**So the two designs differ in what a trial *is*.**

**A. The gate registers, and `mlops.trial` grows a shape for a call.** Matches
`affbe59`'s stated intent ("writes the ledger row"). Costs: a second grain in one
table — one row per tool call plus N rows per Run — so `trials_counted`, the
exploration floor's denominator and `N_eff` all change meaning, and every
selection-adjusted statistic on this platform reads one of those. It also needs
the required columns to become optional for one grain, which weakens them for the
other.

**B. The gate *verifies* rather than registers.** `admit` checks that the run may
spend (it already does: the chat-turn rule, the survey rule, the pre-registration
rule), counts the call against `max_calls_per_run` (it already does), and lets the
compute site write the row it alone can write honestly. `TrialRegistrar` becomes a
reservation or disappears. Costs: `admit` no longer guarantees a row exists at
dispatch — a tool that dispatches and never reaches compute would be a look with
no trial, which is the thing the gate was written to prevent.

**C. The gate registers a *reservation* in its own table**, settled or abandoned
by the compute site. Keeps both properties and adds a table and a reconciliation
nobody has asked for yet.

**Leaning: B**, because the required columns are evidence rather than opinion —
`mlops.trial`'s shape says a trial is a unit of compute, and it has said so since
`0043`. The gate's job is admission, and admission is a different question from
registration. But B gives up a property `affbe59` deliberately bought, and giving
it up is not something to do as a side effect of fixing a 404.

**What holds meanwhile: nothing.** This is not a gap with a safe fallback — the
research loop does not run. It is written up rather than patched because the
patch is the design decision, and picking one to unblock a build is how the trial
counter would come to mean something nobody chose.

**Resolved 2026-09-17 as ADR-S10-10, and by neither option A nor B.**

Both fail on a fact. A cannot be written honestly — every required column of
`mlops.trial` is about compute and a tool call has chosen none of them. B
under-counts silently: `report_kill` and `report_evaluator_defect` declare a trial
and post to `/api/falsification/kills`, which registers nothing, so two of the
seven tools would spend nothing at all.

Which made the real shape visible: these are **two questions**, and only one of
them ever had a table. *What compute ran, under what config, over what data* is
`mlops.trial`, written by `backtest::run` where the facts exist. *May this run
spend, and has it spent one* is `mlops.trial_reservation` (migration `0132`),
written by the gate at the door. The gate posts to `/api/ledger/reservations` now
and the route exists.

The choice is additive, which is most of the argument for it: `mlops.trial` keeps
its shape, `trials_counted`, the exploration floor and `N_eff` all go on reading
it untouched, INV-16 stays satisfied where the facts are, and the gate keeps the
property `affbe59` bought — a look is recorded before a handler is reached.

Two things it fixed on the way. `prereg_hash` now reaches a durable row per look:
it was carried on `TrialRequest` and left out of the body the registrar posted, so
§7.2's "every ticket carries the hash" was true of no ticket. And the receipt
returns `reservation_id` rather than `ticket_id`, because a reservation is
permission and a ticket is a measurement, and a model citing the first as the
second would be citing that it was allowed to look.

**What it does not do:** settlement. `settled_trial_id` exists and nothing sets it,
so an authorised look that never ran is not yet distinguishable from one that did.
Threading the reservation id through tool → API → suite → Run is a separate change,
and no reconciliation is claimed until it lands.

**Status:** resolved.


---

## Q-S11-01 — AGENT-014 names two different mechanisms for one child

**Raised 2026-09-21 during S-11.1. Implemented under ADR-P11-01; the spec text is
what needs a decision, not the build.**

AGENT-014 §3: "each child is created through `JobStore::submit_child_once` keyed
on `(session_id, agenda_item_id)`".

AGENT-014 §10: "**Job**: `JobKind::ContinuousResearch`; children via the
conversation/turn path with `session_id` and `agenda_item_id` on `agent_runs`".

These are two mechanisms and a child is created once. A conversation turn is not
a job: it is started by `AgentManager::send_message`, runs on a spawned task, and
its identity is a row in `agent_runs`, not in the job store. `submit_child_once`
keys on `(parent job, member index)` and hands back a job — it has nothing to
hand back for a turn.

**What was built** follows §10, because that is the path that exists and the one
the desk, the rail and the steer controls already read. The exactly-once property
§3 asks for is delivered by the derived conversation id and the conditional
claim; see ADR-P11-01 and `api::session_driver`'s header.

**What is still open:** whether §3's sentence should be amended to state the
property rather than the mechanism, or whether the intent was a job-backed child
after all — in which case the *worker* for that child is unbuilt and §10's
sentence is the one that is wrong. This is a spec edit either way, and it is not
one to make while implementing the task it is about.

**What holds meanwhile:** the property. A driver killed at any point between
claiming and recording re-derives the same conversation, finds the same run, and
appends `run_started` for the run that exists — asserted by
`restarting_between_start_and_record_re_derives_the_same_state` and by
`a_child_conversation_id_is_the_same_one_after_a_restart`.

**Status:** open (spec wording); the build is not blocked.

---

## Q-S11-02 — §7.1's cost-drift rule is inert at the moment its own gate opens

**Raised 2026-09-21 building S-11.8. Implemented under one reading, with the
proof kept as a test; the spec sentence is what needs deciding.**

AGENT-014 §7.1: *"**cost drift**: `cost_per_knowledge_unit` over the last K
exceeds 3× the session's own median (only once ≥ 2K runs exist)."*

Read literally — the median taken over **every** run, including the K being
judged — the rule cannot fire at `2K` runs, which is the exact point its own gate
opens. At `2K` the window is half the session, so the median falls between the
cheap half and the dear half: with K cheap runs at `c` and K dear runs at `d`,
the median is `(c + d) / 2`, and `d > 3 · (c + d) / 2` reduces to `-d > 3c`,
which has no solution for positive costs. It does not matter how dear the window
is — ten times, a thousand times, a million times the baseline all fail.

**What was built** takes the median over the runs **before** the window — still
"the session's own", not a platform-wide figure — so the rule is live from `2K`
onward as the gate intends. The proof of the literal reading's inertness is kept
as `the_baseline_is_the_history_because_a_window_inside_its_own_median_can_never_drift`
in `crates/research-session/src/stall.rs`, over three orders of magnitude.

**What is still open:** which reading §7.1 means. They differ by an entire class
of alarm — the literal one has no cost-drift rule until roughly `2.5K` runs, and
a session that never reaches `2.5K` runs never has one at all. This is a
one-expression change in `cost_drift` either way, and the sentence should say
which rather than leaving the next reader to re-derive it.

**What holds meanwhile:** the other three rules. They have absolute thresholds
and self-gate, so a stalling session is still caught by barren streak,
degenerating or circling — cost drift is the only one this affects.

**Status:** open (spec wording); the build is not blocked.

---

## Q-S11-03 — AGENT-012 §6.2 asks for four capped budget bars the data cannot fill

**Raised 2026-09-21 finishing S-6.13. Implemented as "no cap recorded"; the spec
sentence is what needs deciding.**

AGENT-012 §6.2: *"Four stacked bars (steps, tokens, $, trials) with the cap, the
90% wrap-up mark, and the projected end at current burn."*

The only budget shape the desk receives is `BudgetView`, which mirrors
`crates/harness/src/state.rs` and carries `steps_total` plus **three bare
remainders**. Three of the four bars therefore have no cap to draw against, and
there is no burn series to project an end from.

The previous code filled the gap by inventing one: `tokensCap = caps?.tokens ??
budget.tokens_remaining`, which makes `used = cap − remaining = 0`, so a run that
had burned half a million tokens rendered **"0 / 40,000"**. The trials bar did it
in reverse — `cap = max(spent, 1)` printed **"4 / 4 · 90%+"**, a run reported as
out of trials when nothing had said how many it had.

**What was built** renders an uncapped bar as `N remaining · no cap recorded`
over a hatched track with no `aria-valuenow`, and shows no projected end. Four
bars still render, so an uncapped one is visibly present rather than quietly
dropped.

**What is still open:** either the harness sends caps for tokens, dollars and
trials — they exist in the profile's `Budgets` — or §6.2 stops promising four
capped bars and a projection. Inventing a cap from a remainder is the one answer
that is not available, because it renders a number that is not a measurement.

**Status:** open (spec wording, or a harness field); the build is not blocked.

---

## Q-S5-01 — AGENT-011's exit units and ADR-0010's canonical form cannot both hold

**Raised 2026-09-21 building S-5.6. Neither reading implemented.**

AGENT-011 §3: *"Units are explicit: `exit stop 1.5 atr from entry`, `exit target
2 % from entry`"*. AGENT-011 §9: *"Changing the canonical JSON storage form —
ADR-0010 stands."*

`domain::strategy_def::exits::ExitRule::StopLoss` carries `fraction: Decimal`,
documented as *"Always expressed as a positive fraction of entry, never as an
absolute price"*. An ATR- or sigma-scaled distance is a **per-bar** quantity;
there is no fraction of entry that means it. So either `ExitRule` gains a unit —
which contradicts §9 — or the language drops two of its three units, which
contradicts §3 *and* S-5.5's acceptance, whose whole point is that the error
names all three.

**What was built implements neither.** The parser accepts all three units, so
S-5.5's acceptance is met literally; the compiler refuses `atr` and `sigma` with
`unit_not_representable`, naming the schema field that cannot hold them. A model
gets a true sentence about why, rather than a silent conversion into a fraction
that would mean something else.

**Status:** open. The decision is whether `ExitRule` gains a unit.

---

## Q-S5-02 — `crossed_above` has no compilation target, and the sentence justifying it describes something that is not there

**Raised 2026-09-21 building S-5.6. Refused by name rather than approximated.**

AGENT-011 §3: *"`crossed_above/below`, `rising/falling(x, n)` … are
compiler-provided; the rising-edge semantics the v1 signal node had become
explicit."*

**The v1 signal node has no rising-edge semantics.**
`strategy-runtime::interpreter::evaluate_signals` and `runtime::run_signals` both
evaluate the condition on the current bar and emit — there is no previous-value
register — and `bytecode::Op` has no `CrossedAbove` (FEAT-004 §3.3 lists it as an
unbuilt extension). The feature engine cannot supply it either:
`features::registry`'s naming scheme is `family_p1[_p2…]` over integer
parameters, so there is no lag *of a derived feature*.

This makes AGENT-011's own headline example half-uncompilable: `entry long when
crossed_above(ema(close,7), ema(close,21)) and rsi(close,14) < 60`.

**What was built** refuses `crossed_above` with `needs_history`, pointing at the
level comparison that does compile (`macd(close, 7, 21) > 0`) and at FEAT-004
§3.3. Approximating a cross with a level test would compile the example and mean
something else, which is worse than refusing it.

**Status:** open. Either the runtime gains a previous-value register and an op,
or §3 stops listing cross detection as compiler-provided.

---

## Q-S7-01 — COMP-007 says a hard-stale feed halts and also that it scales down

**Raised 2026-09-21 building S-7.7. Neither reading changed; the existing
behaviour is §4's, and a test's *name* asserts §3's.**

COMP-007 §3, the Data row:

> *"staleness > 3 × timeframe → strategy warning; > 10 × → **strategy halt**"*

COMP-007 §4, the response table:

> *"breach (behaviour/execution/**data**) | **scale down** to 25% size for that
> strategy"*

The code follows §4: `state::on_sustained` halts only on `Family::Pnl`. The test
covering it is named `a_hard_stale_feed_halts_without_waiting_three_cadences` and
asserts `StrategyState::ScaledDown`, with a comment reading "a data fact scales
rather than halts the book".

**So the test's name says one thing and its assertion says the other**, and the
spec can be read either way. That is worse than a plain contradiction, because a
reader who greps for the behaviour finds the name and stops.

Nothing was changed. This needs somebody with authority over COMP-007 to pick
one, and then the test renamed to match whichever it is.

**What holds meanwhile:** a hard-stale feed scales a strategy to 25% immediately,
without the three cadences a sustained breach needs. It is the weaker of the two
readings, on the family whose facts are not estimates.

**Status:** open.

---

## Q-S3-01 — the spec and the plan disagree about whether `regime_failure_search` spends a trial

**Raised 2026-09-21 building S-3.12. Neither implemented.**

AGENT-006 §5.5's table gives `regime_failure_search` trial **`none`**.
`docs/plans/plan-sets/set-S/phase-3-catalogue.md`'s S-3.13 says *"All spend
`Falsification` trials."*

That is a disagreement about a `spends_trial` declaration, which is a correctness
bug in either direction and not a metadata slip. A spending tool declared free is
an **unrecorded look** (INV-1); a free tool declared spending charges research
for nothing and raises `N_eff` against work nobody did — and every
selection-adjusted statistic on this platform divides by that count.

The tool was not built. Whoever builds S-3.13 must settle this **at the
declaration**, not in the handler.

**Status:** open.

---

## Q-S3-02 — §5.5 assigns a tier that cannot serve the tools it assigns it to

**Raised 2026-09-21 building S-3.12. Built typed instead, and the reason is
recorded here rather than only in the code.**

AGENT-006 §5.5 gives `trade_reshuffle_bootstrap` and `cost_shock_curve` tier
**`P`** (templated), and §9's build-order table repeats it. A template's rows
arrive only as `SeriesKind::Ohlcv` or `SeriesKind::TrialReturns`
(`crates/skills/src/template.rs:98`). **Neither carries per-trade `pnl` or
`costs_paid`**, which both tools need and which §5.5's own signatures imply.

Both were built typed. P6 makes the tier invisible to the model, so the contract
is unchanged — but the spec names a tier that cannot implement the tool it names,
and the next person following §5.5 literally will lose a day to it.

**Status:** open (spec wording).

---

## Q-S5-03 — §4's worked example of the flagship suggestion does not compile

**Raised 2026-09-21 building S-5.11. Neither reading implemented.**

AGENT-011 §4, the `suggest_moves` row: *"maps `get_diagnostics` findings ("loses
in high-vol months") to candidate moves ("add_regime_gate on
`rv(20) pct_rank < 0.8`")"*.

That predicate is **a feature of a feature**, and the canonical expression
grammar's only named value is `feature('name')` — a name, not an expression. So
the spec's illustration of the platform's flagship suggestion is a document the
compiler refuses. `skeleton.rs`'s `VOL_TARGET_NOTES` already records the same
limit for `pct_rank(rv(close, 20), 200)`.

**What was built** never invents a regime predicate: `suggest_moves` takes one
from the caller as a `Predicate` checked against the catalogue at construction,
or withholds the move naming what is missing.

**Status:** open. Either the grammar gains nested features, or §4's example
changes.

---

## Q-S5-04 — §4 defines the blocking profile's second number twice, as two different quantities

**Raised 2026-09-21 building S-5.15. The order-independent reading was built and
named so it cannot be mistaken for the other.**

AGENT-011 §4 says *"for the conjunction, the share on which it was the **last**
clause to fail"*, then two sentences later *"was the **sole blocker** on 91% of
the rest"*.

"Last clause to fail" presupposes an evaluation order. The compiled condition is
`(A * B)`, which evaluates both operands and has **no failure order**, so that
reading has no referent on this platform.

The field is `bars_sole_blocker`, named so nobody can read it as the other.
Phase 5's S-5.15 body text carries the same double wording and needs the same
fix.

**Status:** open (spec wording).

---

## Q-S4-01 — AGENT-009 §3's case format and its `judgement` row cannot both hold

**Raised 2026-09-21 building the judgement suite. Neither implemented literally.**

§3's suite table says cases live at `evals/<suite>/<case>/` with an
**`expected.json`**. §3's `judgement` row says those cases carry *"planted ground
truth"* the agent **must not see**.

`expected.json` **is** the answer key, written to a path on a filesystem the
agent reaches. The agent has `code_execution` on some profiles and a workspace on
all of them, so a file saying "this one is a null" is one `cat` away from making
every number on the scorecard a number about file reading.

**What was built** is neither: no fixture directory and no in-repo truth file.
The corpus is **generated from a seed**, so the key exists only in the memory of
the process that generated it. That satisfies the row's intent and contradicts
the table's format.

Flagged because a future agent reading §3 top-down will build `evals/judgement/**`
with `expected.json` files and undo it.

**Status:** open. The table's format needs amending, or the `judgement` row needs
an exemption from it.

---

## Q-S4-02 — §3 asks the judgement suite to run on replay, where its numbers cannot mean anything

**Raised 2026-09-21. Not implemented.**

AGENT-009 §3's `judgement` row: *"replay where the tape is fixed, live nightly
otherwise"*.

A replayed tape gives the same reply every time. A miss rate measured over replay
cases therefore measures **the tape**, not the tier — it has no sampling error
and no content. The two rates only mean something from live runs.

No replay path was built for this suite. `SuiteKind::Replay` for `judgement`
would produce a number that looks like a measurement and is not one, which is the
failure this whole suite exists to detect one level down.

**Status:** open.

---

## Q-S4-03 — §4.2's sealed 20% hides the cases, not the answers, when the corpus is generated

**Raised 2026-09-21. Both readings left open; the mechanism is wired for either.**

§4.2 seals 20% of a suite's cases as a holdout. Applied to a corpus whose answers
are **regenerable from a seed**, a hash split hides which cases are sealed and
hides nothing about their answers — anyone holding the seed holds the key to all
of them.

The anti-overfitting defence that would actually work here is a **different seed**
for the sealed run, not a split of one corpus. `PlantedCorpus::holdout_input()` is
wired so `Holdout::draw` works over these ids either way.

Whoever builds the nightly promotion has to pick one. Recorded rather than
decided, because the two defend against different things and the choice is a
policy about what a sealed score is for.

**Status:** open.
