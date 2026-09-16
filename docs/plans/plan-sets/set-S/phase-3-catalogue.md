# Phase 3 — The catalogue

**Completion: 37% (7 / 19 tasks), plus 3 partial. 9 not started.**

> Status 2026-09-16. The **templated tier is real**: a tool is a Python file under
> `apps/skill-runner/templates/<ns>/<name>.py` whose module docstring carries a fenced JSON
> `ToolSpec`, `build.rs` embeds it, and one parser in `crates/skills` serves both the
> catalogue and the runner. The catalogue went from 60 tools to 80; every namespace is inside
> its cap of 16.
>
> Four things a following track must know:
>
> 1. **`crates/mcp-server/tests/templates_run.rs` is deliberately not `#[ignore]`d.** It runs
>    every template twice on a reference dataset and compares `detail` byte for byte. It needs
>    an interpreter with `apps/skill-runner/requirements.txt`; if the default one has the
>    stack in a user directory, `RunnerConfig::stack_path` is how the platform names where its
>    stack is (the runner keeps `PYTHONNOUSERSITE=1`, correctly).
> 2. **statsmodels and `arch` are not in the runner's stack**, and that is ADR-S3-01 rather
>    than an omission. ADF, KPSS, Ljung-Box, ARCH-LM, Engle-Granger, Johansen, GARCH(1,1) and
>    HAR are written out against numpy and scipy, with embedded critical-value tables that
>    name their source.
> 3. **Two acceptance criteria are recorded as unmet rather than approximated**: EDGE
>    (OQ-15) and Hansen SPA / White's Reality Check (ADR-S3-06). Both tools ship with fewer
>    procedures than §5.2 and §5.4 name, and both say so in their own descriptions.
> 4. **The invariants refused three things on the first full run and were right every time.**
>    ADR-S3-09, -10 and -11 record what changed. If you add a tool with an approval envelope,
>    `ledger::audit`'s action list and the pinned set in `every_tool_envelope_matches_the_ledger`
>    both need you.
>
> Remaining: S-3.10, S-3.11's static scan, S-3.12 … S-3.19. Most of what is left needs data
> beyond OHLCV — a family's recorded return series, a venue table, or the simulator — so the
> next increment is a second `SeriesKind` for the materialiser rather than more templates.

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
share forward bars. **The event size is a platform constant, not an argument** (ADR-S3-10):
the first version shipped a `threshold_sigma` knob under a docstring explaining why sweeping
it is the search trials exist to prevent, and `inv23_no_governance_fields_in_tool_schemas`
plus `spec_15_tool_surface_conformance` both refused the schema — correctly, and for exactly
the reason written above them.

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
`PARTIAL` 2026-09-16 — four of the five tools are complete; `estimate_liquidity_cost`
ships three of the four spread estimators §5.2 names.

Done: `apps/skill-runner/templates/analysis/{estimate_liquidity_cost,rolling_correlation,
seasonality,stationarity,cointegration}.py`.

`seasonality` applies **both** corrections and reports the Romano-Wolf one, which is the
stricter: BH assumes independence or positive dependence and hourly buckets of one price
series are neither. `n_raw_significant` is in the summary beside `n_survivors` on purpose —
on the reference series four of 31 buckets are significant before correction and none
survive it, and showing that gap is more convincing than explaining it. A bucket is never
reported without its adjusted p-value.

`stationarity` reports a **joint** verdict over ADF and KPSS, whose nulls are opposites. Two
of the four combinations are honest "cannot tell", and a tool reporting only ADF would turn
"neither test rejects" into "random walk" and "both reject" into "mean-reverting", both
confidently.

`cointegration` runs Engle-Granger in both directions because the test is asymmetric, and
says so when they disagree. It does not search for pairs: testing a named pair is a
hypothesis, scanning a universe for the best p-value is a search and spends trials.

**Missing: EDGE** (Ardia, Guidotti & Kroencke 2024). Written from memory of the method
rather than from the paper, it returned a negative spread variance against a series whose
spread is known exactly — which means the moment conditions were wrong, not that the data
was hard. A cost estimator biased low makes every strategy look profitable and the error is
invisible, so it is absent rather than approximate. **OQ-15**, and the tool's own description
says three estimators, not four.

That defect was found by a new test rather than by inspection:
`the_cost_estimators_recover_the_fixtures_known_spread` checks every estimator against the
fixture's known 60 bps. It also caught a fixture bug — the reference dataset drew a
proportional wick around `max(open, close)`, whose mid is not an estimate of the efficient
price, and Abdi-Ranaldo came out at half the true spread because of it. The fixture now
simulates an intrabar path.

`estimate_liquidity_cost` (EDGE, Roll, Corwin-Schultz, Amihud, sqrt-impact — wraps what
`check_tradeable_frontier` already computes for one horizon), `rolling_correlation` (with
NLS covariance and the Marchenko-Pastur band), `seasonality` (hour/weekday with
Benjamini-Hochberg **and** Romano-Wolf), `stationarity` (ADF + KPSS), `cointegration`
(Engle-Granger / Johansen, OU half-life).

**Acceptance:** `seasonality` never reports a survivor without its adjusted p-value; the
multiple-testing correction is not optional.
**Traces:** AGENT-006 §5.2.

### S-3.5 Distributions, analogs, processes — M
`DONE` 2026-09-16 — `apps/skill-runner/templates/analysis/{compare_distributions,find_analogs,
fit_process,simulate_paths}.py`. The `analysis` namespace now holds 13 tools, under the cap
of 16.

`simulate_paths` labels every generator with a trust tier — `resampled_history`,
`resampled_history_iid`, `parametric`, `parametric_naive` — and only the first is
`admissible_as_evidence`. The implausibility filter rejects paths above a configurable
annualised bound and **counts** them; a generator producing more than 1% of them is
mis-calibrated, and the count is the only signal of that, which is exactly what the GAN
failure looked like from outside. Verified on the reference series: the block bootstrap
keeps 200 of 200 and reproduces 4/4 stylised facts; the Gaussian reference rejects 2% and
reproduces 2/4.

**There is no GAN or diffusion generator**, deliberately. The measured failure is that they
produce 2000% annualised backtests *while passing stylised-fact checks* — so a stylised-fact
check is not a sufficient gate, and until one exists a `generated_synthetic` tier would be a
label on something that should not be produced. §5.10 names the tier; nothing here claims it.

`find_analogs` uses six explicitly named, scale-free features rather than a learned
embedding, because the output is a base rate a human acts on and a base rate whose reference
class cannot be stated is not a base rate. Analogs are separated by at least one window
length — neighbouring windows overlap almost completely, so the k nearest would otherwise be
one episode counted k times — and the median distance between two *random* windows is in the
summary, because with enough history the k nearest of anything are close by construction.

A bug this found: the stylised-fact check compared the generated series' return
autocorrelation against zero. The reference fixture's 60 bps bid-ask bounce gives it a real
lag-1 autocorrelation of -0.31, so the block bootstrap was marked unfaithful precisely for
faithfully reproducing the microstructure. The check is now against the source.

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
`DONE` 2026-09-16 — `crates/mcp-server/src/tools/audit.rs`,
`crates/api/src/routes/agent_audit.rs`, `SuiteManager::{audit_deflated_sharpe,audit_track_record}`,
`backtest::stats::min_track_record_length`.

`deflated_sharpe` **reads the number Gate 3 recorded** rather than computing a second one, and
refuses when the experiment has not reached Gate 3. That is the acceptance criterion
implemented literally: a read tool that recomputed the statistic would need a trial count from
somewhere, and the only somewhere available to it is the caller — a Sharpe discounted by a
denominator the selector supplies is not discounted. `N_eff` stays the ledger's, which has no
public constructor.

`min_track_record` is computed on demand, because nothing in it depends on how many things
were tried — which is what makes it usable *before* a funnel, where it answers "is this worth
testing". It is blocking: `blocks_promotion` carries the profile floor (`strict_v1` five
years, `paper_v1` none), and the answer for a Sharpe at or below zero is **no number at all**
rather than a very large one, because no record of any length establishes a Sharpe the
strategy does not have and a large number reads as "keep going".

Two things `min_track_record_length` does that a textbook version does not, both tested:
it works in per-observation units with observations-per-year taken from the equity curve's own
spacing (an hourly run scored as daily looks ~35x more established than it is), and negative
skew and fat tails lengthen the requirement rather than being ignored — a version that dropped
the higher moments clears exactly the strategies that sell tail risk.

Both read `N_eff` from the trial ledger, which already tracks it. The family is fixed at
hypothesis-registration time, never at call time, or the agent can shrink its own
denominator by choosing a scope.

**Files:** `crates/mcp-server/src/tools/audit.rs`, `crates/api/src/routes/`, ledger reads.
**Acceptance:** `deflated_sharpe` refuses to run when the run's trial ticket is not sealed
into a family; `min_track_record` is a **blocking** gate precondition, not an advisory
number.
**Traces:** AGENT-006 §5.4; research brief Theme 1.

### S-3.7 The holdout has a budget, and one door — M
`DONE` 2026-09-16 — `check_holdout_budget` and `spend_holdout_look` in
`crates/mcp-server/src/tools/audit.rs`, served by `crates/api/src/routes/agent_audit.rs`.

**No migration was needed and none was written.** The plan called for
`0079_holdout_budget.sql`; the experiment vault in `0028_backtest_experiments.sql` already
is this mechanism — one logged access, self-sealing, a trial counter with no decrement path
and an append-only access log, both enforced by database triggers rather than by a caller. A
second holdout implementation for the agent would be a holdout that has been looked at twice,
and the second look is the one nobody counts. (Migration 0079 is also long since taken; the
tree is at 0121.)

`spend_holdout_look` carries `Envelope::SealedHoldout` so it suspends for a human on every
tier, and is capped at `max_calls_per_run: 1` independently of that — the cap is the
arithmetic refusing a second attempt inside one run even where a human would have said yes
again. `check_holdout_budget` is free, deliberately: an agent that can only discover the
budget by exhausting it will exhaust it.

Both orderings are checked **ahead** of the spend, so a look that was going to be refused does
not consume the look: Gate 3 must have passed (a holdout reachable before the significance
gate is one a candidate can be tuned against) and the experiment must not be unsafe. The
`reason` needs 20 characters because it is written into a log that outlives the run and is the
only record of why the one look was used.

Verified adversarially by `tools::audit::tests::the_holdout_has_exactly_one_door`, which scans
the **whole catalogue** for `Envelope::SealedHoldout` and fails if any second tool carries it —
a test against this module alone would pass while another module opened a second door.

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
`DONE` 2026-09-16 — `crates/mcp-server/src/tools/audit.rs`,
`crates/api/src/routes/agent_audit.rs`, `SuiteManager::audit_metric`.

The acceptance criterion is structural rather than tested-for: `claimed_value` arrives as a
separate argument and is **only ever compared**. The recomputation's inputs are the run's
recorded `RunResult` and nothing else — no prose, no summary, no number the agent passed in
as "the previous value".

Two things it does that a naive diff does not:

* the comparison is **relative**, because a Sharpe of 0.001 reported as 0.01 is a tenfold
  error that an absolute tolerance waves through;
* it checks whether the claim matches a *different* metric of the same run, and says so. The
  right number under the wrong label is the common honest mistake, and from outside it is
  indistinguishable from the dishonest case until something checks.

**Deviation from the plan's wording, recorded:** the plan says to read `agent_events`. That
table is migration 0036's UI event stream — the same name collision phase 2 hit when it had to
rename the effect log to `agent_effect_log`. The metric is re-derived from the run store,
which is the platform-side record the plan's phrase means.

The metric name is a closed set of nine and anything outside it is refused, because a metric
nothing can re-derive is exactly the kind this tool exists to catch.

Recomputes the headline metric from platform-logged tool calls (`agent_events`, not the
agent's prose) and diffs against the report. Targets the measured failure mode that current
frontier models game reward specifications when given tool access.

**Acceptance:** a planted mismatch between a logged result and a reported metric is flagged;
the audit reads only platform-side logs, never agent-authored text.
**Traces:** AGENT-006 §5.4; AGENT-009 honesty suite; research brief Theme 9.

### S-3.9 PBO and family-wise significance — M
`PARTIAL` 2026-09-16 — both tools exist; `family_wise_test` offers **one** family-wise
procedure rather than the three §5.4 names.

`crates/mcp-server/src/tools/audit.rs`, `crates/api/src/routes/agent_audit.rs`,
`SuiteManager::{audit_probability_of_overfit,audit_family_wise}`.

Neither tool computes a new statistic: `probability_of_overfit` runs the existing
`stats::probability_of_backtest_overfitting` over runs the experiment has **already recorded**,
and `family_wise_test` runs the existing `stats::bootstrap::romano_wolf`. Nothing is
dispatched, so neither spends a trial — a statistic computed from an existing look is not a
new look.

Both acceptance criteria hold. PBO refuses below ten distinct configurations and says why:
below ten there are too few possible rankings for the statistic to take a value, so it is not
a small sample but a number that does not exist yet. The bootstrap block length is
Politis-White over the family's *pooled* autocorrelation — derived, and pooled so one
unusually smooth candidate cannot set the block for everyone.

**Missing: Hansen's SPA and White's Reality Check.** This is a deliberate deviation, not an
omission — recorded as **ADR-S3-06**. Romano-Wolf dominates both (per-candidate adjusted
p-values rather than only a test of the global null), and offering three family-wise
procedures behind one question is a menu to shop between, which is the error the tool exists
to correct, one level up. The same reasoning is why `seed` defaults to 0: re-rolling a
bootstrap until a candidate survives is that error again.

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
`PARTIAL` 2026-09-16 — `bar_boundary_check` is built and tested against a planted defect;
`feature_leakage_scan` (the static pass in `crates/strategy-validator`) is **deferred to
phase 5, with a reason**.

The reason is an ordering fact, checked rather than assumed. The frozen expression grammar
admits exactly two calls — `crates/strategy-validator/src/expressions.rs::check_call` accepts
`feature(name)` and `bar(open|high|low|close|volume)` and rejects everything else — and the
feature vocabulary a backtest can actually evaluate is `ema_N` and `rsi_N`, both causal. A
scan for full-window statistics, future correlation and disguised label transforms therefore
has nothing expressible to find: the constructs it looks for cannot be written today.

Building it now would produce a pass that returns clean on every strategy the platform can
represent, which is worse than not having it — a leakage scan that has never found anything
reads as evidence there is nothing to find. It belongs with AGENT-011's SLv2, which is what
introduces expressions rich enough to hide a leak (phase 5).

`apps/skill-runner/templates/audit/bar_boundary_check.py`. It compares two stored resolutions
of one instrument and asks whether every aggregate is reconstructible from *completed*
constituents — so it detects the defect for every strategy at once rather than one at a time,
and reads no strategy code. A strategy that passes a code review while reading a mis-aggregated
series is exactly what a code review cannot catch.

The fixture now carries a timeframe pair: `fast.parquet` (1m), `slow.parquet` (its exact 1h
aggregate) and `slow_leaky.parquet`, where a quarter of the hours were written twenty minutes
before their last minute landed — the way the platform's continuous aggregation actually
produces the bug, rather than as noise any check would catch.

Acceptance met exactly: the planted leak is caught **with its first violating timestamp**
(`2024-01-01T10:00:00Z`), and the tool recovers *how far early* the row was readable — 20
bars, which is the planted value, asserted.

The clean half of that test is the half that earned its place. The first version of this
template reported **720 of 720** bars violating on a fixture built to contain none: a parquet
round trip had handed it `datetime64[us]` and it divided by 1e9 to get seconds, landing every
timestamp in January 1970 and collapsing every bucket into one. A tool for detecting timestamp
defects had one. `preamble.py::epoch_seconds` now does this resolution-independently, and the
docstring records why.

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
