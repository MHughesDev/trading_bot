# Phase 1 — The §8 evaluation tools

**Completion: 67% (8 / 12 tasks), plus 3 `PARTIAL`, 1 `BLOCKED:UPSTREAM`.** Re-counted 2026-10-05 (track VERIFY2) from the per-task markers; it read 5 / 12.

**Requirement IDs:** EV-08
**Spec:** [BS-007 11_EVALUATION_STANDARD](../../../BRAINSTORM/BS-007_QUANT_RESEARCH_AGENT/11_EVALUATION_STANDARD.MD) §8, §9
**Depends on:** nothing inside this set.
**Migration range:** none.

BS-007 11 §8 lists twelve evaluation tools, eight of them High priority. (ROADMAP §5 and OPEN-TASKS say "11 High tools"; the table has eight High and four Medium, and
ADR-Q-01 records the count that was actually checked.) Several already exist under another name, and that is the first thing each task checks.

**The cheapest honest route is the templated tier** (S-3.1): a tool is a Python file whose docstring carries the catalogue entry, `build.rs` embeds it, and `crates/skills` parses
it for both the catalogue and the runner, so there is no Rust registration to forget. It fits the tools that are a statement about a family of recorded return series or about bars. It does *not* fit
the tools that need to dispatch runs (robustness suite) or to read gate verdicts (alpha-chain audit): those are workflow batteries (S-8) and need the platform.

---

### Q-1.1 `family_test`: a data-snooping-robust test over a candidate family — M
`DONE` 2026-10-04 — closed by S-3.9. Evidence: `crates/mcp-server/src/tools/audit.rs` (`family_wise_test`, spec at line ~586); ADR-S3-06.

Already shipped as `family_wise_test` (Set S S-3.9), which runs the existing `stats::bootstrap::romano_wolf`. The spec's `method=spa|stepwise_spa|romano_wolf` is deliberately one procedure
(ADR-S3-06: Romano-Wolf dominates SPA and White's Reality Check). This task closes the EV-08 line by recording the mapping (ADR-Q-02); it adds no tool.

**Files:** `apps/skill-runner/templates/audit/` (new template)
**Acceptance:** `family_wise_test` is in the catalogue, spends no trial, refuses a family under the minimum, and the descriptor budget holds.
**Traces:** EV-08; BS-007 11 §8

### Q-1.2 `posterior_alpha`: theme-posterior shrinkage with a credible interval — M
`DONE` 2026-10-04 — `attribution/posterior_alpha.py`. pytest `test_set_q_templates.py`: `a_family_with_one_planted_winner_is_pulled_back_toward_the_theme` (the estimate moves toward the theme mean and never past it), `a_family_with_a_common_mean_collapses_onto_it_and_says_so` (tau 0, interval not degenerate), `a_supplied_prior_is_labelled_and_needs_its_spread`, `two_members_are_not_a_family_and_an_unknown_candidate_is_refused`, reproducibility. Reached by production: a template under `apps/skill-runner/templates/` is embedded by `build.rs` and dispatched through `POST /api/agent/templates/run`; `cargo test -p mcp-server-lib --lib` (344 pass) holds the 16-per-namespace cap and the 1,200-byte descriptor budget with it in the catalogue, and `templates_run`'s `every_template_runs_on_the_reference_dataset_and_is_reproducible` and `every_declared_summary_field_is_actually_emitted` pass with it (that file's two other failures are older `portfolio` templates timing out at 30 s on a loaded machine, not this tool).

Empirical-Bayes shrinkage of a candidate's mean return toward the family (theme) mean, normal-normal with the between-candidate variance estimated from the family, returning the posterior mean
and a 95% credible interval and whether the interval excludes zero. The archetype prior from research memory is an optional input; without it the family is the prior and the result says so.

**Files:** `apps/skill-runner/templates/attribution/` (new template)
**Acceptance:** on a family with a common positive mean the shrunk interval is narrower than the raw one; on a family with a planted single winner among noise the winner's interval is pulled toward the family mean and, at small n, includes zero; the result states which prior it used; the call is reproducible (byte-identical `detail`).
**Traces:** EV-08; BS-007 11 §8

### Q-1.3 `capacity_report`: the size at which net expectancy reaches zero — M
`DONE` 2026-10-04 — `portfolio/capacity_report.py`. pytest: `net_edge_falls_with_size_and_break_even_scales_with_volume` (4x volume gives 4x break-even), `break_even_falls_with_volatility`, `an_edge_below_the_fixed_cost_has_a_zero_break_even_and_says_why`, `no_edge_declared_returns_the_cost_curve_and_no_break_even`; fee, spread and Y are labelled declared or placeholder in the output. The experiment-reading variant is the follow-up ADR-Q-03 names and is not part of the acceptance clause. Reached by production: a template under `apps/skill-runner/templates/` is embedded by `build.rs` and dispatched through `POST /api/agent/templates/run`; `cargo test -p mcp-server-lib --lib` (344 pass) holds the 16-per-namespace cap and the 1,200-byte descriptor budget with it in the catalogue, and `templates_run`'s `every_template_runs_on_the_reference_dataset_and_is_reproducible` and `every_declared_summary_field_is_actually_emitted` pass with it (that file's two other failures are older `portfolio` templates timing out at 30 s on a loaded machine, not this tool).

Square-root impact (`dp ~ Y sigma sqrt(Q/V)`) with sigma and ADV measured from the bars, a half-spread and fee, and a **declared** gross edge and turnover. Returns net expectancy against order size and the
break-even size. This is a *pre-trial estimate from a declared edge* and is distinct from Gate 4's ladder, which is measured from a Run's trades (`backtest::capacity`); the spec's
`capacity_report(experiment)` reading an experiment's trades needs an API route and is Q-1.3's follow-up (ADR-Q-03).

**Files:** `apps/skill-runner/templates/portfolio/` (new template)
**Acceptance:** net expectancy is strictly decreasing in size; break-even size scales with ADV and falls with sigma; a gross edge below the half-spread plus fee returns a break-even of zero and says why; the impact coefficient Y and its source are in the output.
**Traces:** EV-08; BS-007 11 §8

### Q-1.4 `benchmark_run`: the baselines on the same slice — M
`DONE` 2026-10-04 — `attribution/benchmark_run.py`. pytest: `buy_and_hold_is_the_price_return_net_of_one_round_trip`, `costs_lower_every_baseline`, `the_random_baseline_is_a_seeded_distribution`, `an_unholdable_trade_count_is_a_refusal_not_an_overlap`, `momentum_never_trades_on_the_bar_it_earns`. Mutation-checked: letting momentum read the next close fails that test (and the first draft's cost bug, where costs were subtracted flat from a compounded return and a long-only rule lost 215%, was found by running it). Reached by production: a template under `apps/skill-runner/templates/` is embedded by `build.rs` and dispatched through `POST /api/agent/templates/run`; `cargo test -p mcp-server-lib --lib` (344 pass) holds the 16-per-namespace cap and the 1,200-byte descriptor budget with it in the catalogue, and `templates_run`'s `every_template_runs_on_the_reference_dataset_and_is_reproducible` and `every_declared_summary_field_is_actually_emitted` pass with it (that file's two other failures are older `portfolio` templates timing out at 30 s on a loaded machine, not this tool).

Buy-and-hold, a random-entry baseline matched to a stated trade count and holding period, and simple momentum, all on the same bars and under the same fee and half-spread, so a strategy's number is
never read without what a dumb rule earns on the same data. The random-entry baseline is seeded and reports a distribution, not a point.

**Files:** `apps/skill-runner/templates/attribution/` (new template)
**Acceptance:** buy-and-hold equals the bar-to-bar return net of one round trip; the random-entry distribution's median is near the buy-and-hold drift times exposure; momentum is lagged one bar (no look-ahead, mutation-checked); costs strictly lower every baseline.
**Traces:** EV-08; BS-007 11 §8

### Q-1.5 `expected_decay`: the haircut prior for the dossier — S
`DONE` 2026-10-04 — `attribution/expected_decay.py` (McLean-Pontiff: 26% out of sample, 58% after publication; labelled a prior estimated on US equities). pytest: `a_published_idea_gets_the_larger_haircut`, `an_unknown_source_is_refused`. Reached by production: a template under `apps/skill-runner/templates/` is embedded by `build.rs` and dispatched through `POST /api/agent/templates/run`; `cargo test -p mcp-server-lib --lib` (344 pass) holds the 16-per-namespace cap and the 1,200-byte descriptor budget with it in the catalogue, and `templates_run`'s `every_template_runs_on_the_reference_dataset_and_is_reproducible` and `every_declared_summary_field_is_actually_emitted` pass with it (that file's two other failures are older `portfolio` templates timing out at 30 s on a loaded machine, not this tool).

McLean-Pontiff: about -26% out of sample and about -58% after publication, as a prior keyed by `(archetype, source)` with the figure's provenance. A literature idea starts with a decay prior and an in-house
idea with the out-of-sample figure only. A pure lookup with arithmetic; the tool says the number is a *prior*.

**Files:** `apps/skill-runner/templates/attribution/` (new template)
**Acceptance:** a published-anomaly input returns the larger haircut; an unknown archetype returns the out-of-sample figure and says it is generic; the source citation is in the output.
**Traces:** EV-08; BS-007 11 §8

### Q-1.6 `compare_to_random`: the agent against a matched-budget random search — M
`BLOCKED:UPSTREAM` 2026-10-04 (track R-MQ) - re-checked: RB-3.1 (the random arm) is still NOT STARTED.

Needs the shadow random arm (Set R-b RB-3.1); until a campaign has a random arm there is nothing to compare. The tool reads the sealed metric of both arms and reports the agent's rank in the random arm's distribution.

**Files:** `apps/skill-runner/templates/attribution/` (new template)
**Acceptance:** a campaign with no random arm returns a typed `no_random_arm`; with one, the agent's percentile is reported with the arm's trial count.
**Traces:** EV-08; BS-007 11 §8

### Q-1.7 `alpha_chain_audit`: the five stages — L
`DONE` 2026-10-05 (track R-MQ) - `gates/alpha_chain.rs` (pure audit, ADR-Q-08), GET /api/ledger/experiments/{id}/alpha_chain, MCP `alpha_chain_audit`. Verified 2026-10-05: compiled and run green (backtest lib 563, mcp-server-lib 388 incl. the 16-per-namespace cap and 1,200-byte budget, templates_run 9, pytest 51).

PIT information, stable signal, feasible positions, executable orders, net alpha after costs with capacity: each `pass | fail | unknown` from evidence the gate stack already stores (Gate 0/1, the family tests, the exits
and fee reads, gates 3 and 4). A stage with no evidence is `unknown`, never `pass`. A workflow battery (S-8), not a template.

**Files:** `crates/mcp-server/src/tools/workflows/` (outside this track's new-files rule) and `crates/api` read route
**Acceptance:** an experiment with no Gate 3 evidence reports stage five `unknown`; a stage's verdict names the gate row it read; no stage can be `pass` from an absent row.
**Traces:** EV-08; BS-007 11 §8

### Q-1.8 `run_robustness_suite`: cost ladder, delay, sub-periods, neighbourhood, perturbations, regime — L
`PARTIAL` 2026-10-04 (track R-MQ) - `crates/mcp-server/src/tools/robustness.rs`: six existing studies posted to the studies route; the result lists failed, not_run and runs_counted (ADR-Q-09). Verified 2026-10-05: compiled and run green (backtest lib 563, mcp-server-lib 388 incl. the 16-per-namespace cap and 1,200-byte budget, templates_run 9, pytest 51).

One call that runs the battery the funnel's robustness gate already needs: the cost ladder (`backtest::capacity::cost_ladder`), delay-by-one-bar (`RunConfig::fill_delay_bars`),
sub-periods, the parameter neighbourhood, transformation perturbations and the regime-conditional split. Each member is a counted run. A workflow battery.

**Files:** `crates/mcp-server/src/tools/workflows/batteries.rs` (Set S territory: report the hook)
**Acceptance:** the suite's runs all carry `experiment_id`; the trial counter rises by exactly the number of runs; a one-bar delay changes the fills; the result lists which of the six failed.
**Traces:** EV-08; BS-007 11 §8

### Q-1.9 `compare_candidates` — S
`PARTIAL` 2026-10-04 — owned by S-10.10, itself `PARTIAL` (the tool needs real measurements); `research::compare` is built. Not counted as new work here.

Owned by S-10.10 (the tool) and `research::compare` (the sweep). Listed here so EV-08 reads complete.

**Files:** `apps/skill-runner/templates/audit/` (new template)
**Acceptance:** see S-10.10.
**Traces:** EV-08; BS-007 11 §8

### Q-1.10 `decompose_strategy_returns`: vol-timing, drift and selection — M
`DONE` 2026-10-04 (track R-MQ) - `attribution/decompose_strategy_returns.py`; pytest: constant long is all drift, vol-timed shows timing with R2 > 0.8, components sum to the total, wrong length refused; mutation-checked (drift term halved fails two tests). Reached through the templated tier like Q-1.2 to Q-1.5; the catalogue caps were not re-run (see note on the mcp build).

Medium priority. Splits a strategy's return into the part explained by timing volatility, the part explained by directional drift (exposure times the asset's mean) and the residual selection component.

**Files:** `apps/skill-runner/templates/attribution/` (new template)
**Acceptance:** a constant-exposure long strategy decomposes into drift only; a vol-timed constant-direction one shows a timing component; components sum to the total within rounding.
**Traces:** EV-08; BS-007 11 §8

### Q-1.11 `factor_attribution` — S
`PARTIAL` 2026-10-04 — covered by Set S's `factor_spanning_test` (itself `PARTIAL`); the size and momentum factor series the spec names are `BLOCKED:UPSTREAM` on data (Set P) and the tool says so.

Covered by `factor_spanning_test` (Set S): market beta and alpha against the platform's one factor. The crypto size and momentum factors of the spec need factor series the platform does not hold.

**Files:** `apps/skill-runner/templates/attribution/` (new template)
**Acceptance:** see S-3.x factor spanning.
**Traces:** EV-08; BS-007 11 §8

### Q-1.12 `collider_check`: filters caused by both the signal and the future return — M
`DONE` 2026-10-04 (track R-MQ) - `audit/collider_check.py`; pytest: signal plus forward dependency flagged, declared confounder not, structural check stated; mutation-checked. `templates_run.rs` has three new argument entries.

Medium priority and the hardest to do honestly: a filter that is plausibly a common effect of the signal and of future returns biases the result when conditioned on. The tool lists each filter's declared causal role and flags the structural pattern; it never claims to *prove* a collider.

**Files:** `apps/skill-runner/templates/audit/` (new template)
**Acceptance:** a filter that is a function of the signal's own output and of a forward-looking quantity is flagged; a filter with a declared causal role of `confounder` is not; the output says it is a structural check.
**Traces:** EV-08; BS-007 11 §8
