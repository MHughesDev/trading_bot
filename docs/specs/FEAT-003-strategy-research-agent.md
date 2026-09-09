# FEAT-003: Strategy Research Agent

**Status:** Phase 1 implemented (2026-09-08); Phases 2–4 planned
**Version:** 0.2
**ADR(s):** ADR-0022 (MCP thin client + internal agent) · ADR-0023
(LLM proposes structure, optimiser chooses parameters, Set J judges) ·
honours BACKTEST_SUITE_CORE_SPEC INV-1/INV-2/INV-3 and its ADR-001/ADR-002

**Phase 1 as built** — typed `parameters` (v1.2) with materialisation
(`crates/domain/.../params.rs`); the real `SimRunExecutor` (Set K live leg);
`Objective` + `MetricKind::Expectancy`; `DiagnosticBundle`; `crates/research`
(search space, seeded Random/TPE samplers, surface summary, sweep loop with
the §7.3 INV-2 path, fidelity calibration); `SuiteManager` generic over a boxed
executor, lock-free study execution, `strategy_ref`/`objective`/`base_params`,
sampler-facing `run_param_batch`; `/api/research/*`; fourteen agent tools;
driver `run_sweep` intercept; research prompt; timeline sweep cards.
Deferred from Phase 1: fidelity *racing* (calibration exists, racing stays
off until measured — P5), `sweep_max_parallel` config (fixed at 2), and the
funnel still executing under the suite lock (pre-existing).
**Crates:** `crates/api/src/agent` (driver) · `crates/backtest` (Set J: run,
study, experiment, nulls, gates, stats) · `crates/mcp-server` (tool layer) ·
`crates/domain/strategy_def` (grammar) · `crates/features` (regime features) ·
new `crates/research` (sweep engine, campaign, knowledge base)
**Companion study:** `docs/research/strategy-search-study.md` (literature and
rationale; this spec is the plan)

---

## 1. Purpose

Turn the internal agent (ADR-0022) from a 15-turn guess-and-check loop into a
**research agent** that searches the space of trading strategies efficiently,
changes a strategy's *structure* in response to diagnostics, tunes its
*parameters* numerically, and cannot fool itself — because every result it
reports has passed through Set J's sealed distributions, null tests, trial
counter and gate funnel.

The agent's product is not "a strategy with a good Sharpe". It is a
**vaulted Experiment** whose verdict carries its null, its trial count, its
deflated Sharpe, its PBO, its regime profile, and the full ledger of
hypotheses that led to it.

## 2. Scope & Non-Goals

**In scope**

- A sweep engine (inner loop) that tunes typed parameters with Bayesian /
  evolutionary samplers and multi-fidelity racing, under Set J invariants.
- A campaign loop (outer loop) in which the LLM proposes *structural moves*
  and *parameter ranges*, reads *diagnostics* and *surfaces*, and never sets a
  number itself.
- A regime engine producing labelled windows that feed Set J's
  `RegimeConditional` study and a learned regime→strategy prior.
- A knowledge base (hypothesis ledger, elite archive, regime table) that
  persists across runs.
- Exposure of Set J (Experiments, Studies, nulls, gates, vault) to the agent
  as tools.
- Agent-page UI additions to show campaigns, surfaces, gates and diagnostics.

**Out of scope (explicitly)**

- Live trading, automations, or arming anything. The research agent's tool
  profile stays read-only with respect to execution.
- End-to-end models that "predict the best strategy" from price data.
- Deep-learning price forecasting from OHLCV (that remains the model-training
  suite's concern, as forecast *features* strategies consume).
- Portfolio-level construction across many instruments (a later FEAT).

## 3. Governing principles

These are decided. They are restated here because every design choice below
is derived from them.

**P1 — Three spaces, three tools.**
Structure (discrete, needs ideas) → LLM inside an evolutionary loop.
Parameters (numeric) → numerical optimiser; **the LLM never sets a number**.
Context (when a structure works) → unsupervised regime detection; the
regime→strategy map is learned from our own backtests, never supervised.

**P2 — Set J is the judge, and the agent goes through it, not around it.**
Every backtest the agent causes is a Run inside a Study inside an Experiment.
The trial counter increments automatically (INV-1). Distributions are sealed:
nothing the agent can call returns "the best member" (INV-2 / ADR-002). No
significance claim reaches the agent without its null and trial count
(INV-3). The agent cannot read the holdout until the Experiment is at the
vault gate.

**P3 — Plateaus, not spikes.** What is carried forward from any distribution
is chosen by Set J's `SelectionRule` (`MedianStableCentroid` /
`WorstCaseRobust`), never argmax. The LLM sees the *surface*, not the peak.

**P4 — Diagnostics drive edits.** The LLM changes a strategy in response to a
diagnostic bundle (trade-level, time-sliced, regime-sliced, failure
attribution), not a summary metric.

**P5 — Cheap before expensive, but only when cheap predicts expensive.**
Multi-fidelity racing is enabled per (instrument, tier) only after a
calibration study shows rank correlation between tiers ≥ 0.8. Otherwise
full fidelity.

**P6 — Efficiency is measured.** Backtests per accepted improvement, LLM
tokens per accepted improvement, fraction killed at low fidelity, campaign
PBO, feature-map coverage. A campaign that improves the score while PBO rises
is failing.

## 4. Architecture

```
                         ┌───────────────────────────────────────────────┐
                         │ Knowledge Base  (Postgres)                     │
                         │ hypothesis ledger · elite archive (feature map)│
                         │ regime→strategy table · fidelity calibration   │
                         └──────────▲──────────────────────▲─────────────┘
                                    │ context               │ outcomes
┌────────────┐   regime   ┌─────────┴─────────┐   ┌─────────┴──────────┐   ┌──────────────────┐
│ Regime     │ ─windows─▶ │ Campaign          │──▶│ Sweep Engine       │──▶│ Set J            │
│ Engine     │            │ (outer loop)      │   │ (inner loop)       │   │ Experiment/Study │
│ features → │            │ LLM proposes      │◀──│ sampler picks      │◀──│ nulls · gates ·  │
│ jump model │            │ moves + ranges;   │   │ where to sample;   │   │ trial counter ·  │
│ → labels   │            │ islands/MAP-Elites│   │ racing; reports    │   │ vault            │
└────────────┘            │ bandit allocation │   │ sealed distribution│   └──────────────────┘
                          └───────────────────┘   └────────────────────┘
                                    ▲                        │
                                    └── diagnostics bundle ◀─┘
```

### 4.1 Component → crate map

| Component | Where | Reuses | New |
|---|---|---|---|
| Typed parameters | `crates/domain/strategy_def` | `ParamMap` on `RunConfig` | `parameters:` block on definition; `param('name')` in grammar (v1.2 additive) |
| Sweep engine | new `crates/research/sweep` | Set J `ParameterSweep`/`Neighborhood` studies, `is_plateau`, `SelectionRule`, backtest manager fan-out | sampler adapters (`optimizer` crate: TPE/GP/CMA-ES/BOHB), fidelity ladder, calibration, surface summariser |
| Diagnostics | `crates/backtest/stats` + `run` | `MetricSet`, `Trade` | `DiagnosticBundle` builder (§7) |
| Objective | `crates/backtest/run` | `MetricKind` | `Objective { primary, constraints }` (§6) |
| Campaign | new `crates/research/campaign` + `crates/api/src/agent` | driver loop, budgets, transcript, run-scoped tokens | move vocabulary, islands + feature map, bandit compute allocation, insight curation |
| Regime engine | `crates/features` (+ `apps/model-trainer` if Python jump model) | feature lane, ClickHouse bars | Hurst, VR, realised vol, ADX, autocorr; jump model; labelled windows → `VarySpec::Regimes` |
| Knowledge base | Postgres | `agent_runs` pattern | tables in §9 |
| Agent tools | `crates/mcp-server/tools` | `ToolProfile::InternalAgent` | `research.rs` (§8) |
| UI | `frontend/src/pages/AgentPage.tsx` | timeline, run list | campaign view, surface plot, gate ledger, diagnostics panel (§11) |

### 4.2 Why a new `crates/research`

The sweep engine and campaign are neither backtest primitives (they *use*
Set J) nor API concerns (they are long-running orchestration with their own
state). Keeping them out of `crates/backtest` preserves ADR-001 ("the Run is a
pure, dumb function") and keeps Set J's invariants enforced *at* Set J rather
than re-implemented by the thing that calls it.

## 5. Typed parameters (domain change)

**Problem.** The v1.0 expression grammar is `number | feature_ref | bar_ref`.
Parameters are literals inside expression strings. `RunConfig.params`
(`ParamMap = BTreeMap<String, Value>`) exists but nothing in the grammar can
reference it.

**Change (v1.2, additive, non-breaking — same pattern as the v1.1 AI nodes).**

```jsonc
// strategy definition, new top-level block
"parameters": {
  "fast": { "type": "int",   "default": 12, "min": 5,   "max": 50,  "step": 1 },
  "slow": { "type": "int",   "default": 26, "min": 20,  "max": 200, "step": 1 },
  "vol_gate": { "type": "float", "default": 0.02, "min": 0.005, "max": 0.08, "scale": "log" },
  "exit_mode": { "type": "enum", "default": "trail", "choices": ["trail", "fixed", "time"] }
}
```

Grammar: `primary = number | feature_ref | bar_ref | param_ref`,
`param_ref = "param" "(" "'" ident "'" ")"`. Feature names may also be
parameterised where the feature lane supports it (e.g. `ema(param('fast'))`
resolves through the existing feature-set registry; if unsupported for a
feature, the validator rejects it — fail closed, as today).

**Rules.**
- `strategy-validator` rejects a `param_ref` with no matching declaration,
  a default outside `[min,max]`, and any constraint the sweep cannot
  enumerate.
- A definition with a `parameters` block and no overrides runs on defaults —
  existing definitions are unaffected.
- `RunConfig.params` is the only override path. The sweep engine never edits
  expression strings.
- Cross-parameter constraints (e.g. `fast < slow`) are declared as
  `"constraints": ["param('fast') < param('slow')"]` using the same grammar;
  infeasible samples are rejected before a Run is created and do **not**
  count as trials.

## 6. Objective (as data)

```rust
pub struct Objective {
    pub primary: MetricKind,            // Sortino | Calmar | ProfitFactor | Expectancy | DetrendedSharpe | Sharpe
    pub constraints: Vec<Constraint>,   // all must hold or the sample scores -inf
    pub aggregate: Aggregate,           // how a distribution collapses to a scalar for *sampling* only
}
pub enum Constraint {
    MaxDrawdownLte(f64),
    MinTrades(u32),                    // default 50; never below 30
    MaxTurnoverPerYear(f64),
    MinTimeInMarket(f64), MaxTimeInMarket(f64),
    MinRegimeCoverage(u32),            // profitable in at least N regimes
}
pub enum Aggregate { Median, Worst5Pct }  // never Max (INV-2)
```

- **Add `MetricKind::Expectancy`** (mean P&L per trade / mean loss) to the
  existing set; keep win rate available in diagnostics but *not* selectable
  as `primary`.
- `Objective` is stored on the Experiment at creation and is immutable for
  its lifetime (changing the objective is a new Experiment, as changing the
  primary null is).
- The `aggregate` is used only by the sampler to rank *its own* samples
  while deciding where to look next. It is never what is reported or carried
  forward (§7.3).

## 7. Sweep engine (inner loop)

### 7.1 Inputs / outputs

```
SweepRequest {
  experiment_id, strategy_ref, strategy_version,
  space: from the definition's `parameters` block, optionally narrowed by the LLM
         ("fast in [8, 30]") — narrowing only, never widening past the declaration,
  objective: Objective,
  sampler: Random | Tpe | Gp | CmaEs | Bohb   (default Bohb when racing enabled, else Tpe),
  budget: { max_runs, max_wall_ms },            // Set J StudyBudget
  fidelity: Ladder | Full,
}
SweepReport {
  study_ids: [..],                    // the Set J studies created (ParameterSweep + Neighborhood)
  distribution: Distribution,         // sealed: median, spread, worst_5pct, n, is_plateau
  carried_forward: Option<ParamMap>,  // chosen by SelectionRule, never argmax
  surface: SurfaceSummary,            // §7.4 — for the LLM
  trials_consumed: u32,               // what this sweep added to the Experiment counter
  racing: { enabled: bool, killed_at_tier: [u32; N] },
}
```

### 7.2 Samplers

Use the Rust `optimizer` crate (MIT, 1.0.1, 2026-07): `RandomSampler`,
`TpeSampler`, `GpSampler`, `CmaEsSampler`, `BohbSampler` (TPE + Hyperband),
`DESampler`, pruners, async parallel evaluation, JSONL journal. Pure Rust —
no Python round-trip in the inner loop. Search-space types map 1:1 onto §5
(`IntParam`, `FloatParam` with log scale, `CategoricalParam`).

### 7.3 INV-2 resolution (this is the important part)

A surrogate-driven sampler *must* look at sample scores to decide where to
sample next. That is exploration and is permitted. What is **not** permitted
is for any of that to leak into what is reported or promoted. Concretely:

1. The sampler runs inside the sweep engine and sees per-sample objective
   values. Its internal state is never exposed through a tool.
2. Every sample becomes a Run in a Set J `ParameterSweep` Study. The Study
   seals the distribution. The Experiment trial counter increments per Run,
   automatically (INV-1).
3. When the sampler stops, the engine issues a `Neighborhood` Study around the
   region the sampler concentrated on, and asks Set J for the
   `SelectionRule::MedianStableCentroid` member of that neighbourhood. **That**
   is `carried_forward`. The sampler's argmax is discarded.
4. The `SweepReport` exposes `distribution` (sealed) and `surface` (a
   description), never a ranked list of samples.

Net effect: BO/CMA-ES make the search *efficient*; Set J makes the *answer*
honest. The two never touch the same field.

### 7.4 Surface summary (what the LLM reads)

```
SurfaceSummary {
  per_param: [{ name, plateau: Option<(lo, hi)>, cliffs: [value], sensitivity: Low|Med|High }],
  interactions: [(a, b, "fast<slow region dominates")],   // top-3 pairwise, from the surrogate
  is_plateau: bool,                                        // Set J's CV<0.5 test on the neighbourhood
  text: "fast: plateau 10–28, cliff >35. slow: insensitive 60–180. vol_gate: high sensitivity, best region log-mid."
}
```

Computed from the sampled points (binned marginals) and, when a GP/TPE
surrogate exists, from its marginal means. No LLM call is involved.

### 7.5 Fidelity ladder and calibration (P5)

Tiers are declared per instrument, e.g.

| Tier | Timeframe | Window | Sim | Relative cost |
|---|---|---|---|---|
| 0 | 1h | last 90 d | full engine | 1× |
| 1 | 15m | last 1 y | full engine | ~16× |
| 2 | 1m | full history | full engine | ~200× |

**Calibration study** (once per instrument per strategy family, refreshed
monthly): sample K=40 configurations, run all at every tier, compute
Spearman ρ of objective ranks between adjacent tiers. Store in
`fidelity_calibration`. Racing between tiers *t* and *t+1* is enabled only if
ρ(t, t+1) ≥ 0.8. If a tier fails calibration the ladder collapses to full
fidelity for that pair — the search is slower but never wrong-by-design.
(The HPO literature is explicit that low-budget ranks frequently do not
predict high-budget ranks; this is why it is measured rather than assumed.)

Successive halving with η = 3: tier 0 runs N, tier 1 runs N/3, tier 2 runs
N/9. Kills are logged as trials (they are Runs). A killed configuration is
never "the best member" of anything, so racing is INV-2-safe.

### 7.6 Concurrency

The sweep engine acquires backtest-manager permits like any other caller
(`MAX_CONCURRENT_RUNS` semaphore). `AgentConfig` gains
`sweep_max_parallel` (default 4) so one campaign cannot starve interactive
backtests.

## 8. Campaign loop (outer loop) and the LLM protocol

### 8.1 Objects

- **Campaign** — one goal, one instrument (v1), one Objective, a budget in
  *backtests + wall clock*, N islands, a feature map, a bandit allocator, and
  a ledger. Owns many **Runs** of the existing agent driver.
- **Candidate** — `(hypothesis, definition, sweep_report, gate_ledger,
  diagnostics, feature_cell)`; lives in an island; may be an elite.
- **Move** — a typed structural edit (§8.3) proposed by the LLM with a stated
  hypothesis and the diagnostic evidence it responds to.

### 8.2 One generation

1. **Allocate.** The bandit (Thompson sampling over per-island improvement
   rate) picks an island and whether to *deepen* (mutate an elite) or
   *broaden* (seed a new category). A fixed 20% of generations are forced
   exploration.
2. **Context.** Assemble for the LLM: the parent candidate (definition,
   surface, diagnostics, gate ledger), 2–3 cousins (best-in-map, feature-space
   neighbour, random), curated insights, the regime profile of the instrument,
   and the regime→strategy prior.
3. **Propose.** The LLM returns exactly one `Move` + hypothesis + parameter
   *ranges* for any new/changed parameters. Structured output; invalid moves
   are rejected with the validator's reason and the LLM retries once.
4. **De-duplicate.** Compute the candidate's signal series on tier 0; if
   Pearson ≥ 0.98 with an existing candidate in the campaign, reject without
   a backtest (RD-Agent's IC de-dup, adapted). Not a trial.
5. **Sweep.** §7. Produces `carried_forward` params and a surface.
6. **Gate.** Advance the candidate's Experiment through Set J: Gate 0
   (integrity), Gate 1 (single path), Gate 2 (CPCV + synthetic worst-5%),
   Gate 3 (primary null, DSR ≥ 0.95, PBO ≤ 0.5). The agent's tools expose the
   *ledger*, never the holdout metric.
7. **Diagnose.** Build the `DiagnosticBundle` (§7 → below) for whichever
   gate it reached.
8. **Place.** Map to a feature cell; replace the incumbent if its
   `WorstCaseRobust` metric is higher (a sealed-distribution comparison, not
   a point comparison). Losers go to the archive with their diagnostics.
9. **Learn.** Append to the hypothesis ledger; update the regime→strategy
   table with the candidate's regime-sliced results; update the bandit.
10. **Curate** every K=10 generations: the LLM summarises the ledger into ≤20
    insights, de-duplicated; older insights decay.

### 8.3 Move vocabulary (v1)

Over `NodeKind` as it exists (`Condition`, `Signal`, `Filter`, `Rank`,
`TakeTopN`, `DataSource`, AI nodes) plus actions and risk overrides:

| Move | Effect | Machine check |
|---|---|---|
| `add_condition_filter(expr)` | AND a new predicate into a Signal's condition | grammar + validator |
| `remove_node(id)` | drop a node and rewire | graph stays acyclic and complete |
| `swap_feature(node, from, to)` | change the feature a Condition reads | feature exists in lane |
| `change_exit(mode)` | switch exit logic (trail / fixed / time / signal) | actions valid |
| `add_regime_gate(regime_label)` | only trade when regime ∈ set | regime engine has the label |
| `change_sizing(mode, params)` | fixed / vol-scaled (when simulator supports it) | sim capability |
| `simplify()` | remove the node whose ablation changes the objective least | ablation Runs (count as trials) |
| `combine(a, b, how)` | crossover: take entries from *a*, exits/filters from *b* | validator |

`simplify` is run automatically on every elite before it can reach Gate 3:
a node whose removal does not reduce the `WorstCaseRobust` metric is
removed. Complexity is overfitting's friend.

### 8.4 Termination contract (replaces the current `FINAL:` free text)

The LLM ends a Run with a structured `final` block:
`{ experiment_id, candidate_id, verdict_summary, what_works, what_is_fragile,
next_moves: [Move] }`. The campaign, not the LLM, decides whether to spend
budget on `next_moves`. A campaign ends on budget, on a vaulted candidate
whose `next_moves` the bandit deems not worth pursuing, or on cancel.

### 8.5 Budget

`CampaignBudget { max_backtests, max_wall_secs, max_llm_tokens }`. The
existing per-Run `max_iterations` remains as a guard but is no longer the
primary denomination.

## 9. Knowledge base (Postgres)

```sql
research_campaigns(campaign_id, user_id, goal, instrument_id, objective_json,
  budget_json, status, islands_json, feature_map_json, bandit_state_json,
  created_at, finished_at)
research_candidates(candidate_id, campaign_id, island, parent_id, experiment_id,
  strategy_ref, strategy_version, hypothesis, move_json, sweep_report_json,
  diagnostics_json, feature_cell, is_elite, created_at)
research_ledger(id, campaign_id, seq, kind, content_json, created_at)   -- hypotheses, outcomes, insights
research_insights(campaign_id, seq, text, supports_candidates[], decayed_at)
regime_labels(instrument_id, timeframe, ts, label, prob, model_version)  -- ClickHouse if high-frequency
regime_strategy_prior(instrument_id, regime_label, strategy_family,
  n_obs, worst5_metric_mean, median_metric_mean, updated_at)
fidelity_calibration(instrument_id, strategy_family, tier_from, tier_to,
  spearman_rho, n, computed_at)
```

`research_candidates.experiment_id` is the link into Set J; nothing in these
tables duplicates a metric that Set J owns — they store *references* and the
agent-facing derived summaries.

## 10. Regime engine

- **Features** (new in `crates/features`): Hurst exponent (R/S and DFA),
  variance ratio (Lo–MacKinlay, lags 2/4/8), realised vol and vol-of-vol,
  ADX, return autocorrelation at lags 1/5/20, skew/kurtosis over a rolling
  window, volume z-score. Computed on the instrument's canonical bars with
  `available_time` semantics (no look-ahead — reuse the leakage harness).
- **Model**: statistical jump model (Nystrup–Kolm–Lindström lineage) for
  persistent regimes. Two implementation options, decision open (§14):
  Python `jump-models` (Apache 2.0, sklearn API, online predict) in
  `apps/model-trainer`, labels persisted; or a Rust implementation (k-means +
  jump-penalised dynamic programme; small). Start with 3 regimes; choose the
  penalty by the library's information criterion; refit monthly.
- **Outputs**: `regime_labels` per bar, and labelled windows
  `(start, end, label)` in exactly the shape `VarySpec::Regimes` already
  takes. `RegimeConditional` studies therefore work unchanged.
- **Prior**: after every candidate's regime-sliced study, update
  `regime_strategy_prior` by `(instrument, regime, strategy_family)`. The
  campaign context includes the instrument's regime occupancy and the top
  families per regime. This is a contextual-bandit table, not a classifier.

## 11. Agent tool surface (additions to `ToolProfile::InternalAgent`)

| Tool | Returns | INV notes |
|---|---|---|
| `create_experiment(strategy_family, objective, primary_null, holdout)` | experiment_id | holdout locked at creation |
| `get_experiment(id)` | state, trial_count, gate ledger, study refs | never holdout metrics |
| `run_sweep(SweepRequest)` | `SweepReport` | sealed; §7.3 |
| `get_surface(study_id)` | `SurfaceSummary` | description only |
| `get_diagnostics(run_or_study_id)` | `DiagnosticBundle` | trade-level, sliced, attributed |
| `run_study(kind, vary, question)` | study_id + sealed distribution | any Set J kind |
| `advance_gate(experiment_id)` | new ledger entry or refusal reason | Gate 3 requires null + counter (INV-3) |
| `get_regime_profile(instrument)` | occupancy, transitions, current label | |
| `get_regime_prior(instrument)` | top families per regime with n_obs | |
| `propose_move(Move, hypothesis, ranges)` | validation result | the only way to change structure |
| `list_ledger(campaign_id)` / `list_insights(campaign_id)` | ledger, insights | |

Removed from the internal profile: direct `create_backtest` / `rerun_backtest`
(the agent may no longer create Runs outside a Study). They remain in the
MCP profile for humans.

### 11.1 `DiagnosticBundle`

```
{ trades: { n, win_rate, avg_win, avg_loss, expectancy, pf, hold_time_p50, mae_p50, mfe_p50 },
  by_month: [{ ym, ret, dd, n }],
  by_regime: [{ label, ret, sortino, n, share_of_time }],
  worst_trades: [{ ts, pnl, entry_features: {..}, regime }],   // 10
  longest_drawdown: { start, end, depth, regime_path },
  exposure: { time_in_market, turnover_yr, cost_drag_pct_of_gross },
  surface: SurfaceSummary?,                                    // when from a sweep
  text: "..." }                                                // ≤ 1.5 KB, LLM-facing
```

## 12. UI (Agent page)

- Left rail gains **Campaigns** above Runs: goal, instrument, budget bars
  (backtests / wall / tokens), generation counter, PBO trend sparkline.
- Right panel, per campaign: a **feature-map grid** (cells coloured by
  `WorstCaseRobust` metric, click → candidate), the **gate ledger** for the
  selected candidate (five gates, verdict, null, trial count at verdict), a
  **surface plot** per parameter (binned marginal with plateau shading), and
  the **diagnostics panel** (by-regime bars, worst-trades table).
- The existing Run timeline stays; `tool_call`/`tool_result` rows for
  `run_sweep` render the surface inline.
- Settings: fidelity ladder editor per instrument with calibration ρ shown.

## 13. Efficiency metrics (P6)

Computed per campaign and shown in the UI header:

- `backtests_per_improvement` = trials / (elite replacements)
- `tokens_per_improvement`
- `low_fidelity_kill_rate` = runs killed at tier < max / total runs
- `campaign_pbo` = PBO over all candidates' train/test matrices
- `map_coverage` = occupied cells / total cells
- `dedup_savings` = candidates rejected by correlation / proposed

Acceptance for Phase 2 (below) includes a target for the first two.

## 14. Decisions

**Decided (agreed on the study):**

1. The LLM never sets a numeric parameter; it proposes ranges within the
   declaration. The sampler chooses.
2. Objective is data: primary + constraints; Sortino / Calmar / ProfitFactor
   / Expectancy / DetrendedSharpe / Sharpe selectable; win rate never
   primary; `MinTrades` default 50.
3. Walk-forward for the realistic path; CPCV + synthetic paths at Gate 2;
   DSR ≥ 0.95 and PBO ≤ 0.5 at Gate 3 (already Set J's thresholds).
4. Carried-forward selection is `MedianStableCentroid` (default) or
   `WorstCaseRobust`; never argmax.
5. Budget is denominated in backtests + wall clock + tokens.
6. Regime→strategy is a learned prior table, not a classifier.

**Open (need a call before the relevant phase):**

7. Jump model in Python (`jump-models`, trainer app) vs Rust-native. Lean
   Rust for a single binary, unless the Python side is already the home of
   fitting jobs — it is (`apps/model-trainer`), so lean Python for v1 and
   revisit.
8. `combine` (crossover) in v1 of the move vocabulary, or defer to v2.
9. Fidelity ladder tiers and η per asset class (proposed §7.5 for crypto
   spot).
10. Feature-map dimensions and bin counts (proposed: category × trading
    frequency × Sortino × MaxDD × regime-of-best, 8–16 bins on numeric
    axes, per QuantEvolve's ablation).
11. Number of regimes (proposed 3) and refit cadence (proposed monthly).
12. Whether `simplify` ablations count toward the trial counter (proposed:
    yes — they are Runs and INV-1 says so).

## 15. Phases and acceptance criteria

### Phase 1 — Inner loop and honesty (wiring, mostly)

Deliverables: §5 typed parameters + grammar v1.2; `Objective` + `Expectancy`;
`crates/research/sweep` with `Random`/`Tpe`/`Bohb`; §7.3 INV-2 path;
`SurfaceSummary`; `DiagnosticBundle`; fidelity calibration study + ladder;
Set J tools in §11 (`create_experiment`, `get_experiment`, `run_sweep`,
`get_surface`, `get_diagnostics`, `run_study`, `advance_gate`); removal of
raw `create_backtest` from the internal profile; migrations for
`fidelity_calibration`.

Acceptance:
- A definition with a `parameters` block round-trips through validator,
  storage, UI and backtest unchanged on defaults.
- `run_sweep` on the EMA-cross fixture produces a sealed distribution, a
  `carried_forward` equal to the `MedianStableCentroid` member, and
  increments the Experiment trial counter by exactly the number of Runs.
- No tool in the internal profile can return the argmax sample (test:
  property-based over `SweepReport` serialisation).
- Calibration on BTC-USD reports ρ per tier pair; racing is disabled where
  ρ < 0.8 and the report says so.
- The agent, given the current 15-turn protocol plus the new tools, completes
  a run that ends at Gate 2 or beyond with a diagnostic bundle in the
  transcript.

### Phase 2 — Outer loop

Deliverables: `crates/research/campaign`; islands + feature map; move
vocabulary + `propose_move`; de-duplication; bandit allocation; ledger +
insight curation; structured termination; campaign budget; UI campaign
view, feature-map grid, gate ledger, surface plot.

Acceptance:
- A 200-backtest campaign on BTC-USD 1h produces ≥ 1 candidate at Gate 3
  and an archive with ≥ 5 distinct feature cells occupied.
- `backtests_per_improvement` ≤ 25 and `campaign_pbo` ≤ 0.5 on the fixture
  campaign.
- Cancelling a campaign stops all sweeps within one poll interval and leaves
  every Experiment in a consistent state.
- Restart marks in-flight campaigns failed (existing orphan policy).

### Phase 3 — Context

Deliverables: regime features; jump model fit + labels; `get_regime_profile`,
`get_regime_prior`; regime-sliced diagnostics; `add_regime_gate` move;
`regime_strategy_prior` updates; efficiency metrics in UI.

Acceptance:
- Regime labels for BTC-USD are persistent (median regime length ≥ 5 days on
  daily features) and reproducible from stored features.
- A `RegimeConditional` study consumes engine windows unchanged.
- On the fixture campaign, the first-generation hypothesis cites the regime
  prior, and `backtests_per_improvement` improves versus Phase 2.

### Phase 4 — Hardening (no new capability)

Multi-instrument campaigns; refit scheduling; calibration drift alerts;
cost-model ladders inside sweeps (`CostSweep` study as a gate input);
documentation and an ADR-0023 write-up once the design has survived contact.

## 16. Risks

| Risk | Mitigation |
|---|---|
| Sampler efficiency leaks into selection (INV-2 breach) | §7.3 structural separation; property test in Phase 1 acceptance |
| Low-fidelity racing kills good candidates | Calibration gate (P5); racing off by default until ρ measured |
| LLM proposes invalid or redundant structures | Typed moves + validator; correlation de-dup; one retry then discard |
| Campaign converges on one idea | Feature map with category axis; forced 20% exploration |
| Trial counter inflation makes DSR unreachable | It should — that is the counter working. Mitigate with de-dup and racing so fewer trials are wasted, not by exempting Runs |
| Regime labels flicker | Jump penalty; persistence acceptance criterion |
| Compute starvation of interactive users | `sweep_max_parallel`; permits shared with backtest manager |
| Token cost | Per-campaign token budget; surface/diagnostic summaries capped at 1.5 KB; local models supported by keeping the tool profile small |

## 17. References

Companion study with full citations: `docs/research/strategy-search-study.md`.
Additional sources for this spec:
- `optimizer` crate — https://docs.rs/optimizer/latest/optimizer/
- `jump-models` (Python) — https://github.com/Yizhan-Oliver-Shu/jump-models
- Extending the statistical jump model for regime identification —
  https://link.springer.com/article/10.1007/s10479-024-06035-z
- Regime-aware asset allocation via statistical jump models —
  https://arxiv.org/html/2402.05272v1
- Multi-fidelity HPO review (rank-correlation caveats) —
  https://www.sciencedirect.com/science/article/pii/S2405959525000244
- AlgoEvolve: LLM-driven meta-evolution of trading programs (2026) —
  https://arxiv.org/pdf/2606.26173
- Deflated Sharpe Ratio (Bailey & López de Prado) —
  https://papers.ssrn.com/sol3/papers.cfm?abstract_id=2460551
- Grammar-guided GP operators — https://link.springer.com/article/10.1007/s00500-006-0144-9
