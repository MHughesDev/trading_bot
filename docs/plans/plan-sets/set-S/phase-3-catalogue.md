# Phase 3 — The catalogue

**Completion: 42% (8 / 19 tasks), plus 10 `PARTIAL` and 1 `NOT STARTED`.**

> **2026-09-27.** S-3.18 closed. S-3.10, S-3.13 and S-3.17 moved
> `BLOCKED:UPSTREAM` → `PARTIAL`: the perturbation runtime mode now exists, the
> return-series route is served, `by_regime` is measured, and one of S-3.17's three
> tools was already on disk behind the blocked marker. **No `BLOCKED:UPSTREAM`
> remains in this phase** — what is left is tools to write and two producers to
> build (a funding read route, shared by S-3.14 and S-3.15; and `SeriesKind::Signal`
> for S-3.16).

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
> **Remaining: S-3.10, S-3.12, S-3.13, S-3.15, S-3.17 — and every one of them is blocked on
> deployment state rather than on effort.** Checked, not assumed, on 2026-09-16:
>
> * the canonical PIT store **is not deployed on the development box**. ClickHouse is up and
>   holds `market_bars` and `market_bars_v2`; `market_bar` — the table
>   `clickhouse/07_canonical_market.sql` creates and the one `BarStore` reads — does not
>   exist. So `probe_pit_provenance` (S-3.15), whose whole value is executing a real as-of
>   query and diffing it against a later one, cannot be exercised against anything here;
> * `future_perturbation_test` (S-3.10) needs a "run with column X shifted" mode in the
>   strategy runtime, which does not exist;
> * `cost_shock_curve` and the S-3.13 battery re-run the simulator, so they are gated on the
>   executor rather than on statistics;
> * S-3.17's model diagnostics need model predictions, and there is no prediction
>   materialisation — only bars and trial returns.
>
> Writing them anyway would mean shipping database and simulator code that no test on this
> machine can run. The fixtures built in this phase caught four real defects that would
> otherwise have shipped — two wrong spread estimators, a double-integrated Hurst exponent,
> and a timestamp bug inside the tool written to detect timestamp bugs — and none of those
> would have been caught by code that compiles. The remainder waits for an environment that
> can run it.

> **This note was wrong, and it was wrong in the most expensive direction.** It
> said `/api/evidence/trades` *"already returns the full raw trade array … for
> any run the caller owns, and nobody had noticed"*, and three tools were built
> on top of it. **The route has never returned a trade to anybody** — its query
> names columns that migration `0026` took away, and the error is swallowed into
> a 404. See the note below. The lesson is not about this route: the claim was
> made by reading the handler and believing it, and it survived because reading a
> handler cannot tell you whether its SQL matches the deployed schema. Q-S3-04. Arithmetic over a response is allowed here (`compare_backtests`
> already does it); arithmetic a **gate** also performs is not, for the reason
> `tools/audit.rs`'s module docs give.

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

### The four that are left are blocked, not unstarted, 2026-09-21 — and the route under three finished ones does not run

A track went after the remaining catalogue tools and **shipped none**. That is
the right outcome, and what it found instead is worth more than four tools.

**`/api/evidence/trades` has never returned a trade to anybody.** It runs
`SELECT result FROM backtest_runs WHERE id = $1 AND user_id = $2`. Migration
`0026` renamed the old job tracker to **`backtest_jobs`** and created a
different `backtest_runs` — `run_id TEXT PRIMARY KEY, status, config_json,
result_json, …` with **no `id`, no `user_id`, no `result`**. The query errors;
`let Ok(Some(row)) = row else { … }` swallows the error into a
`404 no_such_run`. The failure has always read as *"that run doesn't exist"*.

**It is wrong twice.** Pointed at the right table, the document still has no
trades: a completed job's `result` is the market_simulator run document
(`summary`, `stats_pnls`, `stats_returns`, `total_orders`, …) and carries no
`trades` array at all. The `Trade` struct with the eight fields the phase note
promised lives in `backtest::run::result::RunResult`, which lands in
`backtest_runs.result_json` — **the table with zero rows that no route reads.**

And the second failure mode is silent rather than loud: the route emits
`"trades": []` when the key is absent, so `path_risk::fetch_trades` cannot tell
*"no trade array in this document"* from *"this run took no trades"* — and would
print its own sentence, **"An empty trade list is not a result with zero risk;
it is a run that did not trade"**, about a run that placed 58 orders. An absent
measurement reported as a zero, in the file written to forbid exactly that.

**So the four remaining tasks are `BLOCKED:UPSTREAM`, not `NOT STARTED`.** Each
names the exact symbol that must exist first, in its own entry. The short
version:

- ~~**Two ClickHouse schema files were never applied on this deployment.**~~
  **This was wrong, and it was checked against the wrong database.** Both files
  are applied. Queried directly on 2026-09-21 against
  `trading_bot-clickhouse-1`, database `trading`: **`market_bar` holds 170,594
  rows**, `instrument_symbol_dim` 10, `venue_dim` 11, `restatement_index` 0,
  and `funding_rates`, `open_interest` and `option_chain_snapshots` all exist at
  0 rows. The earlier track looked in the `default` database, where they are
  genuinely absent — and this box has fourteen ClickHouse databases, most of
  them test fixtures. **Two tracks agreeing is not the same as two tracks being
  right**; a third checked and disagreed, which is why its report opens by
  saying which claims it refuted.

  What is actually missing is narrower and more useful: `get_funding_history`
  needs a **reader** (`storage::clickhouse::forward` exposes `write_funding`,
  `write_open_interest`, `write_chain_snapshot`, `beat` and no reader at all)
  and a route, plus rows in a table that has none. `probe_pit_provenance` needs
  a route over `BarStore::as_of` returning the as-of/latest diff — and it has
  170,594 real bars to run against, so it is **not** blocked on deployment
  state.
- ~~**Nothing in the workspace mentions `shift_bars`, `shift_column`,
  `mask_column`, `jitter_bars`, `fill_delay_ms` or `perturbation_pct`.** The
  runtime mode §5.4 assumes does not exist.~~ **Closed 2026-09-27** for all but
  `fill_delay_ms`, which is refused by name because the preview produces no fills:
  see S-3.10.
- ~~**No route returns a run's return series as JSON.**~~ **Closed 2026-09-27**:
  `GET /api/backtest/experiments/{id}/returns` serves it, timestamps included.
  Until then `SuiteManager::recorded_returns` was reachable only as a parquet file
  through the template materialiser, so `ablation_sweep` and `parameter_fragility_map`
  have nothing to diff.
- **No membership calendar exists anywhere** — zero hits across the workspace.
  `instruments` has four rows, all `active = true`, with a current-state boolean
  and no `delisted_at`. `DataSlice.universe_ref` is documented as *"pinned by
  membership-calendar version"* against a calendar nobody built, and
  `audit_survivorship` has nothing to audit.

**Two more things the live system said that the plan did not expect:**

- **~~All eleven completed backtest jobs have null metrics.~~ Corrected
  2026-09-23: this was measured from an incompletely-checked snapshot and is
  false.** Re-queried live: **3 of the 11** completed `backtest_jobs` rows carry
  real, non-null `Sharpe Ratio (252 days)` values (`7edd16d0…` = 9.79,
  `09e044e0…` = -0.62, `76d8150a…` = -7.62); the other 8 are null. A fragility
  index over the null 8 would still be dividing nulls, which is why the
  conclusion drawn from this survived — but the sweeping form of the claim did
  not, and this is the **measured-from-residue** pattern this programme has
  already been caught by twice. A count asserted over "every one" needs every
  one actually counted.
- **`by_regime` is built, tested, and always empty.**
  `backtest::stats::diagnostics::by_regime` works and has a passing test; its
  only non-test caller, `crates/api/src/research/mod.rs:316`, passes `&[]`.
  **Every `DiagnosticBundle` this platform has ever served had an empty
  `by_regime`.** Built-but-unreached again, this time one argument wide.

Two questions opened, neither implemented: **Q-S3-03** (§5.4 names column
perturbation and data truncation as one tool) and **Q-S3-04** (nothing anywhere
checks a route's SQL against the schema).


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
`payload.data_path` — a skill has **no network and no database**
(`crates/skills/src/template.rs:374` refuses any template declaring otherwise; the
citation here read `taxonomy.rs:285` until 2026-09-23, which moved when S-1.3 made
`taxonomy.rs` a derived table) so the data must arrive as a file. Limits default to the runner's 30 s / 512 MB / 4 MB;
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
`PARTIAL` 2026-09-27 (was `BLOCKED:UPSTREAM`) — **the runtime mode exists now.**
`crates/backtest/src/perturb.rs` carries `ShiftColumn`, `MaskColumn` and
`JitterColumn` with validation, and `backtest::preview::preview_signals_perturbed`
applies them; `preview_signals` keeps its signature and delegates with an empty set.

**This task's own prescription was wrong and was not followed.** It asked for "a
perturbation argument on `features::runtime::backfill_column`". That function is a
three-line wrapper over `value_at` and a `shift` parameter inside it would hand
**every feature implementation** the ability to read forward — which is the one
structural guarantee `crates/features` exists to provide, and the guarantee
`crates/mcp-server/src/tools/strategy.rs:546` reasons from. The perturbation is
applied to the produced column in `crates/backtest` instead, and `crates/features`
needs no change at all.

That substitution rests on `crates/features/tests/features_versioned.rs::
backfill_and_incremental_serving_agree` (INV-14), which asserts the batch column
equals a growing window's value bit for bit at every index — so at zero shift the
perturbed path is a no-op and the perturbation is the only difference between two
runs. **That test is now load-bearing for a second caller and must not be
relaxed.**

**What remains, and neither is in `crates/backtest`:** a `perturbations` field on
`POST /api/agent/strategy/preview` (shape recorded in the working notes), and
`future_perturbation_test` itself in `crates/mcp-server/src/tools/audit.rs` — two
preview calls and a diff over entry bars, which must refuse to report "no
lookahead" when the perturbed run's `never_fires_code` is
`perturbed_column_has_no_value`. `fill_delay_ms` is refused outright by the
perturbation set: the preview produces no fills, so there is nothing for a delay to
apply to. That half is S-3.13's `timing_jitter_sensitivity` and stays blocked.

**Q-S3-03 is answered: two tools, not one** (§5.4 fuses column perturbation with
data truncation). Four reasons, strongest first. Their null results mean different
things — perturbation's no-divergence is evidence about *causality*, truncation's
is evidence about *sample dependence*, and a compound verdict collapses to "clean"
in a reader's head. They sit on opposite sides of the trial boundary: truncation
changes the `DataSlice`, which is part of the pre-registered subject and must clear
the holdout guard, while perturbation changes nothing the ledger knows about and
spends nothing — one tool would need two incompatible trial-accounting rules.
Only perturbation has an indicator to blame, and naming the offending indicator is
half of this task's acceptance. And their failure modes collide: a truncation short
enough to bite the warm-up and a masked column both present as "the strategy
stopped firing". Truncation is not built, and belongs near `DataSlice` rather than
near the feature engine.

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
`PARTIAL` — **downgraded from `DONE` on 2026-09-21.** All three tools are built and
35 tests pass over fixtures. **None of them can execute.** The marker said `DONE`
because the tools were finished; nobody had asked whether the route under them
answered.

**Re-audited 2026-09-23: still correctly `PARTIAL`, but the reason above is now
stale and the real blocker is narrower.** The route half was fixed two days
after this note was written: commit `f8c097f` (2026-09-21, "five outcomes where
there were two") rewrote `crates/api/src/routes/evidence.rs` to read
`backtest_jobs` rather than the table `0026` renamed away, with five distinct
non-swallowing outcomes including an honest `trades_not_recorded` 422. So "whose
query names a table that migration 0026 renamed away" describes a version that
no longer exists. What remains is the deeper half, and it is not a route bug at
all: the simulator's own `BacktestResult` (`crates/backtest/src/result.rs`)
carries `total_orders`, `stats_pnls`, `stats_returns` and `summary` and **has no
`trades` field at any point**, so there is nothing for a fixed route to serve.
Confirmed live: all 11 completed `backtest_jobs` rows report
`has_trades = false`. These tools stay unusable against real data until the
simulator records per-trade rows — a producer gap, not a query gap.

`trade_reshuffle_bootstrap` runs **two distinct** modes: reshuffle (path risk of a fixed
outcome set) and bootstrap (estimation uncertainty of the endpoint). Conflating them is a
documented error, so the tool refuses to label reshuffle-only output "worst case".
`cost_shock_curve` sweeps 1×–5× cost and reports the break-even multiple, defaulting to
**measured** venue spreads rather than textbook basis points.

**Acceptance:** the mode is named in every result; a reshuffle result cannot be reported as a
capital requirement; the cost grid's origin is the measured spread.
**Traces:** AGENT-006 §5.5; research brief Theme 3.

**Built 2026-09-21 — and the reason it was not built sooner is a lesson about
this file.** The phase header said *"Remaining: S-3.10, S-3.12, S-3.13, S-3.15,
S-3.17 — and every one of them is blocked on deployment state rather than on
effort."* It then gave a reason for four of them and **none at all for
`trade_reshuffle_bootstrap`**, the one tool in the list that is pure arithmetic
over stored results. "Every one of them" was true by omission, not by check.

`crates/mcp-server/src/tools/path_risk.rs`: both tools, typed, over the
**already-existing** `/api/evidence/trades` route. No route, no migration, no
dependency. 35 tests.

`Trade` carries `costs_paid` **and** `pnl` per trade, so `pnl(m) = pnl +
costs_paid − m·costs_paid` is exact and the break-even multiple is
`gross / total_costs` — no simulator re-run, which is what §5.5's `none` trial for
`cost_shock_curve` presupposes. The plan's claim that it "re-runs the simulator,
so it is gated on the executor" was wrong; that claim is right for S-3.13 only.

**Both declare `TrialKind::None` explicitly rather than by default**, so the
claim reads as a claim. Neither dispatches anything: the look they re-read was
charged when the run ran (INV-18). Charging a falsification trial for re-reading
a stored array would raise `N_eff` against work nobody did, and every
selection-adjusted statistic divides by that count.

**What they refuse, each with a test:** `mode` has no default — refusing to
choose between path risk and estimation uncertainty is the tool's whole point;
`endpoint_dispersion` comes back `null` **with a reason** and a `not_a_worst_case`
field, never a zero; `block` mode is refused **by name**, naming
`backtest::stats::bootstrap::politis_white_block` as the symbol it would need,
because a second Politis–White would disagree with the platform's silently; any
trade missing `costs_paid` refuses the whole curve, because a missing cost read
as zero gives an infinite break-even multiple — the most reassuring wrong answer
available; and a run charged zero costs is refused, pointing at `run_oracle_test`.
A static source scan fails the build if anyone adds a `const …BPS` fallback.

**Two spec problems found and neither implemented:** §5.5 assigns both tools the
templated tier, which cannot carry per-trade rows (Q-S3-02); and the spec and this
plan disagree about whether `regime_failure_search` spends a trial (Q-S3-01).

### S-3.13 Ablation, fragility, jitter, regime failure — L
`PARTIAL` 2026-09-27 (was `BLOCKED:UPSTREAM`) — **three of the four blockers are
closed; the tools are the remaining work.** Per tool:

- **`ablation_sweep`** — its data blocker is gone:
  `GET /api/backtest/experiments/{id}/returns` now serves the recorded family as
  JSON, timestamps included. But it still needs a **dispatcher, not a reader**:
  leave-one-rule-out means constructing and running N variants, and that route
  serves only already-recorded ones. It also opens a question worth settling before
  code: this task says all four tools spend `Falsification` trials, but a tool that
  only re-reads recorded returns should declare `TrialKind::None` for the reason
  `path_risk` does — charging a trial for a re-read raises `N_eff` against work
  nobody did, and it moves the tool out of ring 0 into a mode that may not reach it.
- **`parameter_fragility_map`** — **should be re-scoped rather than built.** It is
  substantially already there under another name: `crates/research/src/surface.rs`
  carries `plateau`, `cliffs` ("bin edges across which the mean drops sharply") and
  `sensitivity` per parameter, `SweepReport.neighbourhood_plateau` is already a
  plateau-versus-knife-edge verdict with a stated rule, and `GET
  /api/research/sweeps/{id}` already serves the whole thing to the agent. A tool
  that recomputed this from the returns route would be a second answer to
  arithmetic the platform already performs, which `tools/audit.rs`'s own module
  docs forbid. What is genuinely missing is only the single scalar index and the
  CI comparison.
- **`timing_jitter_sensitivity`** — still blocked, and now for one reason rather
  than two. S-3.10's runtime mode exists, but the preview produces no fills, so
  `fill_delay_ms` is refused by name. This needs the simulator's order path.
- **`regime_failure_search`** — its "one line" is closed.
  `crates/api/src/research/mod.rs` now resolves a market scope from the run's
  experiment and supplies real labelled windows, so `by_regime` is measured rather
  than `Unmeasured { NoLabelsSupplied }`. Every failure along that path still
  yields no windows rather than a guess. The tool itself is `crates/mcp-server`
  work and is not written.

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
`PARTIAL` 2026-09-16 — `get_venue_constraints` is built with the staleness mechanism and the
qualitative rules; its numeric limits are **OQ-17**. `apply_funding_costs` and
`attribute_divergence` are not started.

`crates/mcp-server/src/tools/venues.rs`, answered locally rather than over HTTP — static
reference data with no tenant and no secret, the same exception `list_lanes` takes from
ADR-0010. Updating a fact is therefore a code review and a deploy, which is the right friction
for something whose whole value is that somebody checked it.

The durable half is the **mechanism**: every fact carries `verified_at`, the tool computes its
own age, and past 90 days the answer says `STALE` in the *summary* — where a flat-tier model
that never opens `detail` still sees it.

The qualitative rules are the ones from the brief, and the most useful is Kraken's: the order
rate limit decays and cancelling a younger order costs more, which penalises exactly the
post-and-pull pattern a naive market-making backtest assumes is free. That strategy backtests
perfectly and throttles itself live.

**Every numeric limit is flagged `verified: false`**, and
`no_numeric_limit_claims_to_be_verified` asserts it across the whole table so a later edit
cannot quietly promote a guess. This repository has no verified source for Kraken's counter
maximum, its decay rate, the penalty schedule by order age, or the four Coinbase STP mode
identifiers — so §5.6's "a requote rate exceeding Kraken's limit is refused" cannot be met
honestly. A threshold guessed high lets a strategy be throttled live having passed the check;
one guessed low refuses a strategy that works. Both read like a verified threshold, which is
what makes the guess worse than the stated gap.

Namespace note: `execution`, not `market`. `market` is not in §2.1's canonical list and the
catalogue test caught it; `apply_funding_costs` and `attribute_divergence` belong in
`execution` too.

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
`NOT STARTED` — **corrected 2026-09-21: this was marked `BLOCKED:UPSTREAM` on a
claim that turned out to be false.** Both ClickHouse schema files **are** applied in database `trading`:
`market_bar` holds **170,594 rows**, `instrument_symbol_dim` 10, `venue_dim` 11,
and `funding_rates` / `open_interest` / `option_chain_snapshots` exist at 0
rows. So `probe_pit_provenance` is buildable today against real bars — it needs
a route over `BarStore::as_of` that returns the as-of/latest **diff** rather
than only bars. `get_funding_history` needs a `read_funding` in
`storage::clickhouse::forward`, a route, and rows in a table that has none —
collection is wired (`KrakenFuturesClient::funding_history` → `write_funding`)
and has never run here.

**Re-checked 2026-09-23: the "exposes writers and no reader at all" half is now
stale, and what replaced it is this file's own named defect shape.** Commit
`831b925` (2026-09-22) added `read_funding`, `read_open_interest` and
`funding_at` to `storage::clickhouse::forward` — one day after the claim above
was written, and incidentally: that commit's actual subject is agent
pause/resume and `Spend` typing, so these readers arrived as passengers. They
have **zero callers anywhere outside their own file** — no MCP tool, no route,
no execution-side consumer. So the reader exists and the task is no closer to
done: `get_funding_history` still needs the route and the rows. This marker stays
where it is, correctly, and the missing piece is one item shorter than the note
said.

`audit_survivorship` **is** genuinely blocked: **no membership calendar exists
anywhere** in the workspace, `instruments` has four rows all `active = true`
with no `delisted_at`, and `DataSlice.universe_ref` is documented as pinned by a
calendar version that was never built. One of three sub-tools blocked is not a
blocked task, which is why this marker came back.

`probe_pit_provenance`, `audit_survivorship`, `get_funding_history`.
**Acceptance:** the provenance check executes a representative as-of query and diffs it
against the latest-value query rather than only inspecting schema; an unauditable universe
reports "unauditable", never a false clean.
**Traces:** AGENT-006 §5.1.

---

## Attribution, models, portfolio

### S-3.16 What the signal actually is — L
`PARTIAL` 2026-09-16 — `alpha_health_check` is built;
`factor_spanning_test` and `signal_half_life` are not, and both are blocked on data that
does not exist yet rather than on effort.

`apps/skill-runner/templates/attribution/alpha_health_check.py` checks three ways an alpha is
worthless that a Sharpe ratio cannot show: **redundant** with a sibling (0.8+ correlated, so
a book holding both is levered into one idea it thinks is two), **sign-unstable** between
halves (a full-sample Sharpe averages the two and says nothing about which half the future
resembles), and **concentrated** (most of the return from 5% of periods — an event, not an
edge). Checked separately rather than folded into a score, because a single number lets a
strategy failing one badly pass on the other two.

**Scope is narrower than §5.3 asks**, and says so in its own docstring: it compares against
the experiment's own members, not the platform's whole alpha pool. That catches the common
case — a sweep promoting three points off one plateau as three alphas — and the pool-wide
version needs a tenant-scoped materialisation that does not exist.

**Missing, with reasons:** `signal_half_life` needs an IC-vs-horizon curve over a *signal*
series, and there is no signal materialisation — only bars and trial returns.
`factor_spanning_test` needs the crypto proxy factors §5.3 names, and one of the two is
funding-rate carry, whose history is `get_funding_history` in S-3.15 and is not built. Doing
BTC-beta alone and calling it a spanning test would be reporting a one-factor result under a
two-factor name.

This task is also what produced the family's **timestamps**. Trial returns were
`(run_id, period, ret)`, and an integer index cannot be aligned to anything outside itself —
no factor series, no regime label, no second family. The materialiser now carries `ts` in
nanoseconds, matching the bar materialiser's column, so a template joins the two without
converting between clocks.

`factor_spanning_test` (needs a **crypto** proxy factor set — BTC beta, funding-rate carry —
because Fama-French does not apply), `signal_half_life` (IC-vs-horizon curve, fitted
half-life, recommended rebalance, cross-checked against `cost_shock_curve`'s break-even
turnover), `alpha_health_check` (self-correlation against the ledger's own alpha pool,
sub-universe consistency, turnover fitness, sign-flip).

**Acceptance:** the factor set is fixed platform-wide per asset class, not agent-selectable
per report; a recommended rebalance faster than the break-even turnover is flagged.
**Traces:** AGENT-006 §5.3; research brief Theme 4.

### S-3.17 Model diagnostics — M
`PARTIAL` 2026-09-27 (was `BLOCKED:UPSTREAM`) — **one of the three is built, and the
blocked marker was hiding it.** `apps/skill-runner/templates/model/detect_drift.py` is on
disk and in the catalogue (`every_template_on_disk_reached_the_catalogue` would fail
otherwise). `conformal_calibrate` and `time_aware_importance` are genuinely
unstarted.

The scoring blocker is real and unchanged for the half that needs a label:
`model_predictions` exists, is empty, and carries no realised outcome (`direction`,
`magnitude_str`, `confidence`, `horizon` only), so a drift detector that reached it
would have nothing to score against. `skills::SeriesKind` has `Ohlcv` and
`TrialReturns` and no prediction kind. AGENT-006 §5.7 puts all three tools at tier
**P** (templated), so they belong in `apps/skill-runner/templates/model/*.py`, not
in `crates/mcp-server` — which is where `detect_drift` correctly is.

Marked `PARTIAL` rather than `BLOCKED:UPSTREAM` because a marker that says nothing
can proceed, over a tool that shipped, is the kind of claim this file has been
corrected for three times.

`detect_drift` (live vs train, PSI/KS/MMD — gates live inference), `conformal_calibrate`
(adaptive conformal or EnbPI, **never** vanilla split-conformal on market data),
`time_aware_importance` (block permutation, not the naive i.i.d. shuffle that produces
out-of-distribution inputs on autocorrelated features).

**Acceptance:** a drift-flagged model's live signals are downgraded rather than silently
continuing; `realized_coverage` is monitored and its drift reported.
**Traces:** AGENT-006 §5.7; research brief Theme 6.

### S-3.18 Portfolio — M
`DONE` 2026-09-27 (was `PARTIAL` 2026-09-16) — `shrunk_covariance` and `allocate_book`
were built and tested; `check_control_limits` now exists
(`crates/mcp-server/src/tools/control_limits.rs`) and both acceptance clauses hold.

**"Waits on phase 7's tables" was stale.** Migrations 0074–0078 are all in the tree,
and four agent-readable monitor reads already existed, so the tool composes those —
**no new route was needed and none was written.**

The "no write path and no agent-callable halt" clause is closed by a guard rather
than by inspection: `no_tool_anywhere_posts_to_the_monitor` scans the whole
`src/tools/` tree for a mutating verb near an `/api/monitor` path and separately
pins `ApiClient`'s verb set, so a new verb fails the test rather than slipping past
it. Its first version keyed on the path and failed on its own prose — keying on the
verb is what lets a tool *say* "a human resets it with POST …" without tripping.

Three silences report as themselves rather than as "within limits", which is the
failure this tool was most exposed to: `unwatched` (the lease is not valid, so
nothing has looked), `unmonitored` (no declared limits, or the monitor has never
seen this strategy) and `state_and_events_disagree` (both reported, neither
corrected). Nothing re-derives a breach — `ControlLimit::breached_by` treats a `NaN`
reading as a breach, and a second `>` here would read the same failed measurement
as a pass.

**One recorded deviation from §5.7:** `window=30d|60d|90d` is not accepted, because
`GET /api/monitor/events` serves no `occurred_at` and a window that cannot be
applied should not be in a schema. The payload says `time_bounded: false` with the
reason, and `events_truncated` separates "no events" from "older than the most
recent thousand". Adding `occurred_at` and a `scope` filter to that route is the
one-field change that would close it.

This task needed a **second materialisation kind** first, and that is the reusable half:
`SeriesKind::TrialReturns` gives a template the recorded return series of every run in an
experiment, long format `(run_id, period, ret)`. Long rather than wide because families are
ragged, and a wide table has to pad (inventing returns) or truncate (discarding the most
recent, which are what an out-of-sample claim rests on) *before* the template can see either
choice being made. Long defers the choice to the tool, where it gets stated in the result.
Nothing is dispatched to produce it — these are results the platform already holds, so asking
spends no trial. This unblocks S-3.16 and S-3.17 as well.

`shrunk_covariance` reports the **raw estimate beside the shrunk one**, which §5.7 asks for
and the reason is not symmetry: shrinkage pulls toward historical average correlation, which
is exactly wrong just before a correlation breakdown — the moment diversification stops
working is the moment the shrunk estimate is most confident it still does. The intensity is
Ledoit-Wolf's optimal value and is not a parameter, because a caller who can shrink can shrink
until the book looks diversified. The Marchenko-Pastur edge separates factors from noise:
verified on a fixture carrying exactly one planted common factor, where it finds exactly one —
a tool calling every eigenvalue a factor, or none, fails that test.

`allocate_book`'s cap is applied **after** the objective and then asserted, which is the
acceptance criterion read literally. A cap inside an optimisation is a constraint the
optimiser negotiates with: give it a target it cannot reach and it finds the corner of the
feasible set, which is the cap, on the strategy with the best in-sample numbers. Nothing that
produces weights is consulted about the cap, and a run whose weights fail the final check
refuses rather than returning them.

That needed a fixture of its own to mean anything. On the well-behaved family the largest
weight is 0.15 against a cap of 0.25, so the clip is a branch no run takes;
`family_concentrated.parquet` gives one strategy a hundredth of the variance of the rest —
what a barely-trading strategy actually looks like, and what a variance-minimising optimiser
wants all of. Minimum variance wants ~99% of the book in it and receives 0.25.

**Missing: `check_control_limits`.** It is a read over phase 7's monitor tables, which are
themselves 5 done and 7 partial. Building a read against half-built tables would produce a
tool that returns empty and reads as "no limits are breached" — the same failure shape as the
leakage scan deferred in S-3.11.

`shrunk_covariance` (Ledoit-Wolf, **raw always reported beside shrunk**, because shrinkage
biases toward historical structure which is exactly wrong before a correlation breakdown),
`allocate_book` (max single-strategy weight enforced outside the objective so an optimiser
cannot concentrate to hit a target), `check_control_limits` (**read-only** over Phase 7's
tables).

**Acceptance:** `allocate_book` cannot exceed the weight cap by any input; `check_control_limits`
has no write path and no agent-callable halt.
**Traces:** AGENT-006 §5.7, COMP-007 §6.

### S-3.19 Spending guards: call budgets, cost preview, self-status — M
`PARTIAL` 2026-09-16 — all three tools exist; `estimate_cost` cannot produce dollars, and
that is **OQ-16** rather than a gap to fill in.

**`max_calls_per_run` was already enforced** and needed nothing. `crates/mcp-server/src/gate.rs`
returns a typed `call_budget_exhausted` naming the budget and the count, ahead of the handler.
This entry says so rather than claiming credit for it.

**`estimate_cost`** — `crates/mcp-server/src/tools/core.rs`, `crates/api/src/routes/agent_cost.rs`,
`SuiteManager::observed_throughput`. It dispatches nothing and spends no trial. Of its three
numbers:

* `trials` is **exact** — the target tool's own `spends_trial`, which is also what the call
  gate reads, so it cannot disagree with what happens;
* `est_seconds` is measured then extrapolated, with a band and a `confidence`. The throughput
  comes from the caller's **own completed runs** (rows per second of wall clock), because
  nothing in this platform configures one — `ledger::internal` says as much: the closed form
  is "a rule with no table behind it". The runs are the table. With none, the answer is
  `null` and `unmeasured`, never a constant;
* `est_usd` is **always null**. There is no price table anywhere — a workspace grep finds no
  price per token, per second or per GPU-hour, and `jobs::models` answers in seconds.
  **OQ-16.** A made-up rate is worse than none, because `ask_above_usd` pauses for a human:
  too low never pauses, too high pauses on everything, and both look like the gate working.

Adding it flipped `spec_15_tool_surface_conformance`'s `("estimate_cost", None)` row to
`Some("estimate_cost")` — which is that test doing its job in the direction it advertises
("a pending tool that quietly appears").

**`get_self_status`** — harness-owned, `crates/harness/src/{registry,drive}.rs`. It reads this
loop's own fields and answers in the loop, costing a turn and not a step, like `reflect`.

Two things it refuses to do, both deliberate:

* it does **not** report the monitor or kill-switch state, which §2.4b lists. The loop is
  sans-IO and has not seen them, and a stale or invented kill-switch reading would defeat the
  tool's whole purpose — an agent that cannot tell it is degraded reporting confidently from a
  degraded state;
* it does **not** report tokens or dollars remaining as though they were tracked.
  `BudgetView::usd_tenth_cents_remaining` and `tokens_remaining` are declared and **nothing
  anywhere writes them** (checked by grep: one hit each, the declaration). The state card
  renders neither, so nothing misleading reaches a model today — but a future line in that
  renderer would print "$0.00 remaining", and the tool says the axes are untracked instead.

**It is withheld on profiles that do not reflect** (ADR-S3-12). Core tools are selected before
namespace filling and count against `max_exposed_per_step`; the tightest shipped profile has
three slots, and the eval's degraded 7B has six of which three are core — the arithmetic
§8.3's truncation case is built on.

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
