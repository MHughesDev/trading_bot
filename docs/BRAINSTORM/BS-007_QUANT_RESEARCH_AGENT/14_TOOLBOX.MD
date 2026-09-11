# BS-007 / 14 — Toolbox: the `tbot` SDK/CLI and the MCP surface

**Spec status:** spec-ready draft · **Req prefix:** TB- · **Formal spec:** AGENT-002
(supersedes the tool list in INTG-001; ADR-0022's thin-client principle is kept) ·
**Plan set:** spread across L–R · **Decisions:** D-02, D-05, D-07, D-11

---

## 1. Principles

These matter more than the list. A hundred badly shaped tools make the agent worse.

1. **SDK/CLI first.** Inside the container, the agent reaches the platform through the
   `tbot` Python SDK and CLI from bash and code. There are no tool schemas in context.
   The agent chains calls in code, and only final results reach the context.
2. **API first.** Every capability is a platform `/api/*` route. The SDK, CLI, MCP
   server and UI are all generated from, or thin over, the same routes (ADR-0022). MCP
   stubs are generated from the API definitions, so the surfaces never drift.
3. **Answers, not data.** Analysis runs server-side or in the agent's code. Results are
   statistics with n, SE or CI, window and `as_of`, never raw arrays. Raw data goes to
   files (06 §2).
4. **Handles for everything large** (`art_…`, 05 §6). Commands take and return handles.
5. **Progressive disclosure:** `--detail summary|standard|full` (default summary), and
   `--json` for machine use.
6. **Epistemic class on every number:**
   - `fact` (data);
   - `estimate` (statistic);
   - `exploration` (hypothesis evidence);
   - `result` (sealed Set J);
   - `model_output` (inference).
   The report validator checks the classes (B-02).
7. **Jobs for anything slow** (over ~10 s): `submit → job_id`, then event-driven `wait`
   or `watch` (05 §5).
8. **Errors that teach:** field, rule and fix, ≤ 400 B.
9. **Holdout-aware by construction:** data and analysis calls are clipped at the cutoff
   and logged to the exploration ledger (06 §3).
10. **A stable surface per session:** nothing is loaded or unloaded mid-session (04 §3).
    For MCP clients, all tools are declared, most are deferred, and they're found through
    tool search.
11. **Namespaced and human-readable:** return symbols and slugs alongside ids.

## 2. Surfaces

| Surface | Who uses it | Notes |
|---|---|---|
| `tbot` Python SDK | The agent's code, Layer 1 strategies, BYO models, skills | Typed; returns dataframes and handles |
| `tbot` CLI | The agent in bash; humans | Compact text by default (§4 budgets) |
| In-process tools (`agent-host`) | The agent | `ask_user` only (03 §4) |
| MCP server (`crates/mcp-server`) | External MCP clients | Generated stubs; namespaced (`data_bars`, `exp_sweep`, …); deferred loading; ≤ 15 always loaded |
| REST + SSE | UI, SDK, MCP | The single source of capability |

## 3. Catalogue

**Status:**
- **Exists** — in `INTERNAL_AGENT_TOOLS` today;
- **Extend** — exists but changes;
- **New** — to build;
- **Retire** — goes away.

**Phase** refers to 17. The **Spec** column is the owning doc.

### 3.1 Jobs, artifacts, reports, proposals (05, 03)

| Command | Status | Phase | Spec |
|---|---|---|---|
| `tbot jobs submit / get / list / wait / watch / cancel / logs` | New (generalises `wait_for_backtest`, the `run_sweep` intercept, asset-init jobs) | 1 | 05 |
| `tbot art get / show` | New | 1 | 05 |
| `tbot report submit` | New (replaces `FINAL:` parsing) | 1 | 03 §10 |
| `tbot proposals create --kind paper_deployment\|model_promotion\|budget\|qc_waiver` | New (paper deployment after G-11) | 1 / Track | 02 §5 |
| `ask_user(question, options, default, timeout)` (in-process) | New | 1 | 03 §4 |

### 3.2 Data and discovery (06)

| Command | Status | Phase |
|---|---|---|
| `tbot data catalog / coverage / instruments / instrument` | Extend (`list_instruments`, `get_instrument`, `list_asset_classes`, `list_lanes`) | 1 |
| `tbot data search "<query>" [--asset-class]` ("liquid large-cap crypto") | New | 1 |
| `tbot data bars` (any timeframe; time/volume/dollar/tick) | Extend (`get_bars` → Parquet) | 1 |
| `tbot data trades / quotes / book` | New | 1 (forward data) |
| `tbot data features` | New | 2 |
| `tbot data universe <name> --as-of` | New | 1 |
| `tbot data qc <instr> <tf> <window>` | New (`data_qc` job) | 1 |
| `tbot data live <instr>` (mark, bid/ask, 24h range, last-bar age) | New; **Desk projects only** (06 §3.1) | 1 |
| `tbot data funding / oi / option-chain / prediction-markets / fundamentals / macro` | New | 5 |
| `tbot assets init / init-status` | Exists (`init_asset`, `get_asset_init_job`) | 1 |

### 3.3 Market analysis: the Q&A engine (reviewed server-side code; class `exploration` or `estimate`)

| Command | Returns (summary) | Phase |
|---|---|---|
| `tbot analysis describe-returns` | Mean/vol/skew/kurtosis ± SE, Jarque–Bera, Ljung–Box (r, r²), ARCH-LM, Hill tail index, max drawdown, jump share | 1 |
| `tbot analysis volatility` | PK/GK/RS/YZ, RV/BV/RQ, EWMA/GARCH/**HARQ** forecasts, current-vol percentile | 1 |
| `tbot analysis regime` | Filtered regime and probabilities, occupancy, transitions, Hurst, variance ratios, ADX | 1 (features) / 3 (models) |
| `tbot analysis event-study <condition>` | Forward-return path after events vs matched random entries, block-bootstrap bands, event count | 1 |
| `tbot analysis liquidity-cost` | Fees, **EDGE** spread (Roll/CS cross-check), Amihud, √-impact at sizes | 1 |
| `tbot analysis correlation` | Rolling correlation, beta, **NLS covariance**, Marchenko–Pastur band | 1 |
| `tbot analysis seasonality` | Hour/weekday effects with BH/Romano–Wolf correction | 1 |
| `tbot analysis stationarity / cointegration` | ADF+KPSS; Engle–Granger (correct critical values), Johansen, OU half-life | 1 |
| `tbot analysis compare-distributions` | KS, Wasserstein, quantile deltas | 1 |
| `tbot analysis analogs` | Explicit-vector kNN with an exclusion zone (12 §7) | 6 |
| `tbot analysis fit-process / simulate-paths` | GBM, OU, GARCH-t, GJR, Merton jump, Hawkes, HMM/JM → process artifact; path artifacts | 3 |
| `tbot analysis fit-regime-model --kind jm\|cjm\|sjm\|hmm` | Filtered labels artifact | 3 |
| `tbot profile <instr> <tf>` | Instrument profile (12 §7) | 1 |

The agent can compute anything else in its own code. These commands exist so that common
numbers come from reviewed code with SEs, and cheaply.

### 3.4 Features (07)

| Command | Status | Phase |
|---|---|---|
| `tbot features list [--family]` (formula, warm-up, supported paths) | New | 2 |
| `tbot features register "<expr>" --name` | New | 2 |
| `tbot_features` (PyO3) | New | 2 |

### 3.5 Strategies (08)

| Command | Status | Phase |
|---|---|---|
| `tbot strategy validate / compile <file.tsl>` | Extend (`validate_strategy`) | 4 |
| `tbot strategy save / get / list / versions` | Extend (`create_strategy` upsert → immutable versions) | 4 |
| `tbot strategy translate-v1 <slug>` | New | 4 |
| `tbot strategy explain / diff` | New | 4 |
| `tbot strategy preview-signals --experiment` (in-sample counts, frequency) | New | 2 |
| `tbot strategy forensics --experiment` (IC decay, turnover, overlap with benchmarks, de-dup) | New | 2 |
| `tbot strategy reconcile <layer1.py> <layer2.tsl>` | New | 4 |
| `tbot strategy move <base> --move <typed-move> --hypothesis` (typed structural edit → new version; FEAT-003 Phase 2 move vocabulary) | New | 4 / 6 |
| `@research_strategy` (SDK) | New | 2 |

### 3.6 Backtests and evaluation (09, 11)

| Command | Status | Phase |
|---|---|---|
| `tbot bt run <strategy> --experiment` (definition or position series; experiment required for agent tokens) | Extend (`create_backtest`, MCP-only today → counted job) | 2 |
| `tbot bt outputs / equity-curve / explain-trade` | New | 2 |
| `tbot exp hypothesis register / list / get` | New | 1 |
| `tbot exp create / get / list` | Exists (`create_experiment`, …) | 1 |
| `tbot exp sweep / sweep-status / cancel / carried-forward` | Exists (`run_sweep`, …) | 1 |
| `tbot exp study / studies / diagnostics / surface` | Exists / New (`get_surface`) | 1 |
| `tbot exp null-picker / choose-null / funnel / advance-gate` | Exists | 1 |
| `tbot exp robustness / compare / benchmark` | New | 2 |
| `tbot exp alpha-chain / family-test / capacity / posterior / decompose / factors / collider / decay / vs-random` | New (11 §8) | 2–6 |
| `tbot exp dossier build / show` | New (11 §5) | 2 |

### 3.7 Datasets and models (10)

| Command | Status | Phase |
|---|---|---|
| `tbot models list / get` | Exists | 1 |
| `tbot models templates` | New | 3 |
| `tbot datasets build <spec.yaml> / show` | New | 3 |
| `tbot models train <model.py\|template> --dataset [--hpo n]` | New | 3 |
| `tbot models evaluate / compare / importance` | New | 3 |
| `tbot models predict-series --model --cv` | New | 3 |
| `tbot models calibrate --method aci\|aci_cusum` | New | 3 |
| `tbot models meta-labeler <strategy> --experiment` | New | 3 |
| `tbot proposals create --kind model_promotion` | New, human-approved | 3 |

### 3.8 Text: reader scope only (06 §7)

| Command | Status | Phase |
|---|---|---|
| `tbot text search-social / search-news / filings` (PIT) | New | 5 |
| `tbot text signal-series` (volume, novelty, sentiment from extractions) | New | 5 |
| `tbot text contamination-probe` | New | 5 |

### 3.9 Memory and skills (12, 13)

| Command | Status | Phase |
|---|---|---|
| `tbot memory add / search / get` | New | 1 (add) / 6 (semantic) |
| `tbot skills search / show / glossary / propose / patch / vote / deprecate` | New | 1 (read) / 4–6 |

### 3.10 Portfolio (read-only)

| Command | Status | Phase |
|---|---|---|
| `tbot portfolio rollup / paper-activity / trading-status / order / positions` | Exists / New (positions) | 1 |
| `tbot portfolio paper-vs-backtest <strategy>` | New (blocked on G-11) | Track |

### 3.11 Retired

| Item | Why |
|---|---|
| MCP draft-builder tools (`new_strategy_draft`, `add_*`, `finalize_strategy`) | A coding agent writes files; SLv2 replaces them |
| `get_authoring_guide` blob embedded in the prompt | Becomes core skills (13 §5) |
| `load_toolset` / `unload_toolset` (old proposal) | Would break caching (04 §3) |
| `wait_for_backtest`, the driver's `run_sweep` intercept | Replaced by jobs `wait` and `watch` |
| `update_plan`, `record_finding` meta-tools (old proposal) | NOTEBOOK/RESEARCH_PLAN files plus `tbot memory add` and hypothesis registry |
| `spawn_subagent` (old proposal) | SDK sub-agents (03 §9) |
| Automations tools | Never in any agent scope (D-12) |

## 4. Output budgets (CLI default `summary`)

| Class | Budget | Format |
|---|---|---|
| Discovery / metadata | ≤ 150 tokens | Compact TSV |
| Analysis report | ≤ 400 tokens | Numbers ± SE, one-line interpretation, handle |
| Set J result | ≤ 400 tokens | Sealed distribution plus ledger line plus trials and effective N |
| Diagnostics | ≤ 400 tokens per section | Drill down by section |
| Lists | Top-k + total count + handle | Never unbounded |
| Errors | ≤ 100 tokens (≈ 400 B) | Field, rule, fix |
| Hard cap (any output) | 8k tokens | The output guard spills to a file (03 §7) |

Summaries MUST carry every number needed for the next decision (04 §6).

## 5. Requirements

| ID | Requirement | Pri |
|---|---|---|
| TB-01 | Every capability MUST exist as an `/api/*` route before any SDK, CLI, MCP or UI exposure | MUST |
| TB-02 | The SDK and CLI MUST cover §3 for the phases listed; the MCP stubs MUST be generated from the same definitions | MUST |
| TB-03 | Outputs MUST follow §1.3–1.6 and the §4 budgets, with `--detail` and `--json` | MUST |
| TB-04 | Every numeric output MUST carry an epistemic class | MUST |
| TB-05 | Agent-token backtests MUST require an `experiment_id`, and are counted (INV-1) | MUST |
| TB-06 | Text commands MUST require the reader scope | MUST |
| TB-07 | `tbot data live` MUST be available only in Desk projects (06 §3.1) | MUST |
| TB-08 | The MCP server MUST use namespaced names and deferred loading, with ≤ 15 always-loaded tools | MUST |
| TB-09 | §3.11 items MUST be removed once their replacements ship | MUST |
| TB-10 | Analysis commands MUST return SE or CI for every estimate and state the window and `as_of` | MUST |

## 6. Acceptance criteria

1. A Q&A task ("30-day realised vol of ETH and its percentile over 2 years") completes
   with one `tbot analysis volatility` call; the output is ≤ 400 tokens and includes SEs.
2. The MCP tool list and the SDK are generated from one definition. A CI check fails on
   drift.
3. An agent-token backtest without an experiment id is refused with a teachable error.
