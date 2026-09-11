# BS-007 / 08 — Agent-first reassessment: what to keep, change, or rebuild in the code

**Status:** draft proposal, 2026-09-10. **Supersedes** the modes in 02 §3, the harness plan
in 04, and the build order in 07 wherever they conflict.

**Decisions taken (user, 2026-09-10):**
- Frontier models first (Claude-class).
- The agent gets a sandbox, meaning its own container, like a coding agent.
- Local embeddings.
- **One agent, optimised for long-running work.** Questions are just short tasks, and there
  are no separate modes.
- **The platform is built for the agent.** The manual UI is a byproduct: a viewer and
  editor over the same APIs the agent uses.

**Question this doc answers:** should we keep extending the current code shape with
docs and tools, or change the services themselves so they feed the agent best?

---

## 0. Verdict in one table

| Subsystem | Today (see 01) | Verdict | One-line reason |
|---|---|---|---|
| Collectors, ClickHouse, four-timestamp PIT model | Solid | **Keep, extend** | The foundation is right; reach and history depth are the problems |
| Data access for the agent | `get_bars` JSON, capped | **Rebuild** as a Data API that returns Arrow/Parquet into the agent's workspace | A coding agent should analyse data in its sandbox, not in its context |
| Features | 3 disagreeing implementations, backtest = EMA/RSI | **Rebuild** as one engine (Rust + Python bindings) | Parity by construction; unlocks every archetype |
| Strategy representation | Frozen 1-comparison grammar, JSON node graph, 1 action | **Replace** with two layers: code strategies for research, Strategy Language v2 for deployment | The agent writes code best; deployment needs a safe, compiled form |
| Backtest engine (market_simulator / Nautilus) | Full engine behind a market-orders-only facade | **Keep the engine, widen the facade** | Stops, brackets, portfolio, ~40 indicators and fill models already exist upstream |
| Backtest ↔ strategy integration | Stateless `evaluate_signals`; RunConfig fields ignored | **Rebuild** | Use `StrategyInstance` (live-parity), honour cost/fill/seed/version, add batch and position-series runs |
| Evaluation (Set J + FEAT-003 sweep) | Strongest subsystem | **Keep the core, change the enforcement point** | With a sandboxed agent, the holdout must be enforced by the data layer, not the prompt |
| Model training | 7 fixed adapters, tabular only, spec lacks granularity | **Rebuild the interface**, keep the bundle idea | Dataset-as-artifact, granular specs, more families, bring-your-own-model jobs |
| Model use in strategies | Inert in backtests, unwired live | **Rebuild** around walk-forward prediction series | The only honest way to backtest a model-driven strategy |
| Agent harness (Rust driver loop) | 38 tools, transcript-as-memory, no caching/resume | **Replace** with an agent runtime in a per-project container | This is a coding-agent problem; don't rebuild Claude Code in Rust |
| MCP server | Thin HTTP client over `/api/*` (ADR-0022) | **Keep**, alongside a Python SDK and CLI | MCP for external clients; SDK/CLI for code in the sandbox |
| Jobs / long-running work | Ad hoc per feature (backtest manager, sweeps in memory) | **Build** one durable job service | Backtests, sweeps, training, datasets and simulations all share one lifecycle |
| Live strategy execution | Unwired | **Separate track** | Needed for forward evidence; not on the agent's critical path |
| UI | Page per feature | **Re-centre** on the agent workspace | Humans watch, steer, approve, and inspect artifacts |

**Recommendation: change the code, but don't rewrite the platform.** The data plane, the
engine and Set J are assets. What's missing is agent-facing contracts: data out as files,
strategies as code, one feature engine, durable jobs, a real sandbox, and a real harness.

---

## 1. The target shape

```
┌──────────────────────── Agent workspace container (one per research project) ──────────────────────┐
│  Agent runtime (Claude Agent SDK or Managed Agents self-hosted worker, §2)                         │
│  Built-ins: bash · read/write/edit · grep/glob · subagents · skills · compaction · hooks          │
│  /workspace (git): research/*.py  strategies/*.py|*.tsl  models/*.py  reports/  data/ (parquet)   │
│  .claude/skills/  (quant knowledge, platform how-tos)       memory/  (findings index)            │
│  Python: polars numpy scipy statsmodels arch sklearn lightgbm xgboost torch duckdb plotly         │
│  tbot SDK + CLI  ·  tbot_features (PyO3 build of the Rust feature engine)                         │
│  Token scope: research:* only  ·  egress: platform API + package mirror only                      │
└──────────────┬─────────────────────────────────────────────────────────────────────────────────────┘
               │ HTTPS (SDK/CLI) and MCP (same /api/* underneath)
┌──────────────▼──────────────────────────── Platform (Rust) ──────────────────────────────────────┐
│ Data API (Arrow/Parquet, PIT, holdout-enforced) │ Feature engine │ Job service │ Artifact store │
│ Backtest service (market_simulator, batch, position-series) │ Set J evaluation │ Model service  │
│ Research memory (Postgres + local-embedding Milvus) │ Session orchestrator (containers, budgets,  │
│ SSE events to UI, approvals, steering)                                                          │
└──────────────┬───────────────────────────────────────────────────────────────────────────────────┘
               │
   Collectors → NATS → ClickHouse / Postgres        Trainer + inference sidecars (GPU jobs)
```

**The principle behind every change: enforce rules where the agent can't bypass them.**
A coding agent with bash can compute anything on any data it can read. So:
- **The holdout is enforced by the Data API.** The agent's token cannot read data past
  its project's research cutoff. Only the vault gate, a platform service, evaluates on
  the holdout.
- **Trials are counted by the backtest/evaluation service.** Every submission counts,
  whatever client sent it.
- **Authority is enforced by token scope.** Research tokens can't reach order,
  automation or alias-promotion endpoints; those stay human-approved.
- Prompts and skills teach good behaviour; the platform guarantees the invariants.

---

## 2. Harness: replace, don't extend

The current Rust driver (`crates/api/src/agent/driver.rs`) is a minimal tool loop. What
you now want (a container, files, code, background jobs, sub-agents, compaction, skills,
resumable long runs) is exactly what a coding-agent harness already is. Rebuilding
that in Rust is a large project that wouldn't beat existing harnesses.

| Option | What you own | Upside | Downside |
|---|---|---|---|
| **A. Claude Agent SDK** (Claude Code as a library) inside the project container you host | Container image, orchestration, tools/SDK, hooks | Built-in bash/files/grep, subagents, skills, compaction, hooks (PreToolUse/Stop for policy), MCP; runs fully on your machine | You host and scale the containers; Claude-only harness |
| **B. Claude Managed Agents** with a **self-hosted sandbox** | Agent config (versioned), environment worker, custom tools | Anthropic runs the loop; sessions, SSE events, memory stores, multi-agent, session budgets, scheduled deployments; tool execution stays on your infra | Beta; the loop runs in Anthropic's cloud; less control over loop internals |
| **C. Keep the custom loop**, add caching, compaction, context editing, tool search, programmatic tool calling via the Messages API | Everything | Provider-neutral (keeps `crates/llm` OpenAI/Ollama); full control | You build and maintain a coding-agent harness from scratch |

**Recommendation: A**, with B as the alternative if you'd rather not operate the loop.
Both put the agent in a real workspace. A keeps everything local and gives hooks for
policy, and matches "frontier first, own container". Disclosure: I'm Claude, and A and B
are Anthropic products. C is the provider-neutral path if avoiding lock-in matters more
than time to capability. `crates/llm` stays either way for `LlmInference` strategy nodes
and model adapters.

What the platform still owns (in Rust), whichever option is chosen:
- **Session orchestrator:** create or resume a project container, mount its workspace
  volume, mint a scoped token, stream events to the UI, relay steering messages and
  approvals, enforce dollar/compute budgets, snapshot on stop.
- **Policy hooks** (option A): a PreToolUse hook denies commands that touch secrets or
  unapproved egress. A Stop hook runs the `final_report` validator (citations exist,
  numbers match artifacts).
- **Skill materialiser** (12 §5): at every session start, for every user, resolve
  core > global > user > project skills. Write up to about 40 descriptions and a
  registry-generated `GLOSSARY.md` into a read-only `.claude/skills/`, and verify hashes.
  Revocations propagate to live sessions.
- **Skill admission** (12 §4): the agent can only *propose* skills. Verification
  (replay on synthetic instruments, an isolated reviewer, quant tests) and admission are
  platform-side writes, and the glossary row is written in the same transaction.
- **Retire** `driver.rs`/`prompt.rs` once parity is reached, and retire the MCP
  draft-builder tools, since a coding agent writes files.

### 2.1 One agent, long-running by default

- Every conversation belongs to a **research project**: a persistent workspace plus
  research memory, bound to an instrument set or theme. A quick question is a short turn
  in a project. The agent has the same tools either way and simply stops early.
- The agent keeps a living `NOTEBOOK.md` (plan, hypotheses, findings, candidates) in the
  workspace. It replaces the harness-owned `ResearchState` from 04 §3: a coding agent
  maintains a file better than a bespoke JSON channel, and the UI renders the file.
- Sessions resume after restarts because state lives in the workspace, research memory
  and the job service, not in a process.

### 2.2 Long-running tools

Everything slow is a **platform job**: backtest, sweep, study, gate advance, dataset build,
feature materialisation, training, batch inference, simulation, backfill.

```
tbot jobs submit <spec.json>        → job_id (idempotent: same manifest hash → same job)
tbot jobs wait <id> [--timeout 600] → blocks, prints compact status/result summary
tbot jobs watch <ids…> &            → background; notifies on completion
tbot jobs list / get / cancel / logs
```

- **Durable:** manifest, state, heartbeats, progress, artifacts and structured errors live
  in Postgres. Workers survive restarts. Sweep and suite state currently held in memory
  moves here.
- **The agent keeps working while jobs run:** it backgrounds a wait, starts the next
  hypothesis's data prep, and is notified on completion (the Agent SDK runs background
  shell tasks; Managed Agents streams events).
- **Budgets:** each job carries estimated and actual compute cost, charged to the project
  budget the UI shows.
- **Queues:** a separate queue class for agent jobs, with priority and fair-share against
  human jobs, replacing the fixed 3-concurrent backtest cap.

---

## 3. Data reach: the Data API and new sources

### 3.1 Data API v2 (agent-first)

| Endpoint (SDK: `tbot.data.*`) | Returns | Notes |
|---|---|---|
| `bars(instr, tf, start, end, as_of, kind=time\|volume\|dollar)` | Parquet → `/workspace/data/…` | Any timeframe resampled server-side from 1m; information-driven bars from trades |
| `trades`, `quotes`, `book_snapshots` | Parquet | Where collected |
| `funding`, `open_interest` | Parquet | Where a source exists |
| `option_chain(underlying, as_of)` | Parquet with IV/Greeks | From snapshots (§3.2) |
| `prediction_markets(query, as_of)` | Parquet | Kalshi |
| `text(instrument?, query?, as_of)` | Parquet + embeddings search | Social, news, filings |
| `fundamentals(ticker, as_of)` | Parquet | Filing-date PIT |
| `macro(series, as_of)` | Parquet | Vintage-correct |
| `catalog()`, `coverage(instr)` | Small JSON | Discovery: what exists, gaps, revisions |
| `features(spec, instr, tf, window)` | Parquet | Same engine as backtests (§4) |

- **Every call is PIT and scope-checked.** `as_of` filters on `available_time`, and the
  project's research cutoff clips `end`. Responses carry a manifest (query, hash, row
  counts, cutoff applied) so any number is reproducible.
- Parquet goes to the workspace. Only a short summary goes into the model's context.
- MCP tools become thin wrappers that return summaries plus a file path or handle.

### 3.2 Sources to add, in value order (verify each vendor's terms and limits)

| # | Source | Gives the agent | Why it matters |
|---|---|---|---|
| 1 | **Deep history backfill** for existing crypto/equity instruments (years of 1m/1h/1d) | CPCV, walk-forward and regime work that's actually possible | Today BTC 1h had 22 bars (G-13). Nothing else matters if this is missing. |
| 2 | **Alpaca News API** (Benzinga-sourced; equities and crypto; historical) and **Alpaca corporate actions** | News with timestamps; splits/dividends | Credentials already exist; covers "news fed live" honestly |
| 3 | **SEC EDGAR** (companyfacts/XBRL, submissions; free) | Fundamentals and filing events, PIT by filing date | Stock Q&A and fundamental factors |
| 4 | **FRED / ALFRED** (free; vintages) | Rates, inflation, macro, vintage-correct | Regime and macro conditioning without look-ahead from revisions |
| 5 | **Tradier option chain snapshots** (with Greeks if the API provides them) stored forward | IV surface, skew, term structure history | Options research; volatility-risk-premium signals |
| 6 | **Kalshi history** (already collected live) exposed | Event probabilities | Prediction-market research (06 A13) |
| 7 | Crypto funding/OI from a US-accessible venue | Carry, crowding | Check geo-restrictions (see the Binance 451 note in memory) |

---

## 4. One feature engine

- **Rust crate as the single source of truth** (`crates/features` grows), wrapping the
  Nautilus indicators (~40: SMA/EMA/HMA/WMA/VWAP/DEMA/AMA/VIDYA, MACD, RSI, Stochastics,
  Bollinger, CCI, Aroon, Ichimoku, ATR, Donchian, Keltner, variance ratio, efficiency
  ratio, OBV, book imbalance …) plus the platform's own (Parkinson, Garman–Klass,
  Rogers–Satchell, Yang–Zhang, realised measures, Hurst, rolling stats, calendar).
- **Two execution modes, one definition:** incremental (streaming, used in backtests and
  live) and batch (vectorised over a window, used for datasets and the Data API). A parity
  test asserts they produce identical values.
- **Expression-defined features:** `rolling_std(logret(close), 20) / rolling_std(logret(close), 100)`,
  written in the same expression language as Strategy Language v2 (§5), content-hashed,
  registered to a project or global library. The agent invents features without
  platform code changes.
- **Python bindings (PyO3)** installed in the agent container and the trainer. Research,
  training, backtest and serving call the same code. The Python `features.py` and the API
  `features_compute.rs` are deleted, which fixes G-01 and G-02.
- **Warm-up and PIT metadata per feature** (lookback, finalisation), enforced by the
  engine rather than documented.

---

## 5. Strategy representation: two layers

### 5.1 Why change it

The frozen grammar (one comparison, no functions, no lookback, no state, one action)
caps research at EMA/RSI entries. The JSON node graph is built for a drag-and-drop UI, not
for an agent that writes code fluently. Adding node kinds one by one won't fix that;
the expression model itself is too small.

### 5.2 Layer 1: research strategies as code (position series)

The agent writes a pure function in its workspace:

```python
# strategies/vol_breakout.py
from tbot.strategy import research_strategy, Param

@research_strategy(params={"lookback": Param.int(10, 100), "k": Param.float(0.5, 3.0)})
def positions(data, p):
    rv = data.feature("rolling_std(logret(close), 20)")
    rng = data.feature(f"rolling_max(high, {p.lookback}) - rolling_min(low, {p.lookback})")
    signal = (data.close > data.close.shift(1) + p.k * rv * data.close) & (rng.pct_rank(250) < 0.2)
    return signal.astype(float) * target_vol_scale(rv, 0.4)  # target position in [-1, 1]
```

- It is submitted as a Run of kind `position_series`. The **backtest service executes the
  positions in the engine** with an explicit execution policy (decide at bar close $t$,
  fill at $t{+}1$ open by default, costs from the cost model). The agent never computes
  its own P&L.
- **Mechanical leakage check (truncation test):** the service re-runs the function on data
  truncated at random points $k$. Positions before $k$ must be identical. Any
  difference means look-ahead, and Gate 0 fails. This check is only possible because
  the strategy is a function.
- **Sweeps stay honest:** the platform's sampler calls the function with sampled params
  inside a sandboxed runner. The agent still never picks numbers (ADR-0023) and never sees
  an argmax (INV-2).
- Anything computable (models, text features, cross-asset inputs) is usable immediately,
  because it's just code over PIT data.

### 5.3 Layer 2: Strategy Language v2 (deployable)

Deployable strategies need to be safe, validated, fast, sweepable and identical in
backtest and live. Keep a compiled declarative form, but make it a real language. The
agent writes text; the platform parses it to an AST (the canonical JSON the UI renders and
edits), then compiles it to the existing bytecode VM (`crates/strategy-runtime/src/bytecode.rs`).

```text
strategy vol_breakout_v3 on $instrument timeframe 1h
params  lookback: int[10..100] = 40,  k: float[0.5..3.0] = 1.2,  tv: float[0.1..0.8] = 0.4
inputs  rv   = rolling_std(logret(close), 20)
        rng  = rolling_max(high, lookback) - rolling_min(low, lookback)
        f    = model("vol_forecaster@production").sigma      # typed model output
entry long  when close > close[1] * (1 + k * rv) and pct_rank(rng, 250) < 0.2 and flat
size        vol_target(tv, f)  clamp 0..1
exit        stop   2.0 * atr(14) from entry
            target 4.0 * atr(14) from entry
            time   48 bars
risk        max_position 1.0   max_orders_per_min 6
```

Language features, most of which map directly onto Nautilus capabilities:
- Full feature engine library, lookback indexing `x[k]`, math functions, `crossed_above`, `pct_rank`.
- State: `position`, `flat`, `entry_price`, `bars_in_trade`, `unrealized_return`, `equity`.
- **Exits** (stop, target, trailing, time; in %, ATR or σ units) compile to bracket/OCO
  orders in the engine.
- **Sizing:** fixed, fraction of equity, risk-per-trade, volatility-target, Kelly fraction,
  model-driven, with clamps.
- **Orders:** market, limit (offset), stop; time-in-force.
- **Typed model calls** returning `InferenceOutput` fields, with an abstain policy.
- Multi-timeframe inputs; later, multi-instrument references for pairs and portfolios
  (lifting OQ-040 is a separate decision).
- Existing v1.x JSON definitions translate mechanically into the v2 AST, so nothing
  already saved breaks.

**Research → deployment parity:** promoting a Layer 1 strategy means writing its Layer 2
version. The platform reconciles the two position series on the same data (Set J's
`reconcile` module is the natural home) and blocks promotion below a match threshold.

### 5.4 Node kinds

Most "new node kinds" become **functions** in the language (indicators, transforms,
model calls) rather than graph node types. The AST keeps a small set of structural
constructs: inputs, params, signals/entries, exits, sizing, risk, model calls,
universe/rank. The UI builder becomes an editor over that AST: a byproduct, as intended.

---

## 6. Backtest system

Keep market_simulator. Change the integration:

| Change | Where | Fixes |
|---|---|---|
| Widen `sdk.rs`: limit/stop/stop-limit/trailing/bracket/OCO commands, multi-instrument, quotes/trades data, fill-model and fee-model presets, latency | market_simulator `crates/backtest/src/sdk.rs` (you own both repos) | G-04, G-06 |
| Drive backtests with **`StrategyInstance`** (bytecode, inference slots, sizing), not stateless `evaluate_signals` | `crates/backtest/src/sim.rs` | Backtest/live parity; G-03 |
| **Honour `RunConfig`**: `cost_model_ref` → fee/slippage/spread model (data-driven per instrument), `fill_model` (close vs next open), `seed`, `strategy_version` hash check, `data_snapshot` pin | `sim_executor.rs` | G-06, G-14; makes `CostSweep`/`Seeds` studies real |
| **Batch runs:** load bars and features once, evaluate N param sets | backtest service | Sweeps get much faster; racing becomes cheap |
| **Position-series run kind** (§5.2) | backtest service + Set J | Layer 1 research |
| **Fast vectorised tier** for fidelity tier 0, calibrated against the event engine (FEAT-003 P5 rank-correlation gate) | new | Cheap screening without trusting it blindly |
| **Rich outputs as artifacts:** trades, fills, positions, equity, and feature values at every decision | Arrow → artifact store (ClickHouse `backtest_run_*` tables exist) | `explain_trade`, MAE/MFE, diagnostics |

---

## 7. Models: more types, granular specs, bring your own

### 7.1 Dataset as a first-class artifact

```yaml
DatasetSpec:
  universe: [BTC-USD, ETH-USD]        # or a universe query
  timeframe: 1h
  window: { start, end }              # clipped by research cutoff
  features: [ "ema(close,21)/close - 1", "yang_zhang(20)", "hurst(logret(close),256)", … ]
  transforms: { winsorize: 0.01, scaler: per_fold_standard, lags: [1,2,4] }
  sampling: every_bar | cusum(h=2σ) | strategy_events(strategy_ref)
  target:   forward_return(h=24, log) | realized_vol(h=24) | triple_barrier(pt=2σ, sl=1σ, max=48)
            | meta_label(strategy_ref) | rank(h=24)
  weights:  uniqueness | time_decay(half_life=180d) | none
  cv:       walk_forward(train=365d, test=30d, refit=30d) | purged_kfold(k=5) | cpcv(n=6,k=2)
            purge/embargo derived from label horizon (not hand-set)
```

The dataset is materialised as content-hashed Parquet with label intervals and
purge/embargo metadata. The agent can load it in its sandbox, and the trainer uses the
same bytes. A PIT self-check (the leakage harness) runs at build time.

### 7.2 Model families to add

| Family | Examples | Role |
|---|---|---|
| Baselines | ridge/elastic-net, historical mean, EWMA, HAR | Must be beaten; cheap |
| GBMs (exist) | LightGBM/XGBoost point and quantile, LambdaRank | Workhorse tabular and ranking |
| Volatility | GARCH/GJR/EGARCH (with **conditional serving fixed**), HAR-RV, realised GARCH | Sizing and risk |
| Regime | HMM (filtered), statistical jump model | FEAT-003 Phase 3 |
| Sequence | real-window LSTM/GRU, TCN, DLinear, N-HiTS, PatchTST, TFT, TSMixer (e.g. via Nixtla `neuralforecast`, Apache-2.0) | Only where they beat GBM on lagged features |
| Probabilistic | NGBoost, quantile regression forests, conformalised wrappers (ACI) | Calibrated uncertainty |
| Zero-shot | Chronos/TimesFM-class foundation models | Baselines, with a contamination label |
| **Bring your own** | Any agent-written `fit/predict` class conforming to the model interface | The coding agent's main leverage |

### 7.3 Bring-your-own-model contract

```python
class Model(tbot.models.ModelInterface):
    output = "quantiles"                    # typed → InferenceOutput fields
    def fit(self, ds: Dataset, fold: Fold) -> None: ...
    def predict(self, window: FeatureWindow) -> InferenceOutput: ...
```

The agent submits `models/my_tcn.py` plus a `DatasetSpec` as a training job. It runs in
the trainer container (GPU if available) with the code hash recorded, is scored by the
existing `scoring.py` against baselines, and is registered through the bundle contract.
HPO trials count on a model-trial counter.

### 7.4 Using models in strategies honestly: walk-forward prediction series

A model trained once on 2024–2026 can't be used to backtest 2025. The canonical path:
1. A training job with `cv: walk_forward(refit=…)` produces one model per fold.
2. Out-of-fold predictions are concatenated into a **prediction series** artifact
   (the ClickHouse `model_predictions` table exists), where every prediction was made by a
   model trained only on earlier data.
3. Backtests read that series as a feature stream. Gate 0 rejects any prediction whose
   producing model's training window overlaps its timestamp.
4. Live/paper uses the latest refit through the inference gateway, making the live model
   the last step of the same process.

This fixes G-03 in a way that stays honest, and gives meta-labeling (06 B10) a clean
implementation.

---

## 8. Evaluation (Set J): keep, and adapt to a coding agent

Keep INV-1/2/3, the null library, the funnel and `SelectionRule`. Change:
- **Holdout enforcement moves to the Data API** (research cutoff per project). The vault
  gate is the only reader of post-cutoff data.
- **Run kinds:** `definition` (Layer 2), `position_series` (Layer 1), both counted identically.
- **Model trials counted**, and a model's Experiment links to the strategy Experiments that use it.
- **Effective number of trials** (cluster trials by return correlation) feeds DSR, so
  near-duplicate sweeps don't make significance unreachable, nor cheap.
- **Exploration ledger:** Data API reads and sandbox jobs inside a project are logged and
  reported with every verdict ("12 exploratory analyses preceded this hypothesis").
- **Persistence and parallelism:** suite and sweep state moves into the job service; the
  funnel stops running under the suite lock.
- **Forward stage:** once live evaluation is wired, add a paper-trading gate whose
  pass/fail is backtest-consistency of paper fills.
- **Added by 11:** a theme-posterior (empirical-Bayes shrinkage toward the archetype prior)
  reported alongside DSR; specification counting (features, interactions, transformations);
  a `post_hoc` flag for hypotheses registered after exploration; a `data_qc` grade gate;
  ES backtests (Acerbi–Szekely) before any strategy sizes on `var_95`/`es_95`; and the
  12-part candidate dossier (11 §4) as the Gate 3 → vault entry requirement.

---

## 9. Knowledge (restating 05 for a coding agent)

- **L1 reference knowledge → Skills and files**, not a vector store first. The quant cards
  (06), platform how-tos and the archetype/playbook tables live as `.claude/skills/*` and
  markdown in the workspace. Skill descriptions sit in context; bodies load on demand; the
  agent greps the rest. For a coding agent this usually beats vector RAG on a corpus this size.
- **Local embeddings** where semantic search is really needed: L2 research findings across
  projects (thousands over time) and L4 news/social/filings. Serve a local embedding model
  and version vectors by model. The existing `apps/embedder` switches from OpenAI to the
  local model.
- Findings are still written as atomic, cited claims (05 §4). The agent writes them via
  `tbot memory add`, and the Stop hook requires them for every tested hypothesis.
- **Agent-created skills (12).** Procedures the agent learns the hard way become skills:
  - triggered by repetition, a hard-won verified outcome, a pitfall, or a human request;
  - admitted only if a replay shows equal success at lower cost;
  - never containing findings or tuned values.
  The skill registry is the single source of the glossary. Core and global skills reach
  every user's sessions.

---

## 10. Build order (replaces 07)

| Phase | Deliverables | Unblocks |
|---|---|---|
| **0 Contracts (design, ADRs)** | Data API v2, job/artifact model, token scopes + research cutoff, Strategy Language v2 grammar + AST, position-series run kind, DatasetSpec/ModelInterface, harness choice (A/B/C) | Everything; lets work proceed in parallel |
| **1 Agent foundation** | Project container image; session orchestrator + SSE; scoped tokens; job service; artifact store; Data API (bars/trades/coverage) with PIT + cutoff; `tbot` SDK/CLI; MCP wrappers; workspace UI v1; eval-suite skeleton (noise + planted-edge synthetic instruments) | The agent can research in its sandbox today, with honest data access |
| **2 Engine truthfulness** | Unified feature engine + PyO3; `StrategyInstance` in backtests; `RunConfig` honoured; fill timing verified; widened SDK facade (orders, exits, sizing); batch runs; position-series runs + truncation test | Real strategy space; Layer 1 research |
| **3 Models** | DatasetSpec artifacts; new families; BYO-model jobs; walk-forward prediction series; GARCH conditional serving | Model-driven strategies, meta-labeling |
| **4 Strategy Language v2** | Parser → AST → bytecode; exits/sizing/orders/model calls; v1 → v2 translator; research→deploy reconciliation; UI editor over the AST | Deployable agent strategies |
| **5 Data reach** | Deep backfill (start in Phase 1 as ops work); Alpaca news + corporate actions; EDGAR; FRED/ALFRED; option-chain snapshots; Kalshi exposure | Richer hypotheses; stock Q&A |
| **6 Knowledge + autonomy** | Skills library (12 §11 spreads skills work across phases 1–6: core skills in Phase 1, registry and materialiser in 2–3, `skill_verify` and project admission in 4, promotion and curator in 6); research memory with local embeddings; FEAT-003 campaigns/regime engine inside the new harness | Compounding research across sessions |
| **Track** | Live strategy evaluation wiring (G-11) → forward/paper gate | Deployment evidence |

Deep history backfill (source 1) should start immediately as an operational task. It's
the cheapest high-value item and every phase needs it.

## 11. Risks

| Risk | Mitigation |
|---|---|
| Scope: this is a re-platforming of several services | Contracts first (Phase 0); keep engines and Set J; phase gates with the eval suite |
| A sandboxed agent bypasses honesty rules | Enforcement at data, evaluation and token layers (§1), not prompts |
| Sandbox security on a Windows/Docker/WSL dev box | Rootless containers, egress allowlist, no secrets in the container; note the Docker phantom-socket issue in memory |
| Harness lock-in (option A/B) | Tools live behind the platform API/SDK/MCP, so the harness is swappable; option C stays viable |
| Frontier-model cost on long runs | Prompt caching, compaction, files-not-context, cheaper subagents, per-project dollar budgets |
| Code strategies drift from deployable ones | Mandatory research→deploy reconciliation before promotion |
| Self-made skills degrade the agent, leak alpha or carry injections | Experience-only triggers; platform-side verification with an isolated reviewer; PIT, noise and leakage tests; ~40-skill cap in context; human review for global (12) |
