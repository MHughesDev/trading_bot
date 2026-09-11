# BS-007 / 01 — Ground truth: what the platform exposes today

**Audited:** 2026-09-10 against `main` (HEAD `9f71f22`, with uncommitted changes in
`crates/config` and `apps/platform/src/main.rs`).
**Rule for this brainstorm:** the capabilities the platform actually exposes (MCP tools, API
routes, and the code behind them) are the truth. Everything else, including the external
ChatGPT pack (`Quant_Platform_Agent_Context.md`) and older specs, is input to be checked.
Every claim below cites a path. Re-verify before building on any of it.

**Role in the spec set:** this is the as-built baseline. Spec docs 02–16 cite its gap IDs
(G-01…G-14) and sections. The "what the agent cannot do" list and the harness description
(§8) describe *today*. 03_RUNTIME and 14_TOOLBOX replace them.

---

## 1. The agent's only window: the MCP tool layer

The internal agent and external MCP clients share one tool implementation
(`crates/mcp-server/src/lib.rs` → `dispatch_tool`). Tools are thin HTTP clients of the
platform API (ADR-0022). The in-app agent calls them over loopback with a run-scoped
service token (`crates/api/src/agent/driver.rs`).

**42 tools in the MCP profile. 38 in `ToolProfile::InternalAgent`** (`lib.rs:352`).

| Group | Tools (internal profile unless marked) | What they really give the agent |
|---|---|---|
| Protocol | `get_authoring_guide` | The authoring guide text (`crates/mcp-server/docs/authoring_guide.md`) |
| Discovery | `list_lanes`, `list_instruments`, `list_asset_classes`, `get_instrument` | Instrument metadata, per-timeframe bar coverage |
| Data | `get_bars`, `list_initialized_assets`, `init_asset`, `get_asset_init_job` | OHLCV (size-capped) + summary stats; background backfill of a new symbol |
| Strategy | `validate_strategy`, `create_strategy`, `get_strategy`, `list_strategies`, `list_compatible_strategies` | Full-JSON authoring with structured validator errors; `strategy_id` is an **upsert key** |
| Draft builder | `new_strategy_draft`, `add_*`, `finalize_strategy` … (**MCP only**) | Step-by-step authoring for weaker clients |
| Backtests | `get_backtest`, `wait_for_backtest`, `list_backtests`, `compare_backtests`; `create_backtest`, `rerun_backtest`, `stop_backtest`, `delete_backtest` (**MCP only**) | The internal agent cannot create raw backtests (FEAT-003 §11); it must go through Experiments |
| Research (Set J / FEAT-003) | `create_experiment`, `list_experiments`, `get_experiment`, `run_sweep`, `get_sweep`, `cancel_sweep`, `run_study`, `list_studies`, `get_carried_forward`, `get_diagnostics`, `get_funnel`, `advance_gate`, `get_null_picker`, `choose_null` | Sealed, counted evaluation: sweeps, studies, nulls, gates |
| Models | `list_models`, `get_model` | **Read-only.** No training, evaluation or inference tool exists |
| Portfolio (read-only) | `get_dashboard_rollup`, `get_paper_activity`, `get_trading_status`, `get_order` | Account and paper observability |
| Automations | `list/create/arm/disarm_automation` (**MCP only**, live arming env-gated) | Not available to the in-app agent (by design) |

**What the agent cannot do at all today:** train or evaluate a model, run inference, compute
any statistic beyond `get_bars` summary stats, read news or social data, see live
quotes/marks, look at order books, funding rates, option or prediction-market data, run
custom analysis code, remember anything across runs, or ask the user a question
mid-run.

---

## 2. Data the platform has (and whether the agent can reach it)

| Source | Code | Stored/streamed as | Agent-reachable? |
|---|---|---|---|
| Crypto spot (Kraken live trades → 1m bars) | `crates/collectors/src/crypto/kraken.rs`, continuous 1m aggregation | ClickHouse bars | Yes (`get_bars`) |
| Equities (Alpaca data) | `collectors/src/equity/alpaca_data.rs` | bars/quotes | Bars only |
| Options (Tradier per-contract 1m bars + quotes) | `collectors/src/options/tradier.rs` | bars/quotes | No chain/IV/Greeks anywhere |
| Futures (Tradovate), FX (OANDA) | `collectors/src/futures`, `fx` | bars/quotes | Bars only |
| Prediction markets (Kalshi) | `collectors/src/prediction/kalshi.rs` | `prediction.price` | No |
| DEX (0x quotes) | `collectors/src/dex/zerox.rs` | `dex.quote` | No |
| Reddit posts, web pages | `collectors/src/social/reddit.rs`, `web/scraper.rs` | NATS → `apps/embedder` → Milvus (`trading_social_posts`, OpenAI `text-embedding-3-small`, 1536-d) | No tool |
| News | `DataType::NewsArticle` declared (`crates/domain/src/data_type.rs`) | No news collector | No |
| Funding rates | `DataType::MarketFundingRate` declared | ? | No |

**Point-in-time time model exists and is good.** Every event carries four timestamps;
`available_time = max(window_close, observed_time) + watermark + processing_delay` is the
replay sort key (`crates/domain/src/timestamp.rs:48`). This is the platform's filtration
$\mathcal{F}_t$ in code.

**History depth is the binding constraint.** At the 2026-09-08 runtime check, BTC-USD had
dense 1m bars for roughly May 16 to Jun 17 2026 and only 22 bars at 1h (FEAT-003 memory
note). Many quant methods in this brainstorm need years. Check `list_instruments` before
assuming anything.

---

## 3. What a strategy can express

Canonical JSON definition (`crates/domain/src/strategy_def/`). Versions: 1.0 frozen core,
1.1 AI nodes, 1.2 typed `parameters` (materialised to literals before validation), 1.5
universe nodes.

**Expression grammar (frozen):** `number | feature('name') | bar('open|high|low|close|volume')`
with `+ - * /` and one comparison. No functions, no lookback (`bar()` is the current bar),
no state (entry price, bars in trade, position), no `abs/min/max/log`. Crosses come from
the Signal node's rising-edge semantics.

**Node kinds (14):** `condition`, `signal`, `data_source`, `rank`, `filter`, `take_top_n`,
`surface_action`, `model_forecast`, `inference`, `sizing`, `decision`, `model_rank`,
`llm_inference`, `combine` (`nodes.rs:163`).

**Actions (1):** `place_order { side, size_mode: fixed | percent_of_balance | risk_unit | model, size }`
(`actions.rs`). No stop, target, trailing, time-exit or bracket actions.

**Frontend builder vs backend:** the palette offers EMA, SMA, RSI, ATR, stop loss, take profit
and trailing stop (`frontend/src/components/strategy/Palette.tsx`). SMA/ATR produce a
"not supported by the backtest engine" warning, and exits are dropped from the saved
definition with a warning (`frontend/src/utils/toDefinition.ts:288`).

**Single instrument per strategy, single asset class per instance** (OQ-040 / C-062). Pairs,
spreads and portfolios cannot be expressed.

---

## 4. How a strategy actually runs

### 4.1 Backtest: the only strategy-execution path wired end to end

`SimRunExecutor` (`crates/backtest/src/sim_executor.rs`) → looks up the definition by slug →
materialises params → `derive_requirements` → loads ClickHouse bars with warm-up →
`run_simulation_detailed` (`crates/backtest/src/sim.rs`) on the market_simulator SDK
(Nautilus fork, pinned git rev, `Cargo.toml:147`).

Per bar, the handler (`sim.rs:564–614`) updates indicators on the close, evaluates the node
graph with the **stateless** `strategy_runtime::evaluate_signals`, and submits market
orders on a signal's rising edge. Fees come from the venue preset. The Nautilus engine nets
positions; closed ones are harvested from snapshots.

| Limitation | Evidence | Consequence for the agent |
|---|---|---|
| Backtests compute **only `ema_N` and `rsi_N`** | `requirements.rs:41` error text; `sim.rs:552` builds only EMA/RSI state | The structural search space is two indicators plus current-bar fields |
| Only `size_mode: fixed` | authoring guide; simulator rejects others | No volatility targeting, Kelly, or model sizing can be tested |
| AI nodes are inert | handler calls `evaluate_signals(nodes, features, bars)` with no model inputs; no `ForecastProvider`/`model_results` symbol exists in the repo any more | A strategy with model nodes backtests as if the model always abstained |
| `RunConfig.cost_model_ref`, `fill_model`, `sizing_ref`, `seed`, `data_snapshot`, `strategy_version` are not read | 0 references in `sim_executor.rs` | `CostSweep` and `Seeds` studies over the real executor produce identical members; the version pin is recorded but not enforced, and slugs are overwritable |
| Fill timing unverified | — | Whether a market order sent in the bar handler fills at that bar's close or the next bar's open decides whether P&L is a proper (non-anticipating) Itô sum. **Open question.** |
| MAE/MFE reported as 0.0 | FEAT-003 memory note (`position_to_trade` TODO) | Diagnostic bundle's excursion stats are blank |
| 1m annualisation | FEAT-003 memory note | Sortino/Calmar explode at 1m; use expectancy / profit factor there |

### 4.2 Live and paper strategy execution: not wired

- `apps/platform/src/hot_path.rs:306`: `let mut strategy: Option<StrategyInstance> = None;`
  ("placeholder for demand-manager integration"). Stage 3 never evaluates anything.
- `InstanceManager::dispatch` (`crates/strategy-runtime/src/runtime.rs:514`) has no non-test
  caller. Neither do `StrategyInstance::set_inference_outputs` or
  `InferenceGateway::refresh_node_forecasts`.
- **Works:** manual paper orders (Kraken marks → `/api/orders` → risk gate → paper engine).

The stateful `StrategyInstance` itself is good: compiled bytecode conditions over a slot
array, inference-output slot binding with abstain policies, `Sizing` overrides, `Decision`
class maps, and replay-determinism tests. It is simply not driven by anything outside tests.

---

## 5. Features: three implementations that disagree

| Path | Code | Computes | Unknown names |
|---|---|---|---|
| Backtest | `crates/backtest/src/requirements.rs`, `sim.rs` | `ema_N`, `rsi_N` | Rejected (fail closed) |
| Rust feature registry | `crates/features/src/feature_sets.rs`, `training_frame.rs` | ~35 names: OHLCV, EMA, RSI, rolling mean/std, returns, log return, Parkinson, Garman–Klass, momentum, z-score, relative volume, OBV, hour/day-of-week sin/cos | Rejected |
| Python trainer | `apps/model-trainer/app/features.py` | OHLCV, EMA, RSI, rolling mean/std, `returns_N`, `log_returns_1` | **Silently skipped** (the column is just missing) |
| API Test Lab / serving vector | `crates/api/src/features_compute.rs` | Same as the Python trainer | **Zero-filled** |

A model requested on `fs_extended_v1` through the Python trainer silently trains without
Parkinson, Garman–Klass, z-score and relative volume. The bundle's `feature_order` records
what was actually trained, and serving fills anything it can't compute with 0. Parity
holds only where the names happen to match.

---

## 6. Models

- **Kinds:** `forecaster`, `signal_ranker`, `trade_decision`, `risk_sizing`, `embedding`,
  `external_llm_adapter` (`crates/domain/src/model_def/kinds.rs`).
- **Adapters** (`apps/model-trainer/app/adapters/`): XGBoost, LightGBM and sklearn (point and
  quantile variants), torch (`mlp`, `lstm`), GARCH(1,1)-t via `arch`.
- **The LSTM is not a sequence model in practice.** `LSTMNet.forward` does `x.unsqueeze(1)`
  on 2-D input (`torch_adapter.py:30`), and training frames are tabular rows, so the
  sequence length is 1. **There is no Transformer, TFT, TCN or N-BEATS anywhere in the
  repo** (grep, 2026-09-10).
- **Targets:** `return | price | volatility | direction | action | score | size_fraction | move_size`,
  transform `none | logret | zscore`, horizon token (`target.rs`). The default label is the
  forward simple return (`features.py:83`).
- **Labelers:** triple barrier (close path only, vol-scaled), quantile, devolatized (`labelers.py`).
- **Validation:** ordinal train/val/test with an embargo derived from the label horizon;
  walk-forward folds; in-fold Optuna HPO on CRPS with the trial count persisted (`hpo.py`).
- **Scoring is strong** (`scoring.py`): pinball, CRPS from quantiles, log score, PIT + KS,
  interval coverage, reliability, Kupiec POF, Christoffersen conditional coverage,
  naive / seasonal-naive / zero-shot-empirical baselines, per-fold and per-regime scores.
- **Serving contract:** `InferenceOutput { direction, confidence, median_return, sigma,
  quantile_levels, quantiles_return, var_95, var_99, es_95, skew, spread_90, size_fraction,
  action_class }` (`strategy_def/inference_output.rs`). Fields bind into `feature('…')` slots.
- **Monitoring:** PIT-KS calibration drift, PSI feature drift, staleness → retrain trigger
  (`crates/model-registry/src/quality_monitor.rs`).
- **GARCH serving caveat:** inference falls back to *unconditional* σ when live bars are
  unavailable (adapter docstring). In practice the model can serve long-run vol, not its
  conditional forecast.

---

## 7. The evaluation engine (Set J + FEAT-003 Phase 1): the strongest part of the platform

| Object | What exists (`crates/backtest/src/…`) |
|---|---|
| Run | Content-hashed `RunConfig` (`run/config.rs:130`) |
| Study kinds (10) | `ParameterSweep`, `WalkForward`, `Cpcv`, `NestedCv`, `PermutationNull`, `SyntheticPaths`, `CostSweep`, `TradeMonteCarlo`, `RegimeConditional`, `Neighborhood` |
| Selection | `None`, `MedianStableCentroid`, `WorstCaseRobust`. **No argmax, by design (INV-2).** |
| Nulls (7) | `SignalReturnDecouple`, `BlockPermutation`, `StationaryBootstrap`, `BarPermutation`, `SyntheticGarch`, `RegimeBlock`, `RandomEntryMatched`, plus `recommend_null(strategy_type)` |
| Gates | G0 Integrity → G1 SinglePath (walk-forward) → G2 Robustness (CPCV + synthetic worst-5%) → G3 Significance (primary null p-value + trial counter; DSR ≥ 0.95 and PBO ≤ 0.5 as corroborators) → G4 Vault |
| Statistics | permutation p, selection-bias correction, PSR, expected max of N Gaussians, DSR, PBO (CSCV) (`stats/mod.rs`) |
| Objective | primary ∈ {Sortino, Calmar, ProfitFactor, Expectancy, DetrendedSharpe, Sharpe}, constraints (MinTrades default 50, MaxDD …), aggregate Median or Worst5Pct |
| Diagnostics | Trades (n, win rate, avg win/loss, expectancy, PF, hold p50, MAE/MFE p50), by month, by regime (if regime windows are supplied), worst trades, longest drawdown, exposure (`stats/diagnostics.rs`) |
| Search | `crates/research`: typed search space, seeded Random and compact in-house TPE samplers, `SurfaceSummary`, `run_sweep`, fidelity calibration (racing off) |

Not built yet (FEAT-003 Phases 2–3): campaign/outer loop, knowledge-base tables, regime
engine, move vocabulary, correlation de-duplication. Sweep and suite state are in memory.

---

## 8. The harness today (`crates/api/src/agent/`)

- One loop: the system prompt (`prompt.rs`: research protocol + full authoring guide + run
  constraints) and the **whole transcript are resent every turn**.
- Tool results are truncated at 32 KB. When the transcript passes 400k chars, all but the
  6 most recent tool results are elided to `[elided — was N bytes]` (`driver.rs:37–41, 152`).
- Zero-token waits: `wait_for_backtest` and `run_sweep` are intercepted and polled
  in-harness, with status messages.
- Budgets: max iterations, max total tokens, wall clock. Termination is by parsing a
  `FINAL:` text reply (`parse_final`).
- Persistence: `agent_runs` + `agent_messages`. On restart, non-terminal runs are marked
  failed (`recover_orphans`). There is no resume.
- LLM client (`crates/llm`): OpenAI, Anthropic, Ollama; tool calling; **no prompt caching,
  no streaming, no extended thinking** (no `cache_control` in `anthropic.rs`).
- UI (`frontend/src/pages/AgentPage.tsx`, `components/agent/*`): run config, run list,
  timeline with sweep cards, adaptive polling. No mid-run steering, no approvals, no
  notebook/artifact panes.
- No memory across runs. No sub-agents. No planning state beyond the transcript.

---

## 9. Risk and execution (context only; the agent never trades)

- `crates/risk`: position, per-second and per-minute order rate, price-sanity band, tick,
  lot, daily loss, instrument active, session, halt, trust tier, kill switch, tighten-only
  overrides. **Operational risk only**: no volatility, correlation, VaR or drawdown-based
  exposure.
- Paper engines: CLOB book walk, broker quote, AMM (0x firm quote), prediction-market book
  walk (`crates/execution/src/paper/`).

---

## 10. Gap register: what limits the agent, ranked by how much it caps research quality

| ID | Gap | Why it matters to the agent |
|---|---|---|
| G-01 | Backtests support only EMA/RSI | Most hypotheses (volatility, range, regime, calendar, volume) cannot be tested at all |
| G-02 | Three feature implementations disagree | Train/serve/backtest skew; silent zero-fills poison models |
| G-03 | AI nodes inert in backtests | "Train a model and use it in an algo" cannot be evaluated |
| G-04 | Exits and position state missing | Every strategy is entry-signal plus opposing signal; no stop/target/time exit |
| G-05 | Fixed sizing only | Sizing research (the most reliable quant lever) is impossible |
| G-06 | `RunConfig` cost/fill/seed/version ignored | Cost robustness studies are degenerate; runs are not reproducible after slug overwrite |
| G-07 | No training/eval/inference tools | The agent cannot build models in a session |
| G-08 | No analysis tools | Q&A about an instrument means dumping bars into context |
| G-09 | No live snapshot, news, social, options, prediction or funding tools | The agent can't answer "what's happening now" questions |
| G-10 | LSTM sequence length 1; no attention models | "LSTM vs Transformer vs TFT" is not a real choice today |
| G-11 | Live strategy evaluation not wired | Paper/live forward evidence for agent-built strategies is impossible |
| G-12 | Harness: no caching, memory, resume, steering or structured final | Long sessions are expensive and fragile |
| G-13 | Thin history | CPCV, walk-forward and regime work need years of bars |
| G-14 | Fill-timing semantics unverified | Possible look-ahead in every intraday result |

## 11. Invariants any design must keep

- **INV-1:** every Run is counted automatically; trial counters never decrease.
- **INV-2:** distributions are sealed; nothing returns "the best member". Carry-forward is
  by `SelectionRule`, never argmax.
- **INV-3:** no significance claim without its null and trial count.
- **ADR-0023 P1:** the LLM proposes structure and ranges; an optimiser chooses numbers.
- Validator fails closed; risk overrides are tighten-only; money is `Decimal`/decimal strings (ADR-0002).
- No order-placement tool in any agent profile; the internal agent gets no automations.
