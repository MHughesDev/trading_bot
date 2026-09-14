# Progress

Per `CLAUDE.md` §8. Factual. A partially-enforced invariant is reported as partial, never as enforced.

Supersedes the first Phase 0 report of 2026-09-13, which described an earlier ledger design (`backtest_trials`, lifecycle columns on the trial row). That design was replaced; see ADR-P0-12.

---

## Phase 0 — foundations · 2026-09-13

### Schema

| Migration | What it adds |
|---|---|
| `0043_trial_ledger.sql` | `mlops` schema: `trial` (registration facts, hash-chained by trigger), append-only chained `trial_event` with the typed `outcome_vector` (no scalar score), `trial_state` view, §9 transition table, `decision`, `ledger_anchor`, sealed-holdout call/attempt, `agent_trajectory` + `trajectory_label`, chained `audit_event`, internal-model registry / promotion / freeze, immutable `gate_profile strict_v1`. UPDATE/DELETE refused by trigger on every ledger table. |
| `0044_roles_grants_rls.sql` | `app_role` / `agent_role` / `internal_ml_role` / `backtest_role`, restricted `platform_app` login; append-only grants; FORCE RLS with one permissive policy per tenant table; `rls_policy_audit`. |
| `0045_jobs_trial_link.sql` | `jobs.trial_id`; counted job kinds must carry one. |
| `0046_dataplane.sql` | `dataplane` schema: surrogate instruments + bitemporal symbols, venues, sources with declared vendor lags, corporate actions, index membership, futures/roll schedules, options contracts/universe, crypto funding/listings/seasonal profiles, chain blocks, feature/label/split/dataset specs, `feature_serving_log`, `feature_consistency_diff`, the feature firewall. |
| `0047_ledger_fixation.sql` | `ledger_tenants()` (SECURITY DEFINER, ids only), append-only `ledger_verification`. |
| `0048_feature_consistency.sql` | `serving_tenants()`, append-only diffs. |
| `clickhouse/07_canonical_market.sql` | `market_bar` (four timestamps, open convention, `Decimal(38,18)`, quality flags, revision sequence), `restatement_index`, `instrument_symbol_dim`, `venue_dim`, `option_bar_1m`, `pool_observation`. |

### Code

- **`crates/ledger`** — the only crate that can mint a `TrialTicket`. Postgres ledger (`pg.rs`), outcome vector, §9 state machine, independent Rust hash verifiers for trials, events and decisions; `anchor.rs` (ed25519 signing, `WormStore`/`FsWorm`, whole-ledger `verify_all`); `trajectory.rs`.
- **`crates/dataplane`** — pure L0/L1 logic: identity, bar contract and knowledge-time provenance, restatement, corporate actions, futures, options, crypto, DeFi, backward-only as-of, alignment, calendars, feature runtime contract, labels, splits, datasets.
- **`crates/features`** — the single feature runtime (`runtime.rs`): every feature is a windowed implementation evaluated through `dataplane::feature::EVALUATE_FN`; `consistency.rs` recomputes and diagnoses a logged serve.
- **`crates/invariants`** — build-blocking static tests over the source tree and a live catalog test.
- **`backtest::store::BarStore`** — the one point-in-time bar reader and writer over `market_bar`.
- **`storage::identity`**, **`storage::clickhouse::canonical`** — surrogate identity service; chunked, verified, idempotent v2 → canonical migration with honest knowledge-time flags.
- **Platform jobs** (`apps/platform`) — `ledger_jobs.rs` (hourly idempotent anchors, 6-hourly verification), `feature_jobs.rs` (daily consistency diff). All run as `platform_app`.

### Invariants enforced by mechanism

| Invariant | Mechanism | Test |
|---|---|---|
| INV-01 knowledge time is real | `write_collected` stamps observed vs backfilled; canonical migration flags REST history `BACKFILLED_KNOWLEDGE_TIME` | `canonical_migration.rs`, `bar_store_pit.rs` |
| INV-02 one PIT read path | `BarStore` only; static scan refuses raw bar-table SQL elsewhere | AT-01 (`static_invariants`), AT-02 |
| INV-03 no stored adjusted prices | no column exists; factors composed at read | AT-03 static + catalog |
| INV-04 symbol never a key | surrogate ids; catalog + DDL scans | AT-05 static + catalog |
| INV-05 `DECIMAL(38,18)` | DDL + catalog scans | AT-04 |
| INV-07 IV not greeks | no greek column | AT-08 |
| INV-10 backward-only as-of | `dataplane::asof` is the only as-of join | AT-10 |
| INV-13 declared lookback | `WindowedFrame` refuses reads past the window; `register_check` refuses features whose output depends on data outside it or is impure; no value until the full window exists | `dataplane::feature` tests, `runtime` tests |
| INV-14 one implementation | one windowed implementation per feature, one evaluation function, used by training frames, backtests, warm starts and live serves; Python duplicate deleted; serves logged; nightly diff | AT-15 static; `features_versioned.rs` (backfill ≡ incremental, bit for bit); `consistency.rs` tests |
| INV-16 no compute without a registered trial | type (`&TrialTicket`, private constructor, separate crate) on every dispatch path | AT-19 static; ledger tests |
| INV-17 everything recorded, censoring set | DB CHECKs + event validation | `pg_ledger.rs` |
| INV-19 hash-chained, append-only | trigger-computed chains, UPDATE/DELETE refused everywhere, independent verifiers, daily signed anchors to WORM, verification every 6 h recorded | `pg_ledger.rs`, `pg_anchor.rs` (a forger who rehashes the whole chain still fails against the signed head) |
| INV-20 propensity logged | DB CHECK + validation; legacy marker the only exception | `pg_ledger.rs` |
| S-1 restricted runtime role | `platform_app` NOSUPERUSER NOBYPASSRLS, owns nothing, cannot CREATE | `db_invariants.rs` |
| S-2 tenant isolation | FORCE RLS on every tenant table, transaction-local `set_config` only | AT-37 `pg_tenancy.rs`; static no-bare-SET; catalog check |
| §14.6 trajectories | recorded inside `dispatch_tool` for every front door | static `every_tool_dispatch_is_trajectory_logged`; `pg_trajectory.rs` |
| INV-15 computed embargo, purge on t1 | `walk_forward_folds` requires the pipeline's `EmbargoInputs` and never goes below `compute_embargo`; `split_spec` CHECKs refuse an unexplained lower embargo or `t0` purge | `dataplane::split` tests; `features` walk-forward and leakage-harness tests |
| INV-18 return series persisted | `mlops.trial_return_series` (append-only, RLS); CHECK `chk_event_returns_persisted` refuses a Sharpe without its series; the backtest path persists before settling | `pg_neff.rs`, ledger unit tests |
| INV-22 platform N_eff | sealed `NEff` minted only by `TrialLedger::n_eff` (average-linkage clustering over every stored series); Gate 3 requires it; BHY replaces Šidák | `neff` unit tests; `pg_neff.rs` (AT-28) |
| §12.7 sealed holdout | ledger claim per strategy lineage, admitted once by a partial unique index before any data is read; repeats return the first result with a notice and are logged | `a_lineage_evaluates_its_sealed_holdout_once` (AT-33); `pg_holdout.rs` |
| INV-21/22/23 tool-schema halves | no tool accepts exploration floor, trial count/N_eff, thresholds, deflation, smoothed regimes | AT-24, AT-27, INV-23 static tests |

### Bugs found and fixed along the way

- **Fabricated knowledge time**: v2 stamped REST backfills with their close time, so history looked observed-live. The canonical migration flags them.
- **Venue blending**: symbol reads mixed venues. Reads now resolve one venue by declared quality tier (ADR-P0-14).
- **Backtest trade double-counting**: `run_simulation_detailed` harvested a round trip twice when it closed before a chunk boundary and was re-opened later, because nautilus snapshots positions under a renamed `{id}-{uuid4}` id. Keyed on the base id; regression test added.
- **Recursive indicators**: EMA/RSI/OBV depended on unbounded history, making backfill and live disagree. Replaced by windowed implementations (ADR-P0-19); feature versions bumped to 2.

### What is not enforced yet — reported, not hidden

- **0.23 object-storage prefix isolation** — deferred: no tenant-scoped object writes exist.
- **Trajectory gap** — the local driver's harness-internal loop tools (`search_tools`, `record_finding`) are not recorded (ADR-P0-18).
- **Decision writers** — `mlops.decision` has no production writer until campaign policies exist (Phase 2).
- **Consistency diff live run** — the diff logic is unit-tested; the scheduled job has not yet been exercised end to end against a live serving log.
- **Fabricated Gate 3 inputs in the Suite funnel** — `SuiteManager::advance_funnel` (`crates/backtest/src/suite.rs`) still evaluates Gate 3 on a hardcoded null distribution (`0..999 / 1000`), a hardcoded observed statistic and hardcoded corroborator inputs. N_eff and the BHY correction are now real, but the significance they are applied to is not. This is a known violation, not a pass: the full gate stack (checklist 2.14) must replace these constants with the Experiment's actual null runs and outcome statistics.
- **Platform restart** — migrations 0043–0048 have not been applied to the developer database yet; the next platform start applies them and now requires `PLATFORM_DB_APP_PASSWORD`, `PLATFORM_LEDGER_SIGNING_KEY` and `PLATFORM_WORM_DIR` (present in the local `.env`).

---

## Phase 1 — correctness · 2026-09-14

Phase 1 is complete: 1.4, 1.5, 1.7–1.12, 1.14 and 1.15 were built this pass, joining
1.1, 1.2, 1.3, 1.6, 1.13, 1.16, 1.17 and 1.18 from the previous one. Phase 0's two
open exceptions are also closed (0.21 below; 0.23 remains deferred, still for the
same reason — no tenant-scoped object writes exist).

### What was implemented

| Item | Spec | What it is now |
|---|---|---|
| 1.4 / 1.14 dataset content hashing | §3.1, INV-12 | `DatasetManager::plan` resolves a request into a `dataplane::dataset::DatasetSpec` **before dispatch**: surrogate instrument ids (materialized, not a re-runnable universe query), the point-in-time anchor, the quality exclusion mask, the pinned calendar versions, the adjustment policy, the versioned feature-DAG hash, the label and split spec ids, and a REQUIRED `runtime_image_digest`. `dataset_id` is that spec's hash, and it is what the trial row records. Persisted to `dataplane.dataset_spec`; the Parquet object is addressed by the same hash. |
| 1.5 label spec | §3.4 | The request carries a typed `dataplane::label::LabelSpec`. `sample_weight_method` has no serde default, so an untyped spec does not parse. Registered in `dataplane.label_spec`; `overlapping_labels_unweighted` propagates into `TrialSubject`. |
| 1.7 causal access guard | §12.5·1 | `features::leakage::causal_access` runs a whole pipeline under the windowed proxy and then probes the *effect*: the value is recomputed with all later rows replaced by noise, and with all rows before the declared window replaced by noise. Movement is a blocking finding. |
| 1.8 random-label test | §12.5·2 | `features::leakage::random_label` over the production fold geometry, two probes (ridge + 1-NN), demeaned by the train block's drift, thresholded on the t-statistic. Runs nightly (`apps/platform/src/leakage_jobs.rs`) and in CI. |
| 1.9 snapshot reproducibility | §12.5·3 | `dataplane.dataset_frame_digest` records the digest of the bytes a `dataset_id` first produced, with the request that produced them. The nightly job replays each recent spec verbatim and compares. |
| 1.10 CV − WF gap | §12.5 | `Experiment::cv_wf_gap()` / `suspect_overlapping_label_leakage()`, surfaced on `ExperimentView`. A flag, not a block. |
| 1.11 synthetic leak injection | §13 | `features::leakage::inject` plants four labelled leaks; two new detectors (`target_correlation`, `full_sample_normalization`) were needed and built. |
| 1.12 master-clock densification | §2, INV-11 | `features::align` is the one densifier: sparse bars → the epoch-anchored UTC grid, every carried value flagged `INTERPOLATED` with its age. Feature columns are emitted with `_age_minutes` and `_quality` companions in the dataset Parquet and in the live serve. |
| 1.15 gate profile | §12.3, INV-23 | `backtest::gates::profile`: typed thresholds with no serde defaults, no setter, `superseding()` the only way to change a number. Every `GateVerdict` carries its `profile_id`; `Comparability::of` flags a comparison spanning profiles. |
| 0.21 consistency diff | §3.3, INV-14 | Now exercised end to end against live Postgres + ClickHouse, not only unit-tested. |

### 2.11 — the post-hoc pipeline

`apps/model-trainer/app/posthoc.py`: soup → Caruana greedy ensemble (with
replacement) → calibrate on the dedicated `cal` role → closed-form threshold.
One function, one order, no reorder flags, and a step that does not apply is
**recorded as skipped** so a GBDT report still shows four steps.

AT-35 is enforced twice. `decision_threshold(costs: CostMatrix) -> float` is the
whole signature — there is no data in scope to tune against — and a static test
asserts its body contains no loop, grid or argmax. On the Rust side
`dispatch_posthoc` refuses a response whose steps came back in a different
order. Binning calibrators (isotonic, histogram) are refused by name with the
reason: a free bin count and step-function plateaus that invent confidence the
model never expressed.

### 2.3 — `max_gpu_hours`, the requirement that survived Kueue

Refused at submission (`422`), enforced in the worker by a hard timeout that
settles `budget_exceeded` → `right_budget`. The rule follows the **worker
class** rather than a list of kinds, so a new trainer kind inherits it. There is
no default anywhere on the path, which is the entire mechanism: a default is a
number nobody chose about somebody else's GPU (AT-66).

### 2.14 — the remaining gate evaluators

All sixteen now exist. Gates 2 and 3 are the wirings: the leakage suite's latest
`mlops.leakage_run` (a missing check *or* a stale run is **inconclusive**, never
a pass) and the breakeven cost multiple interpolated off the `CostSweep` ladder
(a ladder that stops below the bar is inconclusive rather than "survived
everything we tried"). Gate 4 measures participation against dollar ADV and
capacity off the measured AUM curve, and refuses to extrapolate past it. Gate 10
regresses net returns on the asset class's **declared** battery with Newey–West
HAC standard errors — HAC because strategy residuals are autocorrelated and
heteroskedastic and both inflate a t-statistic toward finding alpha. Gate 15 is
**inconclusive** without kill criteria registered before deployment: a stopping
rule chosen after watching the result is not a stopping rule.

### 2.15 / 2.18 — the two countermeasures that are grants and rows

`campaign.platform_seed` is generated by the database and `agent_role` holds no
grant on the column in any direction. One reader exists in the tree, it runs as
the platform role, and AT-63 checks both halves — the static absence from every
API type, and `has_column_privilege` asked of the database itself.

The audit trail's `pre` record is written **before** the dispatch, by the API
layer rather than the harness. The property is an ordering, which is the kind of
thing a refactor reverses silently, so AT-64 pins it in the source for both
dispatch paths and enumerates every `dispatch_tool` caller — a new unaudited path
fails the build rather than appearing quietly.

### 2.14 Gate 16 / AT-65 — capital is a type

`ledger::capital::AllowedFraction` takes five values and has no public
constructor. Raising one requires a `RampAuthority`, which `authorise` mints only
for a profile whose `authorises_capital` is true and only on sixteen passes;
`paper_v1` is false, so sixteen passes under it cannot produce the token and
therefore cannot produce a fraction above zero. Not "should not" — cannot
construct. Lowering needs no authority at all, because a reduction that requires
approval is a reduction that happens after the loss.

Migration 0053 says the same thing to every other writer: `capital_authorisation`
is append-only and RLS-scoped, the fraction is one of five rungs, the direction
must match the arithmetic, a raise needs sixteen passes *and* a capital-
authorising profile *and* exactly one rung, and the ladder is continuous — a row
whose `previous_fraction` disagrees with the current tip is refused.
`mlops.current_capital` is what the risk gate reads; `agent_role` may read it and
may not write it.

### 2.16 / 2.17 — the tool surface and the envelopes

§15 is a **checklist over the one catalogue**, not a second tool surface — three
tool-surface designs would mean three places for a forbidden field to appear.
`spec_15_tool_surface_conformance` holds §15's twenty-three tools as data, maps
the seven that AGENT-002 answers today, marks the rest pending, and fails in both
directions: a mapped tool that disappears *and* a pending tool that appears
without its conventions. So the list can only go stale by failing the build.

The four approval envelopes are one rule at the top of `harness::policy::decide`,
ahead of the risk, attendance and allowlist rules. A session allowlist cannot
cover a promotion, and a human at the keyboard is not taken for a human who
agreed. `approval_spend_usd` joins `delta_practical` as a REQUIRED DEFINE field
with no default anywhere on the path — migration 0054 even drops the column
default so an INSERT that omits it fails.

---

## Phase 3 — the knowledge plane · 2026-09-14

Started at the plan's own starting point: **schemas and GRANTs first**, because
AT-42 is build-blocking and costs nothing, and the rule tier makes Gate 11 real
immediately (ADR-P3-02).

### The three separations, each a grant rather than a convention

- **Smoothed regimes are unreachable from anything that evaluates.**
  `regime_causal` and `regime_research` are separate schemas.
  `backtest_role`, `agent_role` and `internal_ml_role` hold no USAGE on the
  research one — not "no SELECT on the table", no USAGE on the schema. Smoothed
  probabilities are `P(state | all data)`, worth roughly 2.2× Sharpe inflation to
  anything that sees them, and the inflation is invisible in a backtest because
  nothing about the number looks wrong. `api::knowledge` also offers no smoothed
  read, and a unit test asserts the module never names one (AT-42 ⛔).
- **No raw point estimate leaves the recommender.** `dr_estimate` is granted to
  `internal_ml_role` alone; every other caller reads
  `knowledge.recommendation_public`, which does not have the column. A grant
  widened later still cannot leak it, and there is no parameter that disables
  shrinkage (AT-44 ⛔).
- **Fingerprints are not features.** AT-68 scans the feature runtime for any
  mention of an embedding dimension. INV-14's whole claim is one windowed
  implementation per feature on one clock; a fingerprint is computed over a
  21-day trailing window by a different job on a different schedule, and letting
  one in by name would put it in a bar-aligned feature vector.

### The rule tier runs

`apps/platform::regime_jobs` labels daily returns by 21-day volatility tercile
every 24 hours, through the single PIT reader, and writes filtered states under
`model_version = rule_vol_tercile_v1`. Two small decisions are recorded as ADRs
rather than left as choices: the version is a **name**, not a fit date, because a
fit date on something never fitted is a lie in the column readers use to tell the
tiers apart (ADR-P3-06); and `regime_scopes` has **no default**, because "the
market" is a different series for crypto, equities and futures and a default
would produce authoritative-looking labels for something the strategy does not
trade (ADR-P3-07).

### Kept honest

- The as-of bound on neighbour lookup covers **both ends**: the query embedding
  and every candidate. Bounding only the candidates leaks, because the vector
  being searched *from* carries the future into every neighbour list built on it
  (AT-43).
- The outcome-tensor fact table stores the propensity and the censoring beside
  every metric, so a stopped trial enters as a censored observation rather than a
  missing row (AT-45).
- The Postgres image is `pgvector/pgvector:pg16` in compose and CI. Exact kNN, no
  ANN index: at this scale that is ~1 ms, and an approximate index's recall
  depends on its own build state, which makes a neighbour list something an
  attacker can probe (ADR-P3-05, R-14).

### What is not built

- **The learned tiers.** No HMM/HSMM, no BOCPD, no monthly refit, no OOS
  regime-conditional separation test. The rule tier is the model.
- **Nothing writes `asset_embedding`.** The table, the grants and the bounded
  read exist; the `InstrumentProfile` sidecar that computes tier-1 fingerprints
  (3.1) does not, so `neighbours` returns an empty list and says so honestly.
- **Nothing writes `outcome_tensor_fact` or `recommendation`.** The shapes and
  the grants are there; the projections from the ledger are not (3.7, 3.8).
- **The insight store has no retrieval endpoint**, no 4K-token cap, no
  decay-on-contradiction and no embedder (3.10).

---

## Phase 4 — internal models · 2026-09-14

ADR-P4-01's decision, carried out: **the ladder and every rule tier now; each
learned tier behind a written-down ledger-size trigger.** At this platform's
scale the ladder is not a fallback, it is the product — the reference's learned
thresholds (M4 ≈ 1 500 runs over ≥ 30 tasks, M5 ≥ 500 gated candidates *with*
≥ 100 passes, M8 ≥ 100 tasks) will not be met for a long time.

### The ladder is a type, not a log line

`Answer<T>` carries its `Rung` and the `LadderEvidence` it was chosen on, and
they cannot be separated. A number whose provenance is optional is a number that
gets reported without it, and the rule-versus-learned A/B this platform will
eventually want is then a query rather than an excavation of old logs.

Two details worth stating:

- **`Rung::Analytic` records as `decision_tier = rule`.** The ledger's question
  is "did a fitted model decide this"; for a closed form and for a tuned table
  the answer is equally no. The distinction is kept in the code because it is the
  difference between a formula and a table somebody tuned, and only the second
  can rot.
- **A met trigger is not a trained model.** `choose_rung` takes
  `learned_available` separately from the evidence, so a platform holding enough
  data but no fit answers `rule`. Reporting `learned` because the data exists is
  the same lie ADR-P5-01 names in the dashboard.

### The three rule tiers

`jobs::models` holds M1, M2 and M11, and each is honest about what it does not
know:

- **M1** returns a *band*, ×3 either way. The formula counts multiply-accumulates
  and nothing else; a run that waits on its dataloader takes twice what it says,
  and a narrow band would be a claim about I/O the model never made.
- **M2**'s third answer is `NothingPredictable`, not "it will work". Two failures
  are visible before a run starts — the working set not fitting, the learning
  rate outside its optimiser's envelope — and everything else is found by
  running. The predictions name the `TerminalReason` they expect, so a prediction
  and an outcome compare without a mapping.
- **M11** runs hard rules first, then a **local** robust-z and a zero-centred
  CUSUM. Both choices came out of failing tests: a global z-score calls a healthy
  decelerating curve's whole first half anomalous, and a single wild value
  inflates the global spread enough to mask itself — the exact case the detector
  exists for. Centring CUSUM on the observed mean increment would subtract out
  the drift being looked for.

### The four collapse guards

A platform that learns from its own decisions can spiral: the policy's outputs
become the next policy's training data, the distribution narrows, every internal
metric improves and the thing gets worse. Each guard is a refusal:

1. `TrainingRange` has **no `from` field**. A set is `[0, seq_max]`; "start
   later" is not expressible.
2. `TrainingSet::build` refuses a set with no `paper`/`live` outcome. A model
   trained purely on backtests has learned what the simulator does.
3. The seed holdout is the **first 200 trials**, frozen once (migration 0056,
   one row per tenant ever, append-only). A prefix is reproducible from the
   ledger by anyone and cannot be re-drawn; `agent_role` holds no grant on it,
   because an agent that knows the holdout can avoid resembling it. AT-50 is a
   type check rather than a report: overlapping trials will not build.
4. `EntropyFloor` blocks a Tier-B promotion below `0.5·ln k` — and also when the
   floor is **unmeasurable**. A platform that cannot tell whether its policy has
   collapsed does not get to promote on the strength of not knowing.

The freeze switch is checked in the same constructor: a frozen platform cannot
build a training set at all, rather than building one and remembering not to use
it.

### What is not built

- **No learned tier exists for anything.** That is the decision, not a gap — but
  it means every `decision_tier` in the ledger will read `rule` for a long time,
  and the dashboards should say so rather than implying a model is thinking.
- **4.11's learning-debt trigger and 4.12's promotion machinery** are not built:
  the registry, promotion and freeze tables exist and are append-only, but
  nothing runs a challenger against a champion on the frozen holdout.
- **4.13's KL and Wasserstein** distances against the seed holdout are not
  computed. The two structural guards are; the two distributional ones are not.
- **M3–M10, M12, M13** have no implementation beyond their triggers being stated.

---

## Phase 5 — surfaces · 2026-09-14

Two surfaces, both built because the thing they render already exists and would
otherwise be invisible.

### The self-monitoring panel renders five states, not two

`PlatformHealthPanel` shows a **word** where a number would go for `not fitted`,
`n/a` and `unavailable`, and its summary counts alarms, unavailable signals and
unfitted ones separately. "Fourteen of sixteen healthy" would fold a broken query
and an untrained model into the same number as a real pass, which is precisely
the failure ADR-P5-01 exists to prevent.

Two colour choices are decisions rather than taste. An unfitted signal is
**neutral**, not `warn`: it is a thing that has not happened yet, and colouring
it as a fault trains readers to ignore the colour. An unavailable one is
**`warn`**, not `neg`: it is a fault in the monitoring rather than a finding
about the thing monitored, and reading it as an alarm would make a broken query
indistinguishable from a real breach.

The panel sits above the model list rather than behind a tab. A self-monitoring
panel nobody navigates to is one nobody reads.

### The gate board shows sixteen rows and says which ones ran

`GateStackBoard` puts `profile_id`, `N_eff` and the **trial count at evaluation**
on the same row as every statistic — a Sharpe of 1.8 after four looks and after
four thousand are different claims, and §12.2's argument is that the second is
mostly the search. A gate with no recorded verdict renders as `not recorded`
rather than being omitted or padded: twelve recorded verdicts is not four
failures, and the two must not look alike.

### What is not built

- **5.1's bounded drop-oldest telemetry queue** on the trainer side (AT-55).
- **5.2's writer budget.** The static test that stops an instrument id reaching a
  metric name is live; the ≤ 100k keys/tenant cap and the `per_instrument_pnl`
  artifact are not.
- **5.4, 5.6 and 5.7.** The comparison/lineage view, the agent action timeline
  over `audit_event` and `decision`, and the preset/guided/expert modes. All
  three have their backends now — `stats::compare`, `ledger::audit`, the DEFINE
  schema — and no surface.
- **5.8** stays deferred (ADR-P5-03).

---

## Closing out Phases 2–5 · 2026-09-14

**78 done, 19 partial, 5 open** — and the five open items are all of Phase 6,
deferred by explicit decision with a written revisit trigger each (a measured
probing win, 100k executor-calls/day, ten concurrent analysts, a measured
exact-kNN p99 failure, a model that actually needs more than one node). Phases
0–5 have no unstarted item left.

### What the last pass added

- **2.4 / 2.5** — the checkpoint contract is a sealed type the artifact registry
  calls before storing any checkpoint, refusing every missing field *by name*.
  `plan_resume` has three outcomes and **none of them mints a trial**; an invalid
  stored checkpoint is `Abandon`, not a quiet restart, because something wrote it
  and treating it as merely absent would hide that.
- **2.6 / 2.7 / 2.8 / 2.10** — the search stack. Searcher selection is §11.1's
  table as a pure function with its rationale attached; the √D lengthscale prior
  puts the GP's prior mass where the distances actually are; all five ASHA
  mitigations are defaults with no per-mitigation switch, the last of which
  re-runs the top three on platform-held seeds before an incumbent is crowned;
  and a stop is `right_asha` censored, never failed.
- **3.1 / 3.2 / 3.4 / 3.6 / 3.11** — fingerprints with EDGE, seasonal deflation,
  whitening, mutual-proximity hubness correction, greedy portfolio coverage and
  the three-part embedding validation. The rule running through all of them:
  **absent is not zero**, and an unmeasured criterion fails.
- **4.5 / 4.6 / 4.8 / 4.10 / 4.11 / 4.12 / 4.17** — the remaining rule tiers and
  the promotion machinery. `authorise_promotion` is sealed and refuses by name on
  four conditions; only Tier A rolls itself back.
- **5.1 / 5.4 / 5.6 / 5.7** — the drop-oldest telemetry queue, the comparison
  matrix, the agent action timeline and presets that suggest without supplying.

### The pattern, stated once

Nearly every decision above took the same shape: a thing that is normally a
setting was made a rule, and a thing that is normally silent was made loud.
Searcher choice, the ASHA mitigations, the checkpoint fields, the preset's three
numbers, the entropy floor, the embedding validation — in each case the
alternative was a switch, and a switch is what somebody turns off at 2 a.m. to
make the sweep finish. The absences are the other half: `not fitted`, `not
recorded`, `not measured`, `Unknown`, `NotComputable`, `None` — each one a state
the type system carries rather than a zero that reads as an answer.

### What remains, and it is all in the partials

The nineteen partial items say in their own entries what is missing. The
recurring shape is that a mechanism exists and its *evidence source* does not:
five gates have no input yet, no learned tier is fitted anywhere (by decision),
nothing writes `asset_embedding`, and the campaign's child workers other than
`GateAdvance` are unbuilt. None of that is hidden — every one of them renders as
an explicit absent state rather than as a number.

---

## The migrations were applied and the live tests were run · 2026-09-14

Everything above was written against a database nobody had run it on. That is no
longer true, and running it found three real bugs — all of the same kind: a
constraint that *looked* like it enforced something and did not.

### What was done

- The Postgres image is `pgvector/pgvector:pg16` in compose and CI. The dev
  volume was initialised under alpine's musl, which records **no collation
  version**, so Postgres compared nothing and warned about nothing — while every
  text index had been built under different sort rules than glibc uses. The
  database was dumped (2.1 MB, 71 tables) and `REINDEX DATABASE` was run before
  anything else touched it.
- `cargo run -p storage --example migrate` is the new runner. The platform
  applies migrations on boot, which is right for the platform and useless for a
  test fixture, a fresh developer database or a CI service container. It refuses
  to run without `DATABASE_URL`: a migration runner that guesses its target is
  one that eventually guesses production.
- **Migrations 0001–0057 apply cleanly to a fresh database, and 0043–0057 applied
  to the developer database, which is now at 57.**
- **28 live Postgres tests pass**, plus 19 job-store tests against a real
  database: `db_invariants` (AT-42 ⬛, AT-44 ⬛, AT-63 ⬛, the catalog scan),
  `pg_campaign` (AT-59), `pg_ledger`, `pg_neff`, `pg_tenancy`, `pg_holdout`,
  `pg_anchor`, `pg_trajectory`, `pg_gate_profile` (AT-29), `pg_self_monitor`, and
  `pg_feature_consistency` end to end against live Postgres *and* ClickHouse.

### The three bugs, all the same shape

**1. `array_length(x, 1) >= 1` does not require a non-empty array.** For an empty
array `array_length` returns NULL, `NULL >= 1` is NULL, and **a CHECK that
evaluates to NULL passes.** So the obvious spelling of "this must not be empty"
accepts an empty one — silently, in four constraints the pack calls REQUIRED: the
leakage suite's `checks_run`, a gate profile's `factor_battery`, an insight's
`evidence_trial_ids`, and the seed holdout's `trial_ids`.

AT-42 found it by trying to insert an insight with no evidence and watching the
database accept it. Reading the DDL would never have found it, because the SQL
says exactly what it means. All five uses now say `cardinality(x)`, which is 0
for an empty array, and
`no_check_requires_a_non_empty_array_with_array_length` fails the build on the
next one.

**2. Migration 0051's backfill could not run on a database that had been used.**
`chk_counted_jobs_have_trial` was added NOT VALID by 0045 precisely so that jobs
predating the ledger could stay; *any* UPDATE to such a row re-checks it and
fails. The backfill now steps over them, and the new
`chk_job_error_has_terminal` is NOT VALID for the same reason — the enforcement
that matters is on INSERT and UPDATE, which NOT VALID gives in full, and a
constraint that stops the next bad row is worth more than one that refuses to be
added at all. One grandfathered row remains on the developer database, and
reading it back fails loudly at the deserialiser, which is the right place for a
row nothing can interpret.

**3. `pg_campaign`'s fixture never applied 0054**, so every `define_campaign` in
it wrote a column that did not exist. A fixture that lists its own migrations
drifts from the migration set every time one is added.

### Still not run

- **Four `storage` live suites fail, and they were failing before this work:**
  `ledger_append`, `ledger_writer` and `pnl_schema` all need a `ledger_events`
  table **that no migration in this repository creates**, and `registry_seed`
  expects 8 asset classes where migration 0006 seeds 6. Those four files are
  unchanged from `HEAD`. They belong to the execution subsystem, not this pack,
  and fixing them means deciding whether the test or the migration is right —
  which is a call for whoever owns it.
- ClickHouse's container healthcheck reports unhealthy while the server answers
  queries normally. Pre-existing, and noted in the memory file.

### Invariants newly enforced by mechanism

| Invariant | Mechanism | Test |
|---|---|---|
| INV-11 staleness companions | one densifier (`features::align`); `build_aligned_training_frame` emits `{f}_age_minutes` / `{f}_quality` per feature; the Parquet schema has all three columns per feature; the live serve returns all three | AT-11 `a_cross_asset_frame_exposes_staleness_across_a_holiday_and_a_weekend`, `align` unit tests, live `pg_feature_consistency` |
| INV-12 dataset is a hash over its full spec | `DatasetSpec` resolved and hashed before dispatch; the trial row carries it; the artifact is addressed by it; `dataset_frame_digest` makes the "identical id ⇒ identical bytes" claim falsifiable | `datasets::tests` (label/feature-set/image/calendar all enter the hash), the nightly replay |
| INV-14 one implementation, diffed nightly | unchanged, **plus** the diff job now densifies identically to the serve, so a data gap cannot read as code drift | `pg_feature_consistency` end to end |
| INV-23 gate profiles immutable and versioned | DB trigger refuses UPDATE/DELETE; the Rust type has no setter; thresholds have no serde defaults | AT-29 live (`pg_gate_profile`), `profile` unit tests |

---

## Phase 2 — the training system · 2026-09-14

Phases 2–5 were re-scoped first (`backlog/PHASE-2-5-PLAN.md`), with every item
tagged BUILD / BUILD-AS / N/A / DEFER and an ADR behind the tag. ADR-P2-04…P5-03
record the decisions; no open question remains in `OPEN-QUESTIONS.md`. What
follows is what was then built, in the plan's own build order.

### 5.3 — platform self-monitoring, built first

§16.2 calls this "the layer nobody builds", and the plan builds it before
anything else on the grounds that a platform which cannot report on its own
judgment cannot tell whether the rest is working (ADR-P5-01).

- `crates/api/src/self_monitor.rs` — eleven signals over the ledger, each with a
  five-state `SignalState`. `NotFitted` and `Unavailable` are **explicit states**,
  not zeros: a dashboard that renders an unfitted model as 0 % is lying, which is
  the specific failure ADR-P5-01 exists to prevent. `HealthReport` separates
  alarms from unavailable signals so a broken query cannot read as a healthy one.
- `crates/api/src/routes/platform_health.rs` + `/api/platform/health`, RLS-scoped
  through `ledger::pg::tenant_tx` like every other read.
- `apps/platform/src/health_jobs.rs` — hourly; P1 logs `error!`, P2 `warn!`,
  unavailable signals separately from failing ones.
- Migration `0050_gates_v2.sql` — `mlops.gate_verdict` (tenant-scoped, FORCE RLS,
  append-only, `gate_no BETWEEN 1 AND 16`) with a CHECK that gates 8 and 14 may
  not record a pass without the statistic that justifies it; `gate_profile_asset_class`;
  `gate_profile.authorises_capital`; the `paper_v1` profile; and
  `campaign.platform_seed` with **no SELECT grant for `agent_role`** (2.15's
  mechanism, ADR-P2-18).
- `crates/ledger/src/gates.rs` — `GateRecord` with `structural()` / `measured()`
  constructors that mirror the DB CHECKs, and the `GateLog` trait.
- Live-tested end to end (`crates/api/tests/pg_self_monitor.rs`, 4 tests).

### 2.14 — five of the eight missing gate evaluators

- `backtest::stats::bootstrap` — Politis–Romano stationary bootstrap,
  Politis–White automatic block length, Romano–Wolf stepdown. The resample floors
  (1 000 for a CI, 5 000 for a stepdown) are constants, not parameters, and
  `MIN_OBSERVATIONS` refuses a series too short to resample honestly.
- `backtest::gates::evaluators` — Gate 9 (minimum length against the multiple-looks
  noise boundary), Gate 11 (regime coverage on vol terciles), Gate 12
  (perturbation plateau), Gate 13 (stationary bootstrap), Gate 14 (Romano–Wolf).
  `GateOutcome::inconclusive` is a third state that is **never** a pass.
- `paper_v1` is a second immutable profile that drops only the calendar floor and
  carries `authorises_capital = FALSE`; a test asserts the noise boundary and the
  event floor still bind under it.

Two design gaps surfaced as failing tests and were fixed as design, not fixtures:

- **Gate 12's concentration cap is universe-relative** (ADR-P2-28). A flat 20 %
  cap is arithmetically unsatisfiable below five instruments — one name must hold
  ≥ 1/n — so the flat cap tested universe size rather than concentration. The cap
  is now `max(profile_cap, 1/n)` and the verdict says when the universe is what
  binds.
- **Gate 11 labels regimes from the market series, not the strategy's**
  (ADR-P2-29). A vol-targeted strategy has near-constant own volatility by
  construction, so its own terciles are noise rather than regimes; and attribution
  over *signed* returns let a losing regime contribute a negative share and lower
  measured concentration. Shares are now over net gains, and a verdict computed
  without a market series is flagged `labelled_from_strategy`.

### 2.19 — sample weighting is true rather than flagged

`TrainingFrame.sample_weight` carries average-uniqueness weights, computed over
the **densified clock** and then subset to surviving rows (computing them over
survivors alone would understate concurrency, because a dropped warm-up row's
label still overlaps the ones that follow). The weight reaches the parquet
dataset, `Prepared.w_tr`, and all seven trainer adapters — LightGBM/XGBoost
`weight=`, sklearn `sample_weight=` with a `TypeError` fallback, torch with
`reduction="none"` and per-batch renormalisation so the learning rate does not
silently change meaning. `RESERVED_COLUMNS` closes a leak found on the way:
`select_dtypes` had been handing the model `ts_ns` (a tree can split on "before
or after March") and `sample_weight` (a function of the label's own overlap) as
features. The label spec now declares `uniqueness` because that is what runs
(ADR-P2-22, AT-67), and `overlapping_labels_unweighted` stops being true of every
trial.

### 2.2 — the terminal-reason mapping is typed, and there is no sniffing left

Four settlement paths recovered a `TerminalReason` by substring-matching an error
message (`contains("nan")`, `contains("data")`) and defaulted everything else to
`dependency_failure`. All four are gone (ADR-P2-30). The reason is now carried by
the type:

- `JobError.terminal: TerminalReason` — REQUIRED, no serde default. Migration
  `0051` backfills rows written before the field existed (running the old guess
  once, in SQL, where it can be read) and a CHECK refuses new ones without it.
- `RunStatus::Failed(TerminalReason)` — a Run that failed without saying how is
  now unrepresentable.
- `sim_executor::ExecFailure` and the backtest driver's `PhaseFailure` name the
  reason at the point of failure, where what went wrong is known.
- The trainer sidecar classifies its **own** exceptions, by type rather than
  message (`apps/model-trainer/app/failures.py`), and returns `terminal` on the
  wire. `train_torch_model` raises `NanDivergence` on a non-finite loss, which is
  what turns `nan_divergence` from a label into an observation.

AT-60 is three tests: the reason set is exactly §9's, parsing is exact (`"nan"`
and `"loss became nan"` do **not** parse), and `asha_stopped` censors `right_asha`
rather than `failed`.

### 2.1 — the campaign workflow, event-sourced

A campaign's state is a fold over `mlops.campaign_event`; there is no status
column and resumption is replay (ADR-P2-04).

- `crates/ledger/src/phase.rs` — the thirteen §10 phases, the legal transitions,
  and `fold`, which refuses a log that does not open with `define`, an illegal
  transition, or anything appended after an ending.
- Migration `0052_campaign_phase.sql` — `seq` assigned by the database rather than
  the writer, unique per campaign, and a BEFORE INSERT trigger enforcing the same
  transition table against every writer, not just this code. DEFINE now opens the
  log in the transaction that writes the campaign row.
- `JobKind::Campaign` + `api::campaign_driver::CampaignWorker`. Each phase's work
  is a child job at `(campaign job, seq)` through `JobStore::submit_child_once`.
  The child is created **first** and the event written second: a crash in between
  leaves a child nothing points at, which the next fold re-derives and reuses,
  where the opposite order would leave an event claiming a child that does not
  exist.

### 2.12 / 2.13 — the comparison protocol and the matrix

`backtest::stats::compare` is §11.5 in one module. Three things are worth naming:

- **The `Comparison` is sealed.** Private fields, no public constructor, and a
  static test keeps `ComparisonPlan::judge` the only way to mint one. The point
  is not that the arithmetic is hard; it is that a verdict must not be able to
  travel without its profile, `N_eff`, effect size, trial counts and flags.
- **The interval is anytime-valid.** A fixed-sample interval is wrong the moment
  anyone looks twice, and the whole protocol is built on looking until it
  separates. `confidence_sequence` is the predictable plug-in
  empirical-Bernstein CS of Waudby-Smith and Ramdas.
- **The ROPE scale is declared and cuts both ways.** Differences are mapped into
  `[0, 1]` through a pre-registered bound. A generous bound widens the interval
  and makes separation harder; a flattering one makes real observations fall
  outside it and be refused rather than clipped. A test demonstrates both
  directions on the same data, which is what stops the scale being a knob.

The honest consequence of the CS being anytime-valid is that k ≈ 29 separates
only when the declared scale is genuinely tight. That is the protocol working:
twenty-nine paired looks is not much evidence, and the interval says so.

AT-62 is live and scans Rust, TypeScript, Python, SQL and JSON across `crates`,
`apps`, `frontend`, `web` and `ui`.

### Invariants newly enforced by mechanism

| Invariant | Mechanism | Test |
|---|---|---|
| INV-17 every ending is recorded with its censoring | the reason is a field of `JobError` and a payload of `RunStatus::Failed`; `jobs.error` CHECKs its presence; no code path infers one from prose | AT-60 (`ledger::state`, `jobs::types`), `job_store` live |
| INV-23 gate profiles immutable and versioned | unchanged, **plus** `paper_v1` and the per-asset-class rows are equally immutable, and `authorises_capital` is a column the risk gate can read | `pg_gate_profile`, `profile` unit tests |
| §10 phase order | `fold` refuses an illegal sequence and so does `trg_campaign_transition`; `seq` is assigned by the database | `phase` unit tests, `pg_campaign` (AT-59) |
| §11.5 a verdict carries its context | `Comparison` has private fields and one constructor, which cannot be called without the profile, `N_eff`, δ, both trial counts and the flags; a static test keeps it the only one | `only_the_protocol_constructs_a_comparison`, `a_comparison_is_never_a_bare_number` |
| §11.5 no critical-difference diagram | a static scan over eight file types in five trees | AT-62 |
| §12.3 Gate 16 capital is earned, not set | `AllowedFraction` has no public constructor and `raise` needs a `RampAuthority` only `authorise` can mint; migration 0053 enforces the rungs, the one-step rule and the authorising profile against every writer | AT-65 (`capital` unit tests, `gate_16`), `db_invariants` |
| §12.7 the platform seed is unreadable | a column grant `agent_role` does not hold, checked by the database on every read; one reader in the tree | AT-63 static + live |
| §15 a denial leaves evidence | the `pre` record is written before the dispatch in both agent paths; the table refuses a `post` without its `pre` | AT-64 (source ordering + caller enumeration) |
| §8 no compute without a declared ceiling | `require_gpu_budget` refuses a trainer-class submission without `max_gpu_hours`; the rule follows the worker class | AT-66 |
| §11.4 the threshold is arithmetic | `decision_threshold` takes only the cost matrix; a static test asserts no loop, grid or argmax in its body | AT-35 (static + Python) |
| §3.4 overlapping labels weighted | the weight is a column of the frame and of the parquet dataset, and every adapter consumes it; the label spec declares what runs | AT-67 (`training_frame`), adapter wiring |

### What is not enforced yet — reported, not hidden

- **Nothing consumes a `Comparison` yet.** The protocol computes verdicts and the
  matrix displays them, but the COMPARE phase's worker does not exist, so the
  intended consequence — that a promotion path takes a `&Comparison` and a
  `StudyResult` will not compile in its place — is a design consequence rather
  than an enforced one. The seal holds; the substitution ban does not yet.
- **The sixteen-gate stack now has a caller, and most of its evidence is still
  missing.** `api::gate_worker::GateAdvanceWorker` assembles `StackEvidence` from
  the ledger and records sixteen verdicts, so `gate_pass_rate` has rows and
  `GateStackBoard` has something to render. What it can fill today is Gates 2,
  13 and 14 — the leakage run, the stored return series, and the experiment's
  whole candidate family. The other eleven are left **absent**, which makes them
  inconclusive, which is recorded as a failure carrying its reason.

  That direction is deliberate and is the part worth stating. The obvious design
  — fill what you can, report on what you filled — has the property that the
  measured gate pass rate goes *up* as the evidence pipeline degrades. Here it
  goes down, and the board says "3 of 16 recorded" rather than "3 of 3 passed".

  **The channel for the rest exists and is empty.** `mlops.trial_statistic`
  (migration 0057) is an append-only, RLS-scoped store with a *closed vocabulary*
  of eight names, CHECKed by the database and stated again in
  `ledger::TRIAL_STATISTICS`, because a misspelled name is a gate input nobody
  ever finds — and a gate that never finds its input is inconclusive forever
  without anybody noticing. `TrialLedger::record_statistic` writes; the gate
  worker reads and fills Gates 1, 3, 5, 6, 7 and 8 from it. The agent may read
  the table and may not write it: writing one is how a candidate would hand the
  gate its own answer.

  **And the funnel writes to it.** `Backtest::run_traced_with_trial` returns the
  trial id it registered, `run_observed` carries it out, and
  `advance_funnel` records five numbers against the run that produced them: the
  CPCV 5th percentile, the walk-forward median, PBO, the deflated Sharpe and the
  raw permutation p-value. A failed write is logged rather than fatal — the
  funnel's verdict stands, and a gate with no recorded statistic is inconclusive,
  which is the safe direction. Failing the funnel over a bookkeeping write would
  throw away the evidence as well as the record of it.

  So a gate stack run after a funnel now fills **eleven of the sixteen** from the
  ledger: 1, 2, 5, 6, 7, 8, 9, 11, 13, 14 and 16 (which follows from the others).

  Gate 1's evidence is the campaign's own immutable DEFINE hash, read from
  `mlops.campaign_event` — a hash the submitter supplies is a claim the submitter
  could have written after the result, and only the campaign row can attest that
  the claim predates the evidence. Gate 6's regime count is measured from the
  strategy's own volatility terciles over the same 21-day window the labeller
  uses, which is a weaker reading than a market one and a much stronger reading
  than none: "this result spanned one volatility regime" is worth catching now
  (ADR-P2-33).

  Still to build: the cost ladder (3), capacity inputs (4), factor returns (10, Phase 3),
  the neighbourhood study and per-instrument P&L (12, also 5.2), and forward-test
  observations (15). A real market series for Gate 11 arrives with 3.5's labels
  once `regime_scopes` is configured.
- **A campaign cannot yet run to completion.** The driver and the log are built,
  but the child worker kinds it dispatches (`Study`, `GateAdvance`, `EvalTask`,
  `ResearchRun`) have no registered workers, so a campaign fails at its first
  dispatching phase with `no_worker` — visibly, as a failed child, leaving the log
  where it is. `DIMINISHING_RETURNS` and `CONVERGED` need 2.12's confidence
  sequence and are reported by the driver as `NotComputable` rather than
  approximated; only `BUDGET_EXHAUSTED` is live.
- **The trainer's failure classification is not total, deliberately.** An
  exception type nobody has classified maps to `dependency_failure`. That is a
  true statement about a dependency that raised without saying why, but it is not
  the same as knowing, and it is the one place in the chain where a reason is a
  residue rather than a decision.
- **The external MCP front door writes no audit trail.** `apps/mcp-server` is a
  separate process holding a service token; the `pre`/`post` records cover the
  two internal agent dispatch paths only. AT-64 enumerates every caller, so this
  is a named exemption rather than an oversight, but it is an exemption.
- **The trainer sidecar's post-hoc stage has no caller.** `/posthoc` is
  reachable and `dispatch_posthoc` is typed, but the COMPARE phase's worker —
  the thing that would run it — does not exist (2.1's gap).
- **5.3 has no frontend.** The signals are computed, served and logged; the MlOps
  page does not render them.
- **0.23 object-storage prefix isolation** — still deferred; no tenant-scoped
  object writes exist.
- **Trajectory gap** — the local driver's harness-internal loop tools
  (`search_tools`, `record_finding`) are still not recorded (ADR-P0-18).
- **Overlapping-label leakage still has no static detector.**
  `inject::Leak::OverlappingLabelBlocks` is planted and deliberately not caught by
  any check in the suite (OQ-12); it is visible only as the 1.10 CV/WF gap, and
  the test asserts that, so the day a static check does catch it the test fails
  and the claim gets updated.
- **`lint-no-json-hotpath` fails** on one pre-existing line
  (`crates/collectors/src/crypto/kraken.rs:20`). Not a CI job, predates this work.
- **Migrations 0043–0052 are still not applied to the developer database** (it is
  at 42). The next platform start applies them and needs
  `PLATFORM_DB_APP_PASSWORD`, `PLATFORM_LEDGER_SIGNING_KEY` and
  `PLATFORM_WORM_DIR`. Every live-Postgres test in this pass — including AT-59 and
  AT-60's job-store half — is therefore written but has not been run against a
  database; the unit tests have.
