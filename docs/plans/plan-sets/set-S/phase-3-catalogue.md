# Phase 3 — The catalogue

**Completion: 0% (0 / 19 tasks)**

**Requirement IDs:** TC-11…TC-37.
**Spec:** [AGENT-006 §3.3, §5](../../../specs/AGENT-006-tool-catalog.md), and the research
brief [quant-agent-toolbox-uncommon-tools.md](../../../research/quant-agent-toolbox-uncommon-tools.md)
for the evidence behind each tool.
**Depends on:** Phase 1 track A (`ToolSpec`, `ToolResult`).
**Blocks:** Phase 8 (the workflows compose these).
**Migration range:** 0079–0081.
**Parallel with:** Phases 4, 5, 6, 7.

Roughly sixty tools. The phase is ordered so the *mechanism* lands first: once the templated
tier exists, most of the rest is a Python file with a JSON header and a test, which is why
the analysis suite AGENT-002 specified and never built becomes tractable here.

**One PR per task.** Each namespace is independently reviewable, and the description-budget
and namespace-cap tests from S-1.4 are mechanical gates every PR must pass.

---

## The templated tier

### S-3.1 `templated.rs` — tools that are Python files — L
`DONE` 2026-09-16 — `crates/skills/src/template.rs` (the one parser),
`crates/mcp-server/build.rs` (embeds the files), `crates/mcp-server/src/tools/templated.rs`
(the catalogue entries), `apps/skill-runner/preamble.py` (the shared statistics),
`apps/skill-runner/requirements.txt` (what a template may import).

The parser lives in `skills` rather than being mirrored into `mcp-server`, so the catalogue
and the runner cannot read one file two ways — a template declaring one schema to the model
and running under another would present as a model calling a tool wrongly.

Enforced, not documented: a template is `read` (it has no network and no database, so
`write` names a side effect it cannot cause and `outbound` is an attempt at the egress
policy's exemptions); wall-clock ceilings are per namespace and asking for more is a parse
error rather than a clamp; CPU seconds follow the wall clock; `deny_unknown_fields` on the
header, so a typo'd `spend_trial` cannot parse as "no trial"; a summary slot must name a
declared field; and a malformed template panics at catalogue construction rather than
vanishing from the list (ADR-I-01).

Verified by `crates/mcp-server/tests/templates_run.rs`, which is deliberately **not**
`#[ignore]`d — an unrun determinism check is an invariant claimed and not enforced. It runs
every template on the reference dataset twice and compares `detail` byte for byte.
`every_template_on_disk_reached_the_catalogue` walks the directory independently of
`build.rs` so an empty catalogue cannot pass.

Two runner changes this needed: pyarrow imports `socket` during initialisation and the
import guard refused it, so `read_parquet` failed with "Unable to find a usable engine" —
the analysis stack now loads before the door closes, and a skill's own `import socket` still
raises. And `PYTHONNOUSERSITE=1` hides the platform's own numpy on a developer machine, so
`RunnerConfig::stack_path` lets the platform name where its stack is.

**ADR-S3-01** records why statsmodels and `arch` are not in the runner's stack: pinning them
would pin a second, older numpy into a deployment that already has one, and two numpys are
two answers to the same arithmetic. The statistics are written out against numpy and scipy.

A template is a Python file under `apps/skill-runner/templates/<ns>/<name>.py` whose module
docstring carries a fenced JSON `ToolSpec`. `build.rs` reads the directory into `specs()`.
The harness materialises the series the template asks for into a parquet file and passes
`payload.data_path` — a skill has **no network and no database** (`taxonomy.rs:285` asserts
it) so the data must arrive as a file. Limits default to the runner's 30 s / 512 MB / 4 MB;
a template may declare up to 120 s (analysis) or 300 s (audit/attribution, where block
bootstraps are slow).

Templated tools are exposed **even on `code_execution: denied`** profiles: the harness runs
the template, not the model. That is what makes a 7B get the same statistics a frontier
model gets.

**Files:** `crates/mcp-server/src/tools/templated.rs`, `crates/mcp-server/build.rs`,
`apps/skill-runner/templates/`, `crates/skills/src/runner.rs`.
**Acceptance:** a template's header schema equals its catalogue schema (build-time test);
every template runs under the runner's limits on the reference dataset with `seed` fixed and
produces byte-identical `detail` twice; `detail.provenance` names the numpy/pandas/statsmodels
versions used.
**Traces:** AGENT-006 §2.2, §3.3; §10.5.

### S-3.2 Data materialisation and the determinism test — M
`DONE` 2026-09-16 — `crates/api/src/routes/agent_templates.rs`,
`skills::template::DataKey`.

Rows reach a template through `BarStore::as_of` — the same point-in-time read
`GET /api/data/bars` uses, clipped by the same `Project::clip`. There is no second query.

`as_of` is part of the cache key, which is the half that is easy to leave out: it bounds
which *revisions* are visible, so two projects with different horizons asking for one window
are asking for different data. Without it the first caller would decide what the second saw,
which is a future leak with a cache in front of it.

Write-once by rename, so a reader sees no file or a whole one. Losing the rename race is
success: the content is addressed by its key.

Verified by `routes::agent_templates::tests::{a_window_past_the_cutoff_is_clipped_not_served,
a_window_entirely_past_the_cutoff_is_refused, two_horizons_do_not_share_a_materialised_file,
the_materialised_schema_is_the_one_templates_read}`. The static scan refusing `socket`,
`requests` and `subprocess` is `crates/skills/src/lib.rs`'s existing one and is unchanged.

**Files:** `crates/mcp-server/src/tools/templated.rs`, `crates/api/src/routes/`.
**Acceptance:** the same (instrument, timeframe, window) hashes to the same parquet path and
is written once; a template that imports `socket`, `requests` or `subprocess` is refused by
the existing static scan (`crates/skills/src/lib.rs:218`).
**Traces:** AGENT-006 §3.3.

---

## `analysis` — the suite AGENT-002 §6.3 specified and never built

All templated, all take `(instrument, timeframe, start, end, seed)`, all return `ToolResult`.

### S-3.3 Moments, volatility, regime, event study — L
`DONE` 2026-09-16 — `apps/skill-runner/templates/analysis/{describe_returns,
estimate_volatility,detect_regime,event_study}.py`.

Every estimate carries `n`, and `se` wherever it has a closed form worth trusting — the HAC
error on a mean, the quarticity error on realised variance, the finite-sample errors on
skewness and kurtosis rather than the asymptotic `sqrt(6/n)`, which is badly optimistic at
the sample sizes an agent will actually ask about.

`event_study`'s control is matched on volatility decile and its p-value comes from a block
bootstrap whose block is at least the horizon, because events within a horizon of each other
share forward bars. It has no threshold-sweep mode: sweeping until the p-value drops is the
search this platform counts trials to prevent, and offering it as a convenience would make
the catalogue the easiest place to do it by accident.

Reproducibility is asserted rather than asserted-about: `templates_run.rs` runs all four
twice on the reference dataset and compares `detail` byte for byte.

Two real bugs this found on the way, both of the kind that produce a confident wrong answer:

* `_hurst` was handed the cumulative price path, and it forms the cumulative deviation
  itself. Double-integrating returns H ≈ 1.0 for a random walk — every series would have
  been reported as strongly trending.
* the reference fixture's first GARCH used a raw Student-t, whose variance is `nu/(nu-2)`,
  so the effective persistence was 1.03 and the price path overflowed. The generator now
  standardises the innovation and asserts stationarity.

`describe_returns` (moments ± SE, JB, Ljung-Box on r and r², ARCH-LM, Hill, max DD, jump
share), `estimate_volatility` (PK/GK/RS/YZ, RV/BV/RQ, EWMA/GARCH/HARQ forecasts, percentile),
`detect_regime` (vol tercile / HMM / Hurst-VR-ADX), `event_study` (forward paths vs matched
random entries with block-bootstrap bands).

**Acceptance:** every estimate carries `n` and, where defined, `se`; the event study's random
control is matched, not naive; results are stable across two seeded runs.
**Traces:** AGENT-006 §5.2; AGENT-002 §6.3.

### S-3.4 Liquidity, correlation, seasonality, stationarity, cointegration — L
`NOT STARTED`

`estimate_liquidity_cost` (EDGE, Roll, Corwin-Schultz, Amihud, sqrt-impact — wraps what
`check_tradeable_frontier` already computes for one horizon), `rolling_correlation` (with
NLS covariance and the Marchenko-Pastur band), `seasonality` (hour/weekday with
Benjamini-Hochberg **and** Romano-Wolf), `stationarity` (ADF + KPSS), `cointegration`
(Engle-Granger / Johansen, OU half-life).

**Acceptance:** `seasonality` never reports a survivor without its adjusted p-value; the
multiple-testing correction is not optional.
**Traces:** AGENT-006 §5.2.

### S-3.5 Distributions, analogs, processes — M
`NOT STARTED`

`compare_distributions` (KS, Wasserstein, quantile deltas), `find_analogs` (explicit-vector
kNN), `fit_process`, `simulate_paths` — the last two gain a **`generator_trust_tier`** field
so a GAN/diffusion path can never be reported with the same evidentiary weight as a
bootstrap (the measured failure: standard GANs produce 2000%+ annualised backtests while
passing stylised-fact checks).

**Acceptance:** `simulate_paths` labels every generator; an implausibility filter rejects
paths whose annualised return exceeds a configured bound, and the rejection is reported.
**Traces:** AGENT-006 §5.2, §5.10; research brief Theme 3.

---

## `audit` — the honesty arithmetic

### S-3.6 Deflated Sharpe and minimum track record — M
`NOT STARTED`

Both read `N_eff` from the trial ledger, which already tracks it. The family is fixed at
hypothesis-registration time, never at call time, or the agent can shrink its own
denominator by choosing a scope.

**Files:** `crates/mcp-server/src/tools/audit.rs`, `crates/api/src/routes/`, ledger reads.
**Acceptance:** `deflated_sharpe` refuses to run when the run's trial ticket is not sealed
into a family; `min_track_record` is a **blocking** gate precondition, not an advisory
number.
**Traces:** AGENT-006 §5.4; research brief Theme 1.

### S-3.7 The holdout has a budget, and one door — M
`NOT STARTED`

`check_holdout_budget` and `spend_holdout_look`. The second is the **only** path that may
read holdout rows; every other tool's query scope excludes them. It carries the
`SealedHoldout` envelope, so it suspends for a human on every tier (S-2.12).

**Files:** `crates/mcp-server/src/tools/audit.rs`, `migrations/0079_holdout_budget.sql`,
the data-access layer's scope filter.
**Acceptance:** a holdout partition is unreadable by any tool except `spend_holdout_look`,
asserted adversarially by attempting it from a read tool; an exhausted budget blocks with an
explanation rather than returning data.
**Traces:** AGENT-006 §5.4, §2.6; research brief Theme 1 (adaptive reuse invalidates a
holdout regardless of sample size).

### S-3.8 `metric_integrity` — re-derive what the agent claims — M
`NOT STARTED`

Recomputes the headline metric from platform-logged tool calls (`agent_events`, not the
agent's prose) and diffs against the report. Targets the measured failure mode that current
frontier models game reward specifications when given tool access.

**Acceptance:** a planted mismatch between a logged result and a reported metric is flagged;
the audit reads only platform-side logs, never agent-authored text.
**Traces:** AGENT-006 §5.4; AGENT-009 honesty suite; research brief Theme 9.

### S-3.9 PBO and family-wise significance — M
`NOT STARTED`

`probability_of_overfit` (CSCV; refuses below 10 distinct configs) and `family_wise_test`
(Romano-Wolf / Hansen SPA / White's Reality Check) reusing the null library's existing block
bootstrap, with the block length taken from `describe_returns` diagnostics rather than a
constant.

**Acceptance:** PBO refuses a family with too few configs and says why; the bootstrap block
length is derived, not hard-coded.
**Traces:** AGENT-006 §5.4.

---

## Leakage

### S-3.10 `future_perturbation_test` — M
`NOT STARTED`

Re-run with each signal/indicator shifted or masked and diff against the unperturbed run.
Mechanical: it reads no strategy code. Needs a "run with column X shifted" mode in the
strategy runtime.

**Files:** `crates/strategy-runtime`, `crates/mcp-server/src/tools/audit.rs`.
**Acceptance:** a seeded lookahead is detected and the offending indicator named; a clean
strategy produces no divergence.
**Traces:** AGENT-006 §5.4; research brief Theme 2 (Freqtrade's shipped precedent).

### S-3.11 Bar-boundary and feature leakage — M
`NOT STARTED`

`bar_boundary_check` proves a decision at T could not have seen an unfinished
higher-timeframe candle — the bug class that lives exactly in this platform's continuous
1m→OHLCV aggregation. `feature_leakage_scan` is a static pass in `crates/strategy-validator`
for full-window statistics, future correlation and disguised label transforms.

**Acceptance:** a deliberately leaking resample pair is caught with its first violating
timestamp; the static scan flags a full-dataset normalisation.
**Traces:** AGENT-006 §5.4; research brief Theme 2.

---

## Falsification — the adversarial battery

### S-3.12 Reshuffle, bootstrap, cost shock — M
`NOT STARTED`

`trade_reshuffle_bootstrap` runs **two distinct** modes: reshuffle (path risk of a fixed
outcome set) and bootstrap (estimation uncertainty of the endpoint). Conflating them is a
documented error, so the tool refuses to label reshuffle-only output "worst case".
`cost_shock_curve` sweeps 1×–5× cost and reports the break-even multiple, defaulting to
**measured** venue spreads rather than textbook basis points.

**Acceptance:** the mode is named in every result; a reshuffle result cannot be reported as a
capital requirement; the cost grid's origin is the measured spread.
**Traces:** AGENT-006 §5.5; research brief Theme 3.

### S-3.13 Ablation, fragility, jitter, regime failure — L
`NOT STARTED`

`ablation_sweep` (leave-one-rule-out over the JSON strategy def, deltas must clear the null
library's CI rather than raw point deltas), `parameter_fragility_map` (plateau vs knife-edge,
one scalar index), `timing_jitter_sensitivity` (a collapse at 1-bar jitter is itself a
lookahead signal), `regime_failure_search` (hunt the worst regime, with the same BH /
Romano-Wolf correction used for seasonality). All spend `Falsification` trials.

**Acceptance:** each registers its trial before running; the multiple-comparisons correction
across regimes is applied.
**Traces:** AGENT-006 §5.5, P5; research brief Theme 3.

---

## Execution realism

### S-3.14 Venue constraints, funding, divergence attribution — L
`NOT STARTED`

`get_venue_constraints` is a versioned table with a `verified_at` and a 90-day staleness
flag, carrying the facts verified on 2026-09-14: Coinbase's price-time priority and four
self-trade-prevention modes; Kraken's **decaying** rate-limit points where cancel cost rises
as an order gets younger, which penalises exactly the quote-and-yank pattern a naive
market-making backtest assumes is free; Alpaca's 2026 intraday-margin framework that
replaced the PDT rule. `apply_funding_costs` applies 8-hourly funding and **never silently
interpolates** a gap. `attribute_divergence` decomposes the existing paper-vs-backtest
tracking into signal / execution / timing / data.

**Acceptance:** a requote rate exceeding Kraken's limit is refused before paper testing
validates it; `data_gap_pct > 10%` downgrades confidence rather than filling; divergence
attribution names a dominant component with its evidence.
**Traces:** AGENT-006 §5.6; research brief Theme 5.

### S-3.15 Data provenance, survivorship, funding history — M
`NOT STARTED`

`probe_pit_provenance`, `audit_survivorship`, `get_funding_history`.
**Acceptance:** the provenance check executes a representative as-of query and diffs it
against the latest-value query rather than only inspecting schema; an unauditable universe
reports "unauditable", never a false clean.
**Traces:** AGENT-006 §5.1.

---

## Attribution, models, portfolio

### S-3.16 What the signal actually is — L
`NOT STARTED`

`factor_spanning_test` (needs a **crypto** proxy factor set — BTC beta, funding-rate carry —
because Fama-French does not apply), `signal_half_life` (IC-vs-horizon curve, fitted
half-life, recommended rebalance, cross-checked against `cost_shock_curve`'s break-even
turnover), `alpha_health_check` (self-correlation against the ledger's own alpha pool,
sub-universe consistency, turnover fitness, sign-flip).

**Acceptance:** the factor set is fixed platform-wide per asset class, not agent-selectable
per report; a recommended rebalance faster than the break-even turnover is flagged.
**Traces:** AGENT-006 §5.3; research brief Theme 4.

### S-3.17 Model diagnostics — M
`NOT STARTED`

`detect_drift` (live vs train, PSI/KS/MMD — gates live inference), `conformal_calibrate`
(adaptive conformal or EnbPI, **never** vanilla split-conformal on market data),
`time_aware_importance` (block permutation, not the naive i.i.d. shuffle that produces
out-of-distribution inputs on autocorrelated features).

**Acceptance:** a drift-flagged model's live signals are downgraded rather than silently
continuing; `realized_coverage` is monitored and its drift reported.
**Traces:** AGENT-006 §5.7; research brief Theme 6.

### S-3.18 Portfolio — M
`NOT STARTED`

`shrunk_covariance` (Ledoit-Wolf, **raw always reported beside shrunk**, because shrinkage
biases toward historical structure which is exactly wrong before a correlation breakdown),
`allocate_book` (max single-strategy weight enforced outside the objective so an optimiser
cannot concentrate to hit a target), `check_control_limits` (**read-only** over Phase 7's
tables).

**Acceptance:** `allocate_book` cannot exceed the weight cap by any input; `check_control_limits`
has no write path and no agent-callable halt.
**Traces:** AGENT-006 §5.7, COMP-007 §6.

### S-3.19 Spending guards: call budgets, cost preview, self-status — M
`NOT STARTED`

`spends_trial` bounded simulator runs and nothing bounded anything else — a loop could call
`run_sweep` fifty times and exhaust a dollar budget without moving the trial counter. Add
`max_calls_per_run` per `ToolSpec`, enforced by the harness, exceeded as a typed error naming
the budget and the count.

`estimate_cost(tool, arguments) -> {trials, est_seconds, est_usd, est_rows, confidence}` for
anything that spends a trial or dispatches a job, computed from the job service's existing
`CostInputs`/`CostEstimate` and spending nothing itself. A model that can ask what a call will
cost can plan inside a budget instead of discovering it at the wall, and it is the number the
human sees in the approval card (S-2.16).

`get_self_status()` returns what the agent needs to know about itself and currently cannot
ask: profile and tier, measured or `unmeasured`, decode ladder and canary result, exposure
budget and slice occupancy, budget remaining across steps/tokens/dollars/trials, calibrated
bytes-per-token with sample size, any open circuit breaker, monitor and kill-switch state, and
any `Degradation` this run. An agent that cannot tell it is degraded reports confidently from
a degraded state.

**Files:** `crates/mcp-server/src/tools/{core,research}.rs`, `crates/jobs/src/models.rs`.
**Acceptance:** exceeding `max_calls_per_run` returns a typed error, not a silent refusal;
`estimate_cost` spends nothing and its estimate is within the job service's own error band;
`get_self_status` reports `unmeasured` for a profile with no calibration row.
**Traces:** AGENT-006 §2.4b; AGENT-010 §12.1.

---

## Exit criteria

- The templated tier is real: a tool can be a Python file plus a header, and the
  build-time schema-agreement test proves catalogue and template never drift.
- The `analysis` namespace exists and is the same code the strategy engine and the model
  trainer read (the one-engine rule is enforced in Phase 5).
- Every simulator-running tool registers its trial first; a run of the Phase 8 battery
  raises `N_eff` by exactly the number of simulator runs it made.
- Holdout data is unreachable except through `spend_holdout_look`.
- No namespace exceeds 16 tools; every description is within budget.
