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

**Status:** answered 2026-09-21 (decided under Mason's delegation of 2026-09-21 — ADR-D-01). **B stands, and C is
pre-authorised on the trigger.** The mirror is the thing that must not disagree with
its original, so B is right today; and C is not a decision anybody should have to
re-make under pressure, so it is taken now: **the first time a number is observed
arriving both ways, `jobs` gains a canonicalisation version and the verifier gains
its branch.** `canonical_number`'s deviation test is the trigger, and it is wired to
say so in its failure message. Nothing to revisit — the condition is the decision.

### OQ-15 · The EDGE spread estimator is specified by AGENT-006 §5.2 and is not implemented — implement it from the paper, or drop it?
**Context:** found 2026-09-16 while building `analysis.estimate_liquidity_cost` (S-3.4). The spec names five cost estimators and EDGE (Ardia, Guidotti & Kroencke, 2024) is one of them. The first implementation here was written from memory of the method rather than from the paper, and it returned a *negative* spread variance against a reference series whose spread is known exactly (60 bps, set by `apps/skill-runner/reference_dataset.py`). A negative variance is not a hard-data refusal; it means the moment conditions were wrong.

**What it costs to get wrong:** a cost estimator that is biased low makes every strategy look profitable, and the error is invisible — the number is plausible, it has a citation behind it, and nothing downstream can tell it is too small. This is the same failure shape as a zero spread from a clamped Roll estimator, which the tool now refuses to produce.

**Options.**
**A. Implement it from the paper** and check it against the fixture, which now measures exactly this (`the_cost_estimators_recover_the_fixtures_known_spread`). EDGE is the current best single choice on both daily and intraday bars, so this is the only option that delivers what §5.2 asked for.
**B. Drop it from the spec.** The three that remain — Roll, Corwin-Schultz, Abdi-Ranaldo — recover the fixture's known spread to 58.4, 45.0 and 59.6 bps, and the consensus is a median, so a fourth estimator is an improvement rather than a necessity.
**C. Ship the approximation with a caveat.** Refused outright: a caveat in prose does not travel with the number into a cost model.

**Decision:** **A, deferred.** The tool ships with three verified estimators and its description says three; the docstring names EDGE and says why it is absent. Nothing is blocked on it — the consensus is a median of estimators that agree — and the calibration test means whoever implements it will know immediately whether they got it right, which is not true of the version that was nearly committed.

**Status:** answered 2026-09-21 (decided under Mason's delegation of 2026-09-21 — ADR-D-02). **A: implement EDGE from
the paper.** Not deferred any longer. The reason it was deferred — nothing is blocked,
the consensus is a median of three — is exactly why it is cheap to do properly now, and
the calibration fixture (`the_cost_estimators_recover_the_fixtures_known_spread`, a
known 60 bps) means a wrong implementation announces itself on the first run rather
than biasing every strategy's costs invisibly. Option C stays refused: a caveat in
prose does not travel with the number into a cost model.

### OQ-16 · `max_usd_per_task` and `ask_above_usd` are declared in every profile and there is no price table anywhere in the platform. What are the prices, and who owns them?
**Context:** found 2026-09-16 building `estimate_cost` (S-3.19). `harness::profile::Budgets` carries `max_usd_per_task` (0.5 on the local profile, 25.0 on the frontier one) and `ask_above_usd`; `policy::decide` has a cost axis that fires when an estimate exceeds the threshold. The axis is correct and has never fired, because `drive.rs` passes `estimated_usd: None` — and it says so in a comment, honestly: *"No estimator yet (S-3.19) … `None` is 'not estimated', never 'free'."*

S-3.19's `estimate_cost` is supposed to supply that number. It cannot supply the dollars. A grep across the workspace finds **no price per token, per second, per GPU-hour or per job** — `jobs::models::estimate_cost` answers in *seconds* with a factor-of-three band, and nothing converts seconds or tokens into money.

**What it costs to get wrong:** a made-up rate is worse than none. `ask_above_usd` is a gate that pauses for a human, so a rate that is too low never pauses and a rate that is too high pauses on everything until somebody raises the threshold — and both failures look like the gate working. The same argument the code comment already makes about `None` versus zero applies one level up.

**Options.**
**A. A static price table in config**, per provider model (input/output per million tokens) and per hardware class (per second). Prices are public and change a few times a year; a stale entry is visible and correctable, and the table is one file to review.
**B. Read prices from the provider.** Correct for LLM tokens and unavailable for the local GPU, which has no invoice. It also makes a budget depend on a network call.
**C. Budget in tokens and seconds only**, and delete the dollar fields. Honest, and it loses the one unit a human actually reasons about when approving a spend.

**Decision:** none yet — this is a pricing question the platform owner should answer, not one to guess. Until then `estimate_cost` returns `est_seconds`, `est_rows` and `trials` for real, and `est_usd: null` beside a stated reason, so the tool is correct and incomplete rather than confident and wrong. `ask_above_usd` stays unfired and `policy.cost_above_threshold` stays unreachable, which is recorded here so it is not reported as enforced.

**Status:** answered 2026-09-21 (decided under Mason's delegation of 2026-09-21 — ADR-D-03). **A: a static price table
in config**, per provider model (input/output per million tokens) and per hardware
class (per second), each row carrying `source_url` and `verified_at`, with the same
90-day staleness bound the venue table already uses.

**And the part that was actually blocking: an unverified price is still better than no
gate, for this gate only.** `ask_above_usd` *pauses for a human*. Its failure modes are
asymmetric — pausing too often costs annoyance, never pausing costs unbounded spend —
so `policy.cost_above_threshold` fires on an unverified row and says in the pause which
row it used and when that row was last checked. This is the one place in the platform
where an approximate number beats an absence, and it is bounded: it cannot spend, it can
only stop.

**Everywhere else the refusal stands.** `estimate_cost` returns `est_usd: null` for any
model not in the table, and a stale row (> 90 days) is treated as absent for reporting
while still arming the pause. No price is ever inferred from another model's price, and
`max_usd_per_task` is enforced only against a fresh row — a budget cap that cannot be
computed must not silently pass.

### OQ-17 · `get_venue_constraints` carries the venues' qualitative rules and none of their numeric limits. Who verifies the numbers?
**Context:** found 2026-09-16 building S-3.14. The platform's own research brief records, as verified on 2026-09-14, that Coinbase matches price-time with four self-trade-prevention modes, that Kraken's order rate limit *decays* and cancelling a younger order costs more, and that Alpaca's 2026 margin framework replaced the PDT rule with a minimum equity of 2,000 USD rather than 25,000. Those are in the table.

What is not in the brief, and not anywhere else in this repository, is a **number**: Kraken's counter maximum, its decay rate, the penalty schedule by order age, Coinbase's request budget, the four STP mode identifiers.

**What it costs:** §5.6's acceptance is "a requote rate exceeding Kraken's limit is refused before paper testing validates it", and that refusal needs a threshold. A threshold guessed too high lets a strategy be rate-limited live, having passed the check; one guessed too low refuses a strategy that would have worked. Both read exactly like a verified threshold to whoever calls the tool — which is the property that makes a guess worse than a stated gap.

**Options.**
**A. Verify them from the venues' current documentation** and record each with its own `verified_at`, which is what the table is already shaped for. The staleness machinery, the per-fact `verified` flag and the coverage note all exist; only the values are missing.
**B. Read them from the venues' APIs** where exposed. Kraken returns rate-limit headers; that makes the table self-updating for one venue and leaves the others.
**C. Drop the numeric acceptance criterion** and keep the tool qualitative — it still catches the failure that matters most, which is a market-making backtest that assumes cancels are free.

**Decision:** none yet. The table ships with the mechanism (versioned facts, a 90-day staleness bound that reports itself in the summary) and every numeric field flagged `verified: false`, asserted by `no_numeric_limit_claims_to_be_verified` so a later edit cannot quietly promote a guess. A is the right answer and it needs someone to read three sets of exchange docs, which is not a thing to infer.

**Status:** answered 2026-09-21 (decided under Mason's delegation of 2026-09-21 — ADR-D-04). **B where the venue exposes
it, A everywhere else, and §5.6's acceptance is gated rather than dropped.**

Kraken returns rate-limit headers, so that venue's counter maximum and decay are read
from the venue and re-verified on every call that sees a header — self-updating, with
`verified_at` set by the observation rather than by a human. The rest are filled from
the venues' current documentation with `source_url` and `verified_at` per fact.

**§5.6's numeric criterion is kept and made conditional**, which is the part worth
deciding: *"a requote rate exceeding Kraken's limit is refused before paper testing"*
becomes "refused when the limit is verified and fresh; **reported as uncheckable when it
is not**". A refusal that cannot be computed must not be reported as a pass — that is
the same rule as everywhere else, and it means C's "drop the criterion" is wrong:
dropping it would silently convert an uncheckable refusal into an approval.
`no_numeric_limit_claims_to_be_verified` keeps holding the door.

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

**Status:** answered 2026-09-21 (decided under Mason's delegation of 2026-09-21 — ADR-D-05). **A, with C built beside
it. There is no `preregistration_required` boolean, and there will not be one.**

The requirement is the **task demand**, which is what `sealed_registration` already
implements: a `SingleLookup` needs none, everything else needs one, at every tier.
§11's per-tier table then supplies the *authoring mode* —
`preregistration: free | skeleton_grammar | skeleton_confirm` — which is the field
§11 actually implies and which S-10.14 is building anyway.

**B is refused on the asymmetry.** A boolean defaulting to `true` that nobody ever sets
is a field that does nothing; the first configuration that sets it `false` gets a tier
spending trials against a hypothesis that can only be confirmed — the exact failure
§7.2 exists to close — and INV-16 would not catch it, because a per-Experiment
`prereg_hash` satisfies the constraint while carrying no falsifier. A field whose only
purpose is to be set to the unsafe value should not exist.

S-10.3's sentence is a description of the demand check and should be reworded to say so.
`max_prereg_amendments` is unaffected and stays.

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

**Status:** answered 2026-09-21 (decided under Mason's delegation of 2026-09-21 — ADR-D-06). **§3 is amended to state the
property; §10's mechanism is the one that exists.** A conversation turn is not a job:
it is started by `AgentManager::send_message`, its identity is a row in `agent_runs`,
and `submit_child_once` has nothing to hand back for one. The exactly-once requirement
is real and is what §3 was reaching for, so §3 should read *"each child is created
exactly once per `(session_id, agenda_item_id)`"* and stop naming the mechanism.
Rewriting the child path as a job would mean building a worker that does not exist in
order to satisfy a sentence, and would break the desk, the rail and the steer controls,
all of which read the conversation path today.

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

**Status:** answered 2026-09-21 (decided under Mason's delegation of 2026-09-21 — ADR-D-07). **The baseline is the history
*before* the window, which is what was built.** The literal reading is not ambiguous,
it is **provably inert** — `d > 3·(c+d)/2` has no solution for positive costs, at any
ratio, which the kept test demonstrates over three orders of magnitude. A rule that
cannot fire at the moment its own gate opens is not a stricter reading of the spec; it
is a sentence with an arithmetic error in it. §7.1 should read *"exceeds 3× the median
of the runs preceding the window"*.

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

**Status:** answered 2026-09-21 (decided under Mason's delegation of 2026-09-21 — ADR-D-08). **The harness sends the caps
it has; §6.2 keeps four bars and stops promising four *capped* ones.**

Three of the four caps exist in the profile's `Budgets` (`steps`, tokens, trials) and
are simply not on the wire — that is a missing field, not a spec problem, and it should
be added. The fourth is dollars, which **cannot** be capped until OQ-16's price table
exists and has a fresh row for the model in question, so the dollar bar renders
`no cap recorded` until then and says why.

The projection is refused outright, in both readings: there is no burn series, and a
projected end drawn from a single remainder is a forecast with one data point. §6.2's
"projected end at current burn" needs a burn *series* on the wire before it means
anything, and inventing a cap from a remainder stays the one answer that is not
available.

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

**Status:** answered 2026-09-21 (decided under Mason's delegation of 2026-09-21 — ADR-D-09). **`ExitRule` gains a unit, at
a new schema version.**

ADR-0010 protects the canonical *storage form* from being rewritten; it does not forbid
the schema from versioning forward, which is exactly what 1.0 → 1.2 → 1.3 already does.
So `ExitRule` gains `unit: fraction_of_entry | atr | sigma`, defaulting to
`fraction_of_entry` so every stored document keeps its current meaning byte for byte,
and the ATR/sigma variants carry the lookback as a typed parameter rather than a string.

The alternative — dropping two of three units from the language — was rejected on what
it costs: an ATR-scaled stop is the most common risk rule in the domain, and a language
that cannot express it will be worked around with a hand-computed fraction that is
**wrong on the next bar** and looks identical in storage. `unit_not_representable` stays
until the field lands, because refusing is still better than converting.

This decision is coupled to ADR-D-19: the validator must accept the new version, and it
currently refuses even 1.3.

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

**Status:** answered 2026-09-21 (decided under Mason's delegation of 2026-09-21 — ADR-D-10). **The runtime gains a
previous-value register and a `CrossedAbove` op.** FEAT-004 §3.3 already lists it as the
intended extension, so this is scheduled work rather than a new want.

The alternative — strike cross detection from §3 — was rejected because a crossing is
not an exotic construct: it is in AGENT-011's own headline example, it is how the
majority of published technical rules are stated, and a language that cannot express it
pushes authors toward the level test that *looks* equivalent and is not. The refusal
(`needs_history`, pointing at the level comparison that does compile) stays correct in
the meantime and must **not** be relaxed into an approximation.

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

**Status:** answered 2026-09-21 (decided under Mason's delegation of 2026-09-21 — ADR-D-11). **§3 wins: a hard-stale feed
halts. The code and the test name both change to say so.**

Two reasons, and the second is the one that decides it. First, §3 is *specific* — it
names the family, the multiple and the response — while §4's table is a generic row
covering three families at once; a specific rule beats a general one where they meet.

Second, and structurally: **scaling to 25% still trades on a stale price.** Every other
response in the table reduces exposure to a risk that is being *measured*. A feed more
than ten times its timeframe stale is not a measured risk — it is an absent measurement,
and the platform's own rule is that an absent measurement is never a small one. A
quarter-size position priced off a dead feed is the same error at 25% of the size.

So `on_sustained` halts on a hard-stale `Family::Data` fact, and
`a_hard_stale_feed_halts_without_waiting_three_cadences` is made to assert what its name
has claimed all along. The soft band (> 3×) keeps warning. — the spec and the plan disagree about whether `regime_failure_search` spends a trial

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

**Status:** answered 2026-09-21 (decided under Mason's delegation of 2026-09-21 — ADR-D-12). **The disagreement dissolves
by construction, and the declaration follows the construction.**

A `regime_failure_search` that **re-reads a completed run's** per-period returns and
partitions them spends nothing: it dispatches no compute and looks at no new data, so
§5.5's `none` is correct for it. A variant that **dispatches per-regime re-runs** is a
different tool with a different signature, and it declares `Falsification`, which is
what the phase plan's sentence describes.

**The re-read form is the one to build**, because it is the one whose inputs can exist
(the returns are recorded) and because a searching-by-re-running tool multiplies trials
against a hypothesis chosen after seeing the data — which is the thing Gate 9 exists to
punish. If the dispatching variant is ever built it gets its own name and its own
`spends_trial: Falsification` **at the declaration**, never a runtime branch: a tool
whose trial cost depends on its arguments cannot be audited from the catalogue. — §5.5 assigns a tier that cannot serve the tools it assigns it to

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

**Status:** answered 2026-09-21 (decided under Mason's delegation of 2026-09-21 — ADR-D-13). **§5.5 and §9's build-order
table are amended to tier `T` for `trade_reshuffle_bootstrap` and `cost_shock_curve`.**
A templated tool receives rows only as `SeriesKind::Ohlcv` or `SeriesKind::TrialReturns`,
and neither carries per-trade `pnl` or `costs_paid`, which both tools require and which
§5.5's own signatures imply. The tier as written is unimplementable, not merely
inconvenient. P6 keeps the tier invisible to the model, so nothing about the contract
changes — this is a correction to stop the next reader losing a day to it.

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

**Status:** answered 2026-09-21 (decided under Mason's delegation of 2026-09-21 — ADR-D-14). **Both, in that order: §4's
example changes now, and nested features become a scheduled language item.**

The example must change immediately, because a spec illustrating its flagship feature
with a document the compiler refuses teaches the wrong thing to every reader and to
every model given the spec as context. It should use a predicate that compiles today.

But `rv(20) pct_rank < 0.8` is not a badly chosen example — **a percentile rank of a
derived feature is the natural way to state a regime gate**, and `skeleton.rs`'s
`VOL_TARGET_NOTES` independently recorded the same limit for `pct_rank(rv(close,20),200)`.
Two places wanting the same missing construct is a language gap, so the grammar gains
nested feature expressions as its own task, and `suggest_moves` keeps taking a
catalogue-checked `Predicate` from its caller until then — never inventing one.

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

**Status:** answered 2026-09-21 (decided under Mason's delegation of 2026-09-21 — ADR-D-15). **`bars_sole_blocker` is the
definition; the "last clause to fail" wording is struck from §4 and from S-5.15's
body.** It is not a second candidate reading — it has **no referent on this platform**:
the compiled condition is `(A * B)`, both operands evaluate, and there is no failure
order for "last" to index. Keeping the phrase would leave a reader looking for an
evaluation order that the compiler is specifically designed not to have.

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

**Status:** answered 2026-09-21 (decided under Mason's delegation of 2026-09-21 — ADR-D-16). **§3's table is amended: a
suite may declare a *generated* corpus, and `judgement` does.** The exemption reading is
the wrong shape — this is not a special case, it is a second legitimate corpus kind, and
any future suite whose cases carry planted ground truth will need exactly the same
thing.

The reasoning is not stylistic. `expected.json` **is** the answer key, on a filesystem
the agent reaches with `code_execution` and a workspace; a file saying "this one is a
null" is one `cat` away from turning every number on the scorecard into a number about
file reading. A corpus generated from a seed keeps the key in the memory of the process
that generated it, which is the only place it cannot be read from. §3's fixture format
stays correct for suites whose cases are inputs rather than answers.

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

**Status:** answered 2026-09-21 (decided under Mason's delegation of 2026-09-21 — ADR-D-17). **Strike `replay` from the
`judgement` row; it runs live-only.** A replayed tape returns the same reply every time,
so a miss rate measured over it has no sampling error and measures the tape rather than
the tier. It would be a number that looks like a measurement and is not one — which is
the exact failure this suite exists to detect one level down, reproduced inside the
detector. `SuiteKind::Replay` stays right for suites that check *determinism*; it is
wrong for one that estimates a rate.

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

**Status:** answered 2026-09-21 (decided under Mason's delegation of 2026-09-21 — ADR-D-18). **For a generated corpus the
holdout is a *distinct seed*, not a hash split of one corpus. §4.2 gains that sentence;
the hash split stays correct for fixture corpora.**

A hash split over a seeded corpus hides *which* cases are sealed and hides **nothing**
about their answers — anyone holding the seed holds the key to all of them, sealed and
unsealed alike. The defence §4.2 is reaching for is against tuning to the holdout, and
against a regenerable corpus the only thing that actually defends it is material the
tuner has never seen. `PlantedCorpus::holdout_input()` is already wired for either, so
this is a policy choice rather than a rebuild.

---

## Q-S3-03 — AGENT-006 §5.4 names two different tests as one tool

**Raised 2026-09-21 auditing the remaining catalogue tools. Neither implemented.**

§5.4 gives `future_perturbation_test` the signature
`(strategy_id, timeframe, shift_bars=1)` and describes it as *"rerun-and-diff,
Freqtrade-style"*, needing *"a shift/mask column X mode in the strategy
runtime"*.

`shift_bars` is **column perturbation** — move one feature column forward and
see whether the edge survives. Freqtrade's shipped precedent, which S-3.10's own
Traces line cites, is `lookahead-analysis`: **data truncation** — run the same
strategy over a shorter window and check that the shorter run's signals are a
prefix of the longer run's.

They detect overlapping but distinct defects, and only the truncation reading is
buildable on this platform today (`preview_signals` already gives the prefix
invariant; nothing anywhere in the workspace mentions `shift_bars`,
`shift_column`, `mask_column`, `jitter_bars`, `fill_delay_ms` or
`perturbation_pct`).

Shipping the truncation test under the name `shift_bars` would claim a
computation it did not perform, so neither was built.

**Status:** answered 2026-09-21 (decided under Mason's delegation of 2026-09-21 — ADR-D-19). **Two tools.** They detect
different defects and only one of them is buildable today, so collapsing them loses the
one we can have.

`future_perturbation_test(strategy_id, timeframe, shift_bars)` stays as specified and
stays blocked on the runtime's missing perturbation mode. **`lookahead_truncation_test`**
is new and buildable now: run the strategy over a shorter window and assert the shorter
run's signals are a **prefix** of the longer run's — the invariant `preview_signals`
already supports, and Freqtrade's shipped `lookahead-analysis` in substance. §5.4 names
both and stops citing the truncation precedent under the perturbation signature.

---

## Q-S3-04 — a route's SQL is not checked against the schema anywhere

**Raised 2026-09-21. Not a spec contradiction — a missing enforcement, recorded
here because it has already produced two dead routes and will produce more.**

`/api/evidence/trades` has never returned a trade. It runs
`SELECT result FROM backtest_runs WHERE id = $1 AND user_id = $2`; migration
`0026` renamed that table to `backtest_jobs` and built a **different**
`backtest_runs` with none of those three columns. The handler's
`let Ok(Some(row)) = row else { … }` turns the resulting database error into
`404 no_such_run`, so the failure reads to every caller as *"you asked for a run
that isn't there"*.

`crates/api/tests/the_desk_calls_routes_that_exist.rs` checks that a **route**
exists. Nothing checks that a route's **query** matches the deployed schema, and
a renamed table is invisible to the type system because these are runtime
strings.

**Status:** being closed — a live-Postgres invariant that `PREPARE`s every
evidence query is in flight. Recorded so the mechanism is not lost if it is not.

---

## Q-S5-05 — the validator refuses every document the language can compile

**Raised and answered 2026-09-21.**

`strategy-validator/src/schema.rs` accepts only `"1.0" | "1.2"`. The fix text attached
to that same error code reads *"1.3 adds the exits block"*. The compiler targets **1.3**,
because `exits` is the only schema that can hold `exit stop 2 % from entry` and `domain`
documents it REQUIRED at 1.3. `routes/strategies.rs` validates before it persists. **So
the SLv2 authoring path is dead at its last step.**

**Status:** answered (decided under Mason's delegation of 2026-09-21 — ADR-D-20). **The version check becomes a bounded
range, not a whitelist: `1.0 <= v <= MAX_SUPPORTED`, with `MAX_SUPPORTED = 1.3` today.**

A whitelist is why this happened: 1.1 and 1.3 were never added because nobody was
looking at that `matches!` when they shipped a version. A range strands nothing additive
and still refuses a version from the future, which is the property the check is for.
**There is a live `1.1` row in `strategy_definitions` that the current whitelist
refuses** — found by querying the registry, not by any test — and a range admits it.

The test pinning this (`the_only_thing_the_validator_holds_against_a_compiled_strategy_is_its_version`)
is to be **deleted** when the range lands, not widened: its whole purpose was to fail on
the day the gate opened.

---

## Q-S5-06 — `create_strategy`'s own advice erases the lineage the same task asks it to record

**Raised and answered 2026-09-21.**

`parent_version` exists nowhere in the repository. Lineage is
`strategy_revision.parent_hash`, derived server-side from the `head_revision` **of the
same `strategy_id`**. `create_strategy`'s description advises *"mint versioned slugs
(`my_strat_v2`) when iterating"* — and a new slug has no head, so `parent_hash` is NULL.
**Measured on the live registry: 40 agent-authored revisions across 34 slugs, 6 with a
parent. 85% of the agent's authoring history has no lineage at all.**

**Status:** answered (decided under Mason's delegation of 2026-09-21 — ADR-D-21). **Strike the slug advice. Reusing the slug
*is* the lineage mechanism, and nothing is lost by it.**

The advice was written as if overwriting `head_revision` discarded the previous
definition. It does not: `strategy_revision` is append-only and the chain holds every
version, which is precisely why `parent_hash` is derivable. So the guidance inverts the
design — it tells the author to do the one thing that breaks the record, to protect
against a loss that cannot happen.

`create_strategy` additionally gains an **optional explicit `parent_strategy_id`**, for
the real case the slug rule cannot express: a strategy derived from a *different*
lineage. Optional rather than required, because a first version genuinely has no parent
and a required field would be filled with something.

---

## Q-S5-07 — the route records agent-authored revisions as `'human'`

**Raised and answered 2026-09-21.**

`routes/strategies.rs` binds `actor_kind = "human"` unconditionally, while migration
`0058` says that column *"distinguishes a human author from the research agent, which is
the difference between a hypothesis and a sample"*. The live table holds 40 `'agent'`
rows, so a second writer exists and the two paths disagree about who wrote a revision.

**Status:** answered (decided under Mason's delegation of 2026-09-21 — ADR-D-22). **This is a bug, not a question. Bind the
actual actor from the authenticated principal**, and make the column's meaning
enforceable rather than conventional: a service-token principal is `'agent'`, a user
session is `'human'`, and there is no default — a caller the platform cannot classify is
refused rather than recorded as either. A misattributed author turns a sample into a
hypothesis in the multiple-testing accounting, which is the one direction that flatters
results.

---

## Q-S6-01 — §3.5's "pin a tool for this run" names a mechanism that does not exist

**Raised and answered 2026-09-21.**

`search_tools` is a tool the *model* calls and the harness answers. There is no
user-initiated equivalent, no route and no column recording a per-run pin, and
`Driver::route` builds `pinned` from exactly three sources: `discovered_tools`, the
active plan step's named tool, and a three-hit search over that step's goal text. A
human message is none of them.

**Status:** answered (decided under Mason's delegation of 2026-09-21 — ADR-D-23). **Strike the clause from §3.5.** A pin would
be a fourth source in the router serving a need the three already meet: steering by
message reaches `discovered_tools` and the plan step, which is the documented way to
change what the agent can reach. Building a parallel mechanism for it would add a
second way to influence tool availability — and two paths into the same decision is how
`reorder` came to switch the scorer off earlier today.

The panel already names the three real levers, which is the honest replacement for a
button that did nothing.

---

## Q-S6-02 — §19.6's "within one event" has no channel

**Raised and answered 2026-09-21.**

Nothing on this platform pushes kill-switch state to a browser. `useDeskStream` is
per-run; there is no global channel. A trip from the risk engine or another operator
reaches the banner by three-second poll and by nothing else. The criterion is a
phase-7 exit criterion and was being held up by a comment describing a mechanism that
did not exist.

**Status:** answered (decided under Mason's delegation of 2026-09-21 — ADR-D-24). **Restate the criterion as a bounded
interval: the banner reflects a trip within 3 seconds, and immediately for a trip this
client caused.** No push channel is built for it.

The reasoning is about what the banner *is*. **It is not the control that stops
trading** — the order gate is, server-side, and it is already fail-closed: with no
monitor lease every order is refused, today, with no browser involved. The banner is an
operator *notification*, and a three-second bound on a notification is a real guarantee,
while "within one event" over a channel that does not exist is not one. Building a
global push channel to improve 3s to ~0s on a surface that authorises nothing is effort
spent where the failure is already contained.

If a push channel is built later for other reasons, this criterion tightens for free.

---

## Q-S6-03 — §3.5's "acceptance criteria with ticks" has nothing to tick

**Raised and answered 2026-09-21.**

`WorkingState.acceptance` is `string[]` on both sides of the wire. No per-criterion
met/unmet flag is emitted by the harness, so a tick would be an invented measurement and
the panel renders a plain list.

**Status:** answered (decided under Mason's delegation of 2026-09-21 — ADR-D-25). **The harness emits the status; the field
becomes `[{text, met: bool | null}]`.** §3.5 is right to ask — an acceptance list with no
status is a list of things nobody is tracking, which is the failure mode the working
state exists to prevent.

`null` is a required third state and not a convenience: *"not yet evaluated"* is the
honest answer for most criteria for most of a run, and collapsing it into `false` would
report every unstarted criterion as failing. The panel renders three glyphs, never two.

---

## Q-S6-04 — S-6.22's acceptance names a file the aliases were never in

**Raised and answered 2026-09-21.**

The criterion says the grep must return nothing *"outside `compat.css`"*. The same task
body says the aliases live in *"`tailwind.theme.css` lines 85-94"*, which is where they
were. `compat.css` contains no alias and never did.

**Status:** answered (decided under Mason's delegation of 2026-09-21 — ADR-D-26). **Amend the criterion to name
`tailwind.theme.css`, and keep the stronger form that was actually implemented:** the
grep must return nothing **anywhere**, because the declarations themselves are now
deleted. Allow-listing `compat.css` would have been the silent fix and would have left
the criterion pointing at the wrong file for the next person.

Worth recording beside it, because it is the more general lesson: the criterion was
**narrower than its own goal**. Five class spellings were clean while 36 usages reached
the same deprecated colours through other utility prefixes, so the grep would have
passed with the aliases still in service. A guard narrower than its claim is catchable
by discipline; a criterion narrower than its goal is not, and only re-reading what the
rule was *for* finds it.

---

## Q-S6-05 — AGENT-012 requires a pause in two places and marks it unbuilt in a third

**Raised and answered 2026-09-21.**

§3.4: *"`Pause` · `Resume` · `Stop` · `Steer…` … Keyboard: `space` pause/resume"*.
§19.3 makes it an acceptance criterion: *"Approve, deny, pause, stop and steer work from
the keyboard while the agent is mid-call"*. §17 of the same document lists
*"pause/resume, fork, escalate"* as **✗ unbuilt**. The platform agrees with §17: all three
routes answer 404 while `…/cancel` answers 401.

**And the UI did not merely have a dead button.** `onPause` set a local `paused` flag
**before** issuing the request that 404s, so the conversation header read **paused** over
a run that was still spending trials. `space` was bound to it. And the Stop dialog
offered *"Pause instead"* as the gentler option — **actively talking the operator out of
the control that works and into the one that does not.**

**Status:** answered (decided under Mason's delegation of 2026-09-21 — ADR-D-27). **Build pause and resume, bounded. Strike
`escalate` from §3.4.**

Deleting the button was right and does not close the question, because **the need that
produced the lie is real**: `Stop` is destructive, an operator watching a run go wrong
wants to interrupt it without discarding its context, and when the only offered control
is destruction somebody will build a gentler-looking one that does nothing. That is
exactly what happened here, twice — once in the button and once in the dialog copy that
recommended it.

**Bounded is the load-bearing word.** A paused run holds its budget, its lease and its
place in the queue, so an unbounded pause is a resource leak that looks like an operator
being careful. `paused` carries a deadline; **on expiry the run stops — it does not
resume.** Fail-closed: a run that nobody came back to must not quietly start spending
again, and an operator who has forgotten a paused run is better served by finding it
stopped than by finding it finished.

**`escalate` is struck** from §3.4's control row and stays in §17 as unbuilt. It is a
tier change mid-run, which is a different product decision with its own budget
consequences, and it has no caller and no demand. A control row is not a place to park
an intention.

---

## Q-S6-06 — §3.3's `attach` names four things and no route of any kind

**Raised and answered 2026-09-21.**

§3.3 line 153: attach *"a strategy version, a dossier, a finding, a file from the
workspace"*. `/api/attachments`, `/uploads`, `/files` and a bare `/api/memory` are all
404, and no message or session POST carries an attachment field.

**Status:** answered (decided under Mason's delegation of 2026-09-21 — ADR-D-28). **Split it. The three platform objects are
buildable and worth building; the workspace file upload is struck.**

A strategy version, a dossier and a finding are **objects this platform already has ids
for**. "Attaching" one is inserting a typed reference the agent can resolve with the
tools it already has — no storage, no upload, no new content to scan or serve, and it
composes with the provenance fence because a reference to a platform object carries the
object's own provenance rather than acquiring a new one.

**A file from the workspace is a different thing wearing the same word.** It is a general
upload endpoint: new storage, new lifecycle, new size and content limits, and a new
untrusted-content surface feeding the exact channel the provenance fence exists to
guard. That is a large security surface for a convenience, and the workspace is already
reachable by the agent's own tools, which is the path that keeps the fence intact.

§3.3's sentence should name the three and drop the fourth.

---

## Q-P-01 — two bar tables, different contents *and* different durability, and no plan file mentions either

**Raised and answered 2026-09-21. Verified directly against ClickHouse, not reported.**

`trading.market_bar` and `trading.market_bars_v2` both hold OHLCV bars and **disagree by
12,307 rows** for one series (BTC-USD / coinbase / 1m: 105,760 vs 93,453). Two endpoints
serve the two numbers: `/api/backtests/coverage` reads `market_bar`, and P-2.6's
published QC census read `market_bars_v2`. Neither says which is authoritative.

**The durability difference is the serious half**, and it is structural rather than
configurable:

```
market_bar      ReplacingMergeTree(ingest_time)
                ORDER BY (instrument_id, venue_id, period_secs, event_time,
                          knowledge_time, revision_seq)

market_bars_v2  ReplacingMergeTree(ingested_time)
                ORDER BY (instrument_id, timeframe, venue_id, source,
                          event_time, revision)
```

`knowledge_time` is **in `market_bar`'s sort tuple and absent from `market_bars_v2`'s**.
So on v2, two rows for the same bar that share a `revision` value and differ in knowledge
time **collapse on merge**, keeping whichever arrived later by `ingested_time`. A
restatement is silently destroyed by a background merge — no error, no row, nothing to
read afterwards. On `market_bar` the same pair cannot collapse, because both discriminators
are in the key.

**Status:** answered (decided under Mason's delegation of 2026-09-21 — ADR-D-29).
**`market_bar` is authoritative. `market_bars_v2` is frozen.**

The live system settles which: **`market_bar`'s newest row was ingested minutes ago
(`23:02` today); `market_bars_v2`'s newest is `2026-09-14`** — a week stale. The canonical
table is the one receiving live bars, and it is also the only one that can hold the
revision guarantee DA-16 asks for.

Three consequences, and the second is the one that costs work:

1. **Nothing may read `market_bars_v2` for a point-in-time answer.** Its merge semantics
   are a non-PIT read path that nobody added deliberately — it was inherited from a sort
   key — and INVARIANTS forbids one however it arrived.
2. **The writers that still target v2 must be redirected or retired** —
   `dataplane/src/bar.rs`, `dataplane/src/source.rs`, `api/src/synthetic_worker.rs` and
   the census example. A table that is frozen in the plan and written in the code is
   worse than either.
3. **P-2.6's published grades were measured on the stale table** and must be re-measured
   before they are quoted again. The precedent for enforcement already exists: `xtask`
   ships `check-bars-v1-frozen`, so freezing v2 is the same mechanism a second time.

Recorded as a question rather than a bug because the choice of authority is a decision;
the merge behaviour is a fact.

---

## Q-G4-01 — two specs give two incompatible lineage designs for the same thing, and no file said which is built

**Raised 2026-09-22.**

**FEAT-004 §4** specifies `strategy_versions(… parent_version TEXT REFERENCES
strategy_versions …)` plus a `strategy_slugs` table, with *"Versions are immutable. A
slug moves `current_version`"*. **AGENT-011 §4** specifies `strategy_revision`,
append-only, with a server-derived `parent_hash` taken from `head_revision`. Different
tables, different lineage mechanism, different immutability story.

Not academic. `create_strategy`'s advice to *"mint versioned slugs (`my_strat_v2`) when
iterating"* was written against the slug-moves-a-pointer model, and under the built
append-only model it **erases the lineage the same task asks it to record** — a new slug
has no head, so `parent_hash` is NULL. ADR-D-21 decided about the *advice*. It did not
decide which table is the design, and until now nothing did.

**Established from the code and the live database, not from either spec — this half is a
discovery and is not open:**

- `strategy_versions`, `strategy_slugs` and `strategy_reconciliations` **do not exist**:
  not in `migrations/`, not in any crate, not in `frontend/`, not in the live database.
  `docs/specs/FEAT-004-strategy-representation-v2.md` is their only occurrence in the
  repository. The migration FEAT-004 §4 names, `migrations/0041_strategy_v2.sql`, was
  never written and that number belongs to `0041_agent_conversations.sql`.
- `strategy_revision` **is built**: `migrations/0058_strategy_revision.sql` creates it
  append-only (chain trigger plus `mlops.refuse_mutation()` on UPDATE and DELETE), adds
  `strategy_definitions.head_revision` as the pointer at it and
  `strategy_instances.revision_hash` as an instance's binding, and enforces both with
  triggers. `crates/api/src/routes/strategies.rs`'s `create_strategy` is the writer and
  derives `parent_hash` from the previous `head_revision` of the same `strategy_id`;
  `crates/api/src/routes/lifecycle.rs` reads definitions back out of it;
  `crates/invariants/tests/external_review_invariants.rs` asserts the route still does
  both. The live database holds the table with the triggers in place.

**What is genuinely open — the decision, with both sides costed:**

FEAT-004 §4 carries one capability the built design has no equivalent for: a per-slug
**`deployed_version`, distinct from the current pointer**, which is what makes §4's next
rule — *"Changing a deployed strategy restarts the funnel for the new version"* —
expressible at all. `deployed_version` appears nowhere in the repository.
`strategy_instances.revision_hash` answers *"what is this instance running"* per
instance, not *"what is approved for deployment"* per slug, and nothing enforces the
funnel-restart rule today. SLv2 also needs somewhere to put `layer`, `ast`,
`source_text`, `code_handle` and `params_schema`, none of which `strategy_revision` has.

  A. **Extend `strategy_revision`** with the SLv2 columns and add a second pointer
     (`strategy_definitions.deployed_revision`) beside `head_revision` — cost: one
     append-only table keeps carrying two formats (v1 JSON and v2 AST) and its
     content-address is already computed two different ways (0058's backfill hashes
     `definition_json::text`, the route hashes the application's canonical form, and
     `actor_kind = 'import'` is the only thing distinguishing them); a `layer` column
     that is NULL for everything written before SLv2; but no second lineage record, no
     second immutability story, and every existing reader, trigger, test and
     `RunConfig.strategy_version` binding keeps working untouched.
  B. **Build `strategy_versions`/`strategy_slugs` as FEAT-004 §4 specifies** — cost: two
     lineage records for one concept, which is precisely the condition that produced
     this question; a migration to keep them consistent, or a second write path that
     does not; and 0058's deliberate refusal of the self-referential parent FK
     (*"a revision imported from another tenant's export names a parent we may not
     hold, and refusing the import loses more than the dangling pointer costs"*) has to
     be re-argued, because §4's `parent_version TEXT REFERENCES strategy_versions` is
     exactly that key. In exchange: a clean per-layer schema and the deployed/current
     split without retrofitting.
  C. **Neither yet** — strike §4's storage block and re-specify it when
     `crates/strategy-lang` exists. Cost: the funnel-restart rule loses its only written
     home, and FEAT-004 §4's `deployed_version` is the only place on the platform where
     that rule is stated at all.

**Spec says:** both, incompatibly. FEAT-004 §4: *"Versions are immutable. A slug moves
`current_version`."* AGENT-011 §4: *"reusing the slug is the lineage mechanism —
`strategy_revision` is append-only and `parent_hash` is derived server-side from the
`head_revision` of the same `strategy_id`."*

**Reference says:** `migrations/0058_strategy_revision.sql` — *"`strategy_id` becomes a
**pointer**, and everything that constitutes evidence binds to a content-addressed
revision that cannot be edited or deleted."*

**Recommendation:** none on the design choice — it is a decision, not a discovery, and
guessing it is how the slug advice came to be written. High confidence on the discovery
half: `strategy_revision` is what is built, and anything written against FEAT-004 §4's
tables today is written against nothing.

**What was done 2026-09-22 (this track, docs only — no code touched):** FEAT-004 §4
gains a lead-in warning above its SQL block and a closing amendment recording all of the
above; **nothing in §4 is struck**, because the unbuilt design is better on the
deployed/current split and a contradiction named is worth more than a design rewritten
on inference. AGENT-011 §4 gains the matching note. Both point here.

**Status:** open (the design choice). The discovery — which design is built — is
settled and recorded in both specs.

---

## Q-G4-02 — the 85% lineage figure was measured from test residue, and there is no agent authoring history at all

**Raised and resolved 2026-09-22.**

Q-S5-06 and ADR-D-21 both carry: *"Measured on the live registry: 40 agent-authored
revisions across 34 slugs, 6 with a parent. 85% of the agent's authoring history has no
lineage at all."* AGENT-011 §4 carried it in bold.

**Every row that measurement counted is test residue.**
`crates/ledger/tests/pg_strategy_revision.rs` used to write through the pool against
`DATABASE_URL` — the platform's own development database — with no teardown. Its own
header now says so: *"40 of the 54 rows in `strategy_revision` are its fixtures, and two
separate measurements of 'how much of the agent's authoring history has lineage' were
mostly counting them before anyone noticed."* The fixtures are written as
`actor_kind = 'agent'` because the column's CHECK admits only
`human|agent|scheduler|import` and there is no `'test'` value to use; they are
identifiable by `actor_id = 'a'`. A DELETE teardown could not have fixed it —
`trg_strategy_revision_immutable` refuses DELETE, which is the property one of the tests
exists to assert — so the file now runs every case inside a transaction it never commits.

Re-measured the same day with the residue excluded, the result is **worse than the
retracted figure and points somewhere else**: every surviving revision was written by
**0058's own backfill** — `actor_kind = 'import'`, `actor_id = 'pre-0058'`, and each
hash reproduces the backfill's `sha256(definition_json::text)` rather than the
application's canonical hasher. Not one revision has been committed by
`POST /api/strategies`. So the honest statement is not that the agent's lineage is
mostly missing; it is that **there is no agent authoring history on this database to
have lineage**, and anyone planning to measure ADR-D-21's effect by watching this table
will be watching an empty one.

This also retires the evidence sentence in **Q-S5-07**: *"The live table holds 40
`'agent'` rows, so a second writer exists and the two paths disagree about who wrote a
revision."* There is no second writer — the only writers are 0058's backfill, the route,
and the fixture. ADR-D-22's decision is unaffected: it rests on `actor_kind`'s meaning
being enforceable rather than conventional, and a hard-coded `'human'` is wrong whether
or not a second writer exists.

**ADR-D-21 is unaffected and is not reopened.** It was argued from the mechanism — a new
slug has no head, so `parent_hash` is NULL — which is in the migration and the route,
not in a row count. A number was offered as corroboration and was not one.

**Still carrying the struck figure, outside this track's write territory:**
`ADR-INDEX.md`'s ADR-D-21 row (*"85% of the agent's authoring history has no lineage
because of it"*) and Q-S5-06 above, which is append-only here. Whoever holds those files
should strike the clause, not the decision.

**The general rule this is the second instance of** (memory: *measured from residue*):
before believing any count from `strategy_revision`, exclude `actor_id = 'a'` and
`actor_id LIKE 'test-fixture:%'`. `pg_strategy_revision.rs`'s closing comment specifies
the migration that would make the marker unforgeable — widen the `actor_kind` CHECK to
admit `'test'` and add `CHECK ((actor_kind = 'test') = (actor_id LIKE 'test-fixture:%'))`
`NOT VALID`, with no backfill, because the table refuses UPDATE as well as DELETE.

**Status:** resolved as a retraction. No decision needed; the correction is factual.
AGENT-011 §4's copy is struck in place with the account beside it.

---

## Q-G4-03 — AGENT-012 §3.3 still offers `/pin`, the mechanism ADR-D-23 struck from §3.5

**Raised 2026-09-22.**

ADR-D-23 struck §3.5's Tools-tab control: *"**pin a tool** for this run (sends a
`search_tools`-equivalent)"*, because `Driver::route` builds `pinned` from exactly three
sources — `discovered_tools`, the active plan step's named tool, and a three-hit search
over that step's goal text — and a human message is none of them.

**§3.3's composer still lists it as a slash command:** *"Slash commands: `/steer`,
`/pause`, `/pin`, `/why`, `/model`, `/budget 5$`, `/tier local`."* The spec already
flags this itself, in §3.5's own amendment: *"**Not reached by ADR-D-23:** §3.3's
composer still offers a `/pin` slash command. It appears to name this same mechanism,
but the decision does not mention it, so it is left standing and recorded as an open
question rather than struck on inference."* This is that record; no Q- entry existed.

  A. **`/pin` is the same mechanism and is struck** — cost: none in code (there is no
     route, no column and no fourth router source to remove), and it closes the one
     remaining place a reader can find the struck control. Risk: if `/pin` was meant as
     something else — pinning a *card* or a *message* in the conversation, which §14
     uses the word for (*"the approval/question card is pinned above the composer"*) —
     striking it deletes an unrelated affordance nobody wrote down.
  B. **`/pin` is a different thing and stays, renamed** — cost: someone must say what it
     pins, and until they do the composer advertises a command with no referent, which
     is principle 10's failure mode (*"a control that renders and does nothing is worse
     than an absent one"*).

**Recommendation:** A, at low-to-moderate confidence — the two entries sit in adjacent
subsections of one spec and no text anywhere describes a second pin. Not applied:
AGENT-012 is outside this track's write territory, and ADR-D-23 does not mention `/pin`,
so applying it would be deciding on inference exactly where the spec says not to.

**Status:** open.

---

## Q-G4-04 — AGENT-012 §3.3's `/tier local` is a mid-run tier change, which ADR-D-27 struck `Escalate to…` for being

**Raised 2026-09-22.**

ADR-D-27 struck §3.4's `Escalate to…` control: *"it is a tier change mid-run, which is a
separate product decision with its own budget consequences, and it has no caller and no
demand. A control row is not a place to park an intention."* §0.2's principle 10 was
amended to match.

**§3.3's composer offers `/tier local`** in the same slash-command list. §3.3 also says
*"Sending while a run is active is a **steer** by default"* — so `/tier local` typed
during a run is a mid-run tier change, in the down direction, by a different widget.
ADR-D-27 names a *button* in a *control row*; it does not mention the slash command, and
the down direction is not obviously the same product decision as the up one.

  A. **The two are one decision; `/tier local` is struck alongside `Escalate to…`** —
     cost: the composer loses a quick way to say "do this cheaply", which the model
     picker beside it already provides for a *new* run. Downgrading mid-run has the
     budget-consequence problem in mirror image (a run's evidence would then come from
     two tiers with no record of the switch), which is ADR-D-27's stated reason.
  B. **They are different; `/tier local` survives as a pre-run shortcut only** — i.e.
     it is the §3.3 model picker in text form, refused while a run is active with the
     reason named. Cost: a command whose availability depends on run state, which is a
     third thing to build and test; but it keeps a genuinely useful shortcut and matches
     what the composer's model picker already does.
  C. **`/tier local` survives unrestricted** — cost: it reintroduces exactly the control
     ADR-D-27 removed, through the widget that was not named in the decision, which is
     how a struck mechanism comes back.

**Spec says:** §3.3, verbatim — *"Slash commands: `/steer`, `/pause`, `/pin`, `/why`,
`/model`, `/budget 5$`, `/tier local`."*

**Recommendation:** B, low confidence, and explicitly not applied. The distinction
between "change the tier of the run I am about to start" and "change the tier of the run
that is spending right now" is the whole of ADR-D-27's reasoning, and which side
`/tier local` falls on depends on a product intent the spec does not state.

**Status:** open.

---

## Q-G4-05 — `escalate` appears in AGENT-012 §14 and §3.2 without saying which of the two escalations it means

**Raised 2026-09-22.**

ADR-D-27 drew a line the rest of the spec does not yet observe, and said so in its own
words: *"**This is not the same thing as `Outcome::Escalated`.** That is the harness
handing a run to a stronger tier when it needs one (AGENT-008 §4, AGENT-010 §6); it
exists, it works… What is struck is an **operator button that changes tier mid-run**."*

Two places still use the bare word:

- **§14 (states)** — *"**Degraded tier refusing**: the refusal card explains what the
  tier cannot do and **offers escalation**."*
- **§3.2 (card kinds table)** — the `Halt / degradation` row's actions column reads
  *"resume / retry / **escalate**"*.

§3.2's separate **`Escalation`** card row — *"from tier → to tier, the working state
carried, why"*, actions *"confirm/decline"* — is unambiguously `Outcome::Escalated`
surviving as specified, and *Needs you* (§2) is right to show it.

The ambiguity is worth an entry because the two readings differ in what gets built. If
the refusal card *offers* escalation, that is an operator-initiated mid-run tier change
with a budget consequence — the struck control, arriving through a state description. If
it *reports* an escalation the harness already decided and asks the operator to confirm
it, that is the `Escalation` card and needs no new control.

  A. **Both are `Outcome::Escalated`; reword to "confirm escalation"** — cost: one word
     in two places, no code. It makes the degraded-tier state consistent with the
     `Escalation` card it would render. Risk: a degraded tier that *refuses* may have no
     pending `Outcome::Escalated` to confirm, in which case "confirm" has nothing to act
     on and the state needs a different answer (retry, or stop and restart at a tier
     that can).
  B. **The degraded-tier refusal genuinely needs an operator-initiated escalation** — in
     which case ADR-D-27 has one exception and should say so, bounded to this state.
     Cost: reopens the control ADR-D-27 closed, and the budget question it closed it
     over; but a tier that cannot do the job and offers the operator nothing is the
     halt-with-no-recourse that principle 10 exists to prevent.

**Recommendation:** A, moderate confidence, contingent on the risk above being checked
against what `Outcome::Escalated` actually emits on a degraded-tier refusal — which is a
code question this entry did not settle. Not applied; AGENT-012 is outside this track's
write territory and ADR-D-27 does not reach §14 or §3.2.

**Status:** open.

---

## Q-G4-06 — AGENT-006 §6 says `leakage_audit` composes `future_perturbation_test`; the built battery runs a different test and says nothing about the substitution

**Raised 2026-09-22.**

**§6's workflow table** gives `leakage_audit`'s sequence as *"`probe_pit_provenance` →
`feature_leakage_scan` → `bar_boundary_check` → null-library label shuffle →
`future_perturbation_test` (bias → leaking) → `llm_contamination_check` if text features
exist"*.

**§5.4 says that step cannot run:** `future_perturbation_test` is *"**column
perturbation**: move one feature column forward and rerun-and-diff; needs a 'shift/mask
column X' mode in the strategy runtime, which does not exist, so this tool stays blocked
on it"*.

Read together, the battery cannot run as specified. **The part of that which is already
decided:** ADR-S8-01 (a battery with a step nothing performs is absent from the
catalogue, and every owed stage is declared where the battery is), ADR-S8-02
(`enforced == workflow.is_runnable()`, held by
`crates/invariants/tests/gates_demand_tickets.rs`) and ADR-S8-09 (a *conditional* owed
stage moves the refusal into `run` instead of hiding the battery). All three are built:
`crates/mcp-server/src/tools/workflows.rs` has `Stage::Owed`, `owed()`,
`owed_unconditionally()` and `is_runnable()`.

**What no ADR covers, and what this entry is for.** In
`crates/mcp-server/src/tools/workflows/batteries.rs`, `leakage_audit`'s two `Stage::Owed`
entries are the *static scan* and *contamination* steps, both `When::IfArgument`, both
carrying a comment explaining why they are deferred by design. The
`future_perturbation_test` step is not owed and not declared missing — **it has been
replaced** by a `neighborhood` study labelled `perturbation`, asking *"does the edge
survive perturbing the parameters it was found at?"*, with no comment recording the
substitution. Every other divergence in that file is annotated; this one is not.

Parameter-neighbourhood perturbation and forward-shifting a feature column are not the
same test and do not detect the same defect: the first is a fragility check, the second
is a lookahead check. Because both conditional gaps leave `owed_unconditionally()`
empty, `leakage_audit` is **in the catalogue and runnable**, and under ADR-S8-02 that
makes its gate-2 requirement enforceable — on a battery whose §6 sequence names a
lookahead check it does not perform. `leakage_audit`'s whole product is the word
`clean`, and ADR-S8-01's own argument applies: *"a `leakage_audit` that quietly ran three
of its six checks would emit `clean` in exactly the form the spec describes, and nothing
downstream could tell it apart from the real thing."*

  A. **The neighbourhood study is the intended replacement** — then §6's sequence is
     stale text and should name the step that runs, and `batteries.rs` should carry the
     one-paragraph justification its three siblings carry. Cost: an edit to two files,
     no code. But it must be argued, not assumed: it concedes that `leakage_audit`
     performs no lookahead check at all, while `lookahead_truncation_test` (ADR-D-19,
     `S`, *"buildable today"*) exists and is not in the sequence.
  B. **The column-shift test is still owed** — then it is a `Stage::Owed` that was
     dropped rather than declared, `leakage_audit` is unconditionally un-runnable under
     ADR-S8-01, and ADR-S8-02's invariant should be flipping its gate-2 requirement to
     `enforced: false`. Cost: gate 2 loses an enforced requirement it may currently be
     enforcing, which is a visible behaviour change and the more expensive option — and
     the more honest one if the check is genuinely absent.
  C. **Substitute `lookahead_truncation_test` for the blocked step** — it answers the
     lookahead question from the other end (a shorter run's signals must be a prefix of
     a longer one's), is rated `S` and buildable on `preview_signals`, and is already in
     the `audit` namespace per ADR-D-19. Cost: real implementation work; but it is the
     only option that leaves `clean` meaning what §6 says it means.

**Recommendation:** none between A, B and C — which check `leakage_audit` must perform
before it may say `clean` is a decision about what the verdict word promises, and
ADR-S8-01 is explicit that the verdict word is the whole product. High confidence on the
finding itself: §6's named step and the built step are different tests, and nothing in
the repository records the swap. Not applied; AGENT-006 and `crates/` are outside this
track's write territory.

**Also worth re-checking by whoever takes this:** AGENT-006 §9's AC-8 note says the
requirement table *"declare[s] all four and enforces one — `selection_bias_verdict` at
Gate 2"*. If `leakage_audit` is now runnable, ADR-S8-02's invariant should have made
that two, and the note is stale.

**Status:** open.
