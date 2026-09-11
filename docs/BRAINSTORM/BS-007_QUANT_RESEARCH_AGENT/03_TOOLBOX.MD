# BS-007 / 03 — The toolbox

> **Additions from later docs:** 10 §11 (quant tools from research) and 11 §5 (research
> standard tools: `posterior_alpha`, `collider_check`, `decompose_strategy_returns`,
> `factor_attribution`, `covariance`, `ofi_features`, `basis_report`, `expected_decay`,
> `data_qc`, `universe(as_of)`, extended `evaluate_forecast`). With the container design
> (08), most of these are `tbot` SDK functions first and MCP tools second.

Status: draft proposal. **Exists** = in `INTERNAL_AGENT_TOOLS` today (see 01 §1).
**Extend** = exists but needs changes. **New** = to build. **Blocked** = needs a platform gap
fixed first (gap ids from 01 §10).

---

## 1. Tool design principles

These matter more than the tool list. A hundred badly shaped tools will make the agent
worse, not better.

1. **Answers, not data.** Tools compute server-side and return statistics, not raw
   arrays. `describe_returns` returns kurtosis ± SE; it does not return 5,000 bars for the
   LLM to eyeball. Raw data only on explicit request, paginated and downsampled.
2. **Handles for everything large.** Datasets, feature matrices, simulated paths, signal
   series, model predictions and charts are stored as **artifacts** (`art_…` ids). Tools
   take and return handles, so `simulate_paths → run_study(null=art_…)` never moves data
   through the context window.
3. **Progressive disclosure.** Every read tool takes `detail: summary | standard | full`.
   The default `summary` is ≤ ~1.5 KB, the budget `DiagnosticBundle.text` already uses.
4. **Dual-channel output.** `{ for_model: "<compact text>", for_user: {chart|table spec},
   artifacts: [...] }`. The harness shows the LLM only `for_model`; the UI renders
   `for_user`.
5. **Every number carries its epistemics.** Fields for `n`, standard error or CI,
   `window`, `as_of`, and a class: `fact` (data) · `estimate` (statistic) ·
   `exploration` (hypothesis evidence, not counted as a result) · `result` (sealed Set J) ·
   `model_output` (inference).
6. **Jobs for anything slow.** Anything over ~10 s is `start_* → job_id`. One generic
   `wait_for_jobs(ids, timeout)` is intercepted by the harness at zero token cost
   (generalising today's `wait_for_backtest`/`run_sweep` intercepts), plus
   `get_job`/`cancel_job`.
7. **Errors that teach.** Validation errors name the field, the rule and a fix, like the
   strategy validator does today.
8. **Holdout-aware by construction.** Any analysis tool called with an `experiment_id`
   clips its window to that Experiment's in-sample data and logs the call to the
   Experiment's **exploration ledger**. The agent cannot peek at the holdout through a
   side door.
9. **API first.** Every tool wraps a platform `/api/*` route, so the UI, external MCP
   clients and the agent share one capability (ADR-0022).
10. **Loaded by group.** The full catalogue will pass 100 tools, too many for one context
    (and far too many for local models). Tools live in groups the harness loads by mode
    and phase, plus a `load_toolset(group)` meta-tool (04 §5).

---

## 2. Catalogue

### A. Session and meta (harness-provided, always loaded)

| Tool | Status | Purpose |
|---|---|---|
| `update_plan(steps)` | New | Maintain the plan checklist in `ResearchState` |
| `record_hypothesis(archetype, statement, rationale, null, kill_criterion)` | New | Pre-registration (behaviour rule 4) |
| `record_finding(claim, evidence_refs, confidence)` | New | Atomic cited finding → notebook + research memory |
| `ask_user(question, options, default, timeout)` | New | Pause for a decision that belongs to the user |
| `load_toolset(group)` / `unload_toolset(group)` | New | Token-efficient tool loading |
| `wait_for_jobs(ids, timeout)`, `get_job`, `cancel_job` | New (generalises existing intercepts) | Zero-token waits |
| `spawn_subagent(role, brief, budget)` | New | Analyst / Author / Critic sub-agents (04 §6) |
| `final_report(...)` | New (replaces `FINAL:` parsing) | Structured, citation-checked termination |

### B. Knowledge and memory (see 05)

| Tool | Status | Purpose |
|---|---|---|
| `search_knowledge(query, kinds?)` | New | Hybrid search over concept cards, platform how-tos, archetypes, playbook |
| `get_card(id)` | New | Full card |
| `search_research_memory(query, instrument?, archetype?, gate?)` | New | Past findings, hypotheses and verdicts across sessions |
| `get_instrument_profile(instrument, timeframe)` | New | Cached stylized facts, regime history, costs, coverage |
| `get_authoring_guide(section?)` | Extend | Split into sections instead of one blob |

### C. Discovery and live data

| Tool | Status | Purpose |
|---|---|---|
| `list_instruments`, `get_instrument`, `list_asset_classes`, `list_lanes` | Exists | — |
| `list_initialized_assets`, `init_asset`, `get_asset_init_job` | Exists | Onboard a symbol |
| `get_bars(…, detail, downsample)` | Extend | Add downsampling, return a handle for full data |
| `search_instruments(query, asset_class?)` | New | "Find me liquid large-cap crypto" |
| `get_data_coverage(instrument)` | New | Per timeframe/source: first, last, gaps, revisions |
| `get_live_snapshot(instrument)` | New | Last mark, bid/ask/spread, 24h range, volume, last bar age |
| `get_trades_summary(instrument, window)` | New | Aggregated trade flow: signed volume, trade-size distribution |
| `get_orderbook_snapshot(instrument, depth)` | New | L2 snapshot (lane exists) |
| `get_funding_rates(instrument, window)` | New/Blocked (collector?) | Perp carry |
| `get_option_chain(underlying, expiry?)` | New/Blocked (chain collector) | Tradier chain with IV/Greeks computed server-side |
| `get_prediction_markets(query)` | New | Kalshi contracts: price → implied probability, volume |

### D. Market analysis (server-side compute; the Q&A engine)

All are exploration-class and holdout-aware when given an `experiment_id`.

| Tool | Status | Returns (summary) |
|---|---|---|
| `describe_returns(instrument, tf, window)` | New | Mean/vol/skew/kurtosis ± SE, Jarque–Bera, Ljung–Box on r and r², ARCH-LM, Hill tail index, max drawdown, jump share (RV−BV)/RV |
| `volatility_report(instrument, tf, window)` | New | Close-close, Parkinson, Garman–Klass, Rogers–Satchell, Yang–Zhang, RV, BV; EWMA/GARCH/HAR forecasts; percentile of current vol |
| `regime_report(instrument, tf)` | New (FEAT-003 Ph3) | Current regime + probabilities (filtered only), occupancy, transitions, Hurst (returns and log-RV), variance ratios, ADX |
| `test_stationarity(series_ref)` | New | ADF + KPSS together, with interpretation |
| `test_cointegration(a, b, window)` | New | Engle–Granger (correct critical values), Johansen, OU half-life of the spread |
| `correlation_report(instruments, tf, window)` | New | Rolling correlation, beta, Ledoit–Wolf covariance, Marchenko–Pastur noise band |
| `seasonality_report(instrument, tf)` | New | Hour/weekday effects with multiple-testing correction (BH/Romano–Wolf) |
| `event_study(condition_expr, instrument, horizons)` | New | Forward-return path after condition events vs matched random entries, block-bootstrap bands, event count |
| `distribution_compare(series_a, series_b)` | New | KS, Wasserstein, quantile deltas: "is this month different?" |
| `liquidity_cost_report(instrument)` | New | Fees, Roll and Corwin–Schultz spreads, Amihud, square-root impact at sizes |
| `find_analog_periods(instrument, as_of, k)` | New | kNN over standardised regime-feature vectors: "when did the market last look like this, and what happened next?" |
| `compute_features(instrument, tf, window, features)` | New (Blocked on G-02 for parity) | Feature matrix handle + summary |
| `fit_process(kind, instrument, window)` | New | GBM, OU, GARCH(1,1)-t, GJR, Merton jump, Hawkes on large-move times, HMM/jump-model: params + CIs + goodness of fit → process artifact |
| `simulate_paths(process_ref, n, horizon, seed)` | New | Path artifact for `SyntheticPaths` studies and process nulls |
| `run_notebook(code, inputs)` | New (decision D3) | Sandboxed Python over PIT-clipped handles (see §3) |

### E. Strategy authoring

| Tool | Status | Purpose |
|---|---|---|
| `validate_strategy`, `create_strategy`, `get_strategy`, `list_strategies`, `list_compatible_strategies` | Exists | — |
| `list_features(detail)` | New | The feature registry, with formulas, warm-up and **which paths support each** (backtest/train/serve) |
| `explain_strategy(slug)` | New | Graph → plain English + features + params + archetype guess |
| `diff_strategies(a, b)` | New | Structural diff |
| `propose_move(base, move, hypothesis, ranges)` | New (FEAT-003 Ph2) | Typed structural edit, validated → new versioned slug |
| `preview_signals(slug, experiment_id)` | New | Signal counts/frequency on in-sample only; catches "never fires" before a sweep |
| `signal_forensics(slug, experiment_id)` | New | IC decay across horizons, turnover, overlap with benchmarks (momentum, buy-and-hold, EMA cross); also de-dup |

### F. Evaluation (Set J)

| Tool | Status | Purpose |
|---|---|---|
| `create_experiment`, `get_experiment`, `list_experiments` | Exists | — |
| `run_sweep`, `get_sweep`, `cancel_sweep`, `get_carried_forward` | Exists | — |
| `run_study`, `list_studies`, `get_diagnostics` | Exists | — |
| `get_null_picker`, `choose_null`, `get_funnel`, `advance_gate` | Exists | — |
| `get_surface(study_id)` | New (spec'd) | `SurfaceSummary` on its own |
| `run_robustness_suite(experiment, preset)` | New | Preset battery: cost ladder, delay-by-one-bar, sub-periods, neighbourhood, regime-conditional. **Blocked** on G-06 for costs |
| `compare_candidates(exp_a, exp_b)` | New | Sealed-distribution comparison (worst-5%, median, overlap), never point estimates |
| `get_equity_curve(run_id, points)` | New | Downsampled; chart goes to `for_user` |
| `explain_trade(run_id, trade_idx)` | New | Entry-time feature values, regime, path, MAE/MFE |
| `benchmark_run(experiment, kind)` | New | Buy-and-hold, random-entry-matched and simple-momentum baselines on the same slice |
| `create_backtest` (raw) | Exists (MCP only) | **Keep excluded** from the agent: raw runs bypass counting |

### G. Models (Blocked on G-02 and G-03 for "use in an algo"; training itself is buildable now)

| Tool | Status | Purpose |
|---|---|---|
| `list_models`, `get_model` | Exists | — |
| `list_model_templates()` | New | Kinds × frameworks × targets that are actually trainable, with default hyperparameters |
| `create_dataset(instrument, tf, window, features, target, horizon, labeler)` | New | PIT dataset artifact with label intervals, overlap/purge metadata and a leakage self-check |
| `start_training(dataset, template, cv, hpo_budget)` | New | Job → model version. HPO trials counted on a **model trial counter** |
| `evaluate_model(model, dataset?)` | New | Scorecard from `scoring.py`: CRPS/pinball/PIT/coverage/Kupiec, Diebold–Mariano vs naive and GBM baselines |
| `compare_models(a, b)` | New | Paired loss differentials with HAC standard errors |
| `feature_importance(model, method)` | New | Purged MDA / clustered importance (not MDI) |
| `predict(model, instrument, window)` | New | As-of inference series → artifact (what the model would have said each bar) |
| `build_meta_labeler(strategy, experiment)` | New | Primary-strategy events → triple-barrier labels with side → classifier → `Sizing`/filter node (04 in 06) |
| `promote_model(model, alias)` | New, human-approved | Alias changes affect live strategies; UI approval required |

### H. Text: news, social, web (Blocked on a news collector for news; social exists)

| Tool | Status | Purpose |
|---|---|---|
| `search_social(instrument, query, as_of, window)` | New | Milvus semantic search, PIT-filtered on `available_time` |
| `search_news(...)` | New/Blocked | Same, once a news source exists |
| `text_signal_series(instrument, kind, window)` | New | Post volume, novelty and sentiment series as a feature handle |

### I. Portfolio and paper (read-only)

| Tool | Status |
|---|---|
| `get_dashboard_rollup`, `get_paper_activity`, `get_trading_status`, `get_order` | Exists |
| `get_positions`, `get_paper_vs_backtest(strategy)` | New (the second is **Blocked** on G-11) |

### J. Deployment (proposal only)

| Tool | Status | Purpose |
|---|---|---|
| `propose_paper_deployment(strategy, experiment_id)` | New | Creates a pending request a human approves in the UI. Requires a vaulted Experiment. The agent never arms anything itself. |

---

## 3. The sandbox question (decision D3)

Pre-built tools will never cover the long tail ("what's the correlation of BTC weekend
returns with Monday volume?"). A human quant opens a notebook. Options:

| Option | Upside | Downside |
|---|---|---|
| A. No sandbox; keep adding tools | Safe; every number goes through reviewed code | The agent hits walls; slow to add capabilities |
| B. Sandboxed Python (NumPy/Polars/SciPy/statsmodels/arch), no network, CPU/RAM/time limits, data only through PIT-clipped handles | Covers the long tail; the agent can prototype new analyses that later become tools | Security surface; numbers from unreviewed code |
| C. SQL-only sandbox (DuckDB over artifact views) | Smaller attack surface, very token-efficient | Weak for statistics and modelling |

**Recommendation: B, with guardrails.** Runs in a separate container (the model-trainer
image already has the stack), sees only handles the harness passes in (clipped to the
Experiment's in-sample window when inside an Experiment), outputs are exploration-class
artifacts, and every execution is logged. Analyses the agent runs repeatedly are
candidates for promotion into first-class tools, so the sandbox doubles as the tool
roadmap's discovery mechanism.

## 4. Token budget per tool (targets)

| Class | `for_model` summary | Notes |
|---|---|---|
| Discovery / metadata | ≤ 600 B | tables as compact TSV |
| Analysis reports | ≤ 1.5 KB | numbers ± SE, one-line interpretation |
| Set J results | ≤ 1.5 KB | sealed distribution + ledger line + trial count |
| Diagnostics | ≤ 1.5 KB (exists) | drill down by section |
| Lists | top-k + total count + handle | never unbounded |
| Errors | ≤ 400 B | field, rule, fix |
