# Trading Bot Repository Summary for Claude Opus 4.8

**Date:** 2026-06-02  
**Repository:** trading_bot (Python 3.11+, modular monolith)  
**Primary Language:** Python (Polars, FastAPI, Streamlit, PyTorch)

---

## 1. PROJECT OVERVIEW

**Trading Bot** is a Python **AI-assisted crypto trading platform** with:
- **Market Data Source:** Kraken (WebSocket + REST)
- **Paper Trading Venue:** Alpaca
- **Live Trading Venue:** Coinbase (optional, configured per-symbol)
- **Architecture:** Modular monolith (single deployable process, not microservices)
- **Core Loop:** Kraken WS → normalize → features → decision engine → risk engine → execution

**Key Goals:**
- Research, backtesting, and careful live execution in one codebase
- Shared decision path between live and replay (parity enforced)
- Canonical decision record for auditability (APEX spec compliance)
- Per-symbol configuration (lifecycle, execution mode, models)

---

## 2. ARCHITECTURE LAYERS

### 2.1 Data Plane (`data_plane/`)
- **Kraken Ingest:** WS + REST clients (`ingest/kraken_ws.py`, `kraken_rest.py`)
- **Normalization:** `kraken_normalizers.py` → `TickerSnapshot`, `OrderBookLevel2Snapshot`, `TradeTick`
- **Bars/OHLCV:** `bootstrap_bars.py`, rolling bar aggregation
- **Features:** `FeaturePipeline` (polars-based) computes returns, volatility, microstructure
- **Memory:** Qdrant vector DB for news context, execution feedback EMA
- **Storage:** QuestDB for canonical bars, Redis for cache

### 2.2 Decision Engine (`decision_engine/`)
- **Entry:** `run_step.py:run_decision_tick()` — shared by live (`live_service`) and replay (`backtesting/replay`)
- **Pipeline:** `pipeline.py` → `canonical_orchestrator.py`
- **Canonical Sequence:** structure → state → trigger → auction → carry (FB-CAN-029)
- **State Engine:** regime (5-class APEX: trend/range/stress/dislocated/transition), degradation, heat, novelty, reflexivity
- **Trigger Engine:** 3-stage (setup → pretrigger → confirm), missed-move memory
- **Auction Engine:** opportunity ranking, diversification split (D_corr, D_thesis, D_liq), thesis overlap
- **Policy:** spec-policy-proposal (routes: NO_TRADE, SCALPING, INTRADAY, SWING, CARRY)

### 2.3 Risk Engine (`risk_engine/`)
- **Hard Constraints:** feed stale, spread too wide, drawdown limit, product untradable
- **Precedence:** feed → data timestamp → spread → drawdown → proposal/exposure (see `docs/architecture/risk_precedence.md`)
- **Sizing:** `canonical_sizing.py` computes notional with degradation from novelty/transition/heat
- **Signing:** `signing.py` signs risk-approved `OrderIntent`

### 2.4 Execution (`execution/`)
- **Adapters:** Base class + Alpaca (paper) + Coinbase (live)
- **Router:** `router.py` creates adapter based on mode (paper/live)
- **Service:** `service.py` (ExecutionService) — entry point for order submission
- **Execution Logic:** venue-agnostic intent builder, partial fill reconciliation, PnL tracking
- **Intent Gate:** `intent_gate.py` requires signing before venue submission

### 2.5 Backtesting (`backtesting/`)
- **Replay:** `replay.py:replay_decisions()` walks OHLCV bars through `run_decision_tick`
- **Simulator:** `simulator.py` — slippage, fees, portfolio tracking
- **Fault Injection:** named profiles for stress testing decision logic
- **Provenance:** dataset fingerprint, seed tracking, reproducibility

### 2.6 Control Plane (`control_plane/`)
- **API:** FastAPI (`api.py`) — `/status`, `/routes`, `/params`, `/system/mode`, `/flatten`, `/governance/*`
- **Dashboard:** Streamlit (`Home.py`, `pages/`) — login, asset overview, PnL, settings
- **Auth:** SQLite user DB, Argon2 hashing, HTTP-only session cookies
- **Multi-tenant:** Optional per-user data scoping + venue credentials per user

### 2.7 Observability (`observability/`)
- **Metrics:** Prometheus `tb_canonical_*`, `tb_decision_latency_seconds`, `tb_governance_*`
- **Lag Tracking:** event lag, decision processing, execution feedback EMA
- **Logging:** structlog with context
- **Governance Metrics:** release gates, probation, config drift, session mode, exchange risk

---

## 3. KEY CONTRACTS (TYPE DEFINITIONS)

**Location:** `app/contracts/`

| Contract | Purpose |
|----------|---------|
| `decisions.py` | `RouteId`, `RouteDecision`, `ActionProposal`, `TradeAction` |
| `canonical_state.py` | `CanonicalStateOutput` (regime, heat, novelty, degradation) |
| `canonical_structure.py` | `CanonicalStructureOutput` (forecast views) |
| `orders.py` | `OrderIntent`, `OrderSide`, `OrderType`, `TimeInForce` |
| `risk.py` | `RiskState` (equity, drawdown, hard override, transition count), `SystemMode` |
| `execution_guidance.py` | `ExecutionGuidance`, `ExecutionFeedback` |
| `replay_events.py` | `ReplayRunContract`, `CanonicalEvent`, `ReplayMode` (FB-CAN-055) |
| `decision_snapshots.py` | Typed boundary inputs: `PortfolioSnapshot`, `StructuralSignalSnapshot`, `SafetyRegimeSnapshot` |
| `auction.py` | `AuctionResult` with clustering metadata |
| `trigger.py` | `TriggerOutput` (stage, ISO timestamps, latency, failure codes) |

---

## 4. CONFIGURATION SYSTEM

### 4.1 Settings (`app/config/settings.py`)
- **Mechanism:** Pydantic `BaseSettings` with `NM_` prefix (env vars override YAML)
- **Defaults:** `app/config/default.yaml` (APEX canonical + risk limits + feature config)
- **Load:** `load_settings()` reads `.env`, merges with YAML, validates
- **Key Settings:**
  - `execution_mode` (paper/live)
  - `execution_live_adapter` (coinbase), `execution_paper_adapter` (alpaca)
  - `market_data_symbols` (BTC-USD, ETH-USD, SOL-USD default)
  - Risk limits: `risk_max_total_exposure_usd`, `risk_max_per_symbol_usd`, etc.

### 4.2 Canonical Config (`apex_canonical`)
- **YAML Structure:** `apex_canonical.metadata` (version, name, created_at, created_by, notes, enabled_feature_families, environment_scope)
- **Domains:** `state_safety_degradation`, `auction`, `risk_sizing`, `execution`, `signal_confidence`
- **Gating:** When `NM_EXECUTION_MODE=live` or `NM_CANONICAL_CONFIG_STRICT=1`, `environment_scope` must not be unspecified
- **Feature Families:** market_microstructure, funding, open_interest, basis, cross_exchange_divergence, liquidation_structure, options_context, stablecoin_flow_proxy, execution_feedback, novelty, heat_components

### 4.3 Environment Variables (`.env` not versioned)
```
NM_ALPACA_API_KEY, NM_ALPACA_API_SECRET
NM_COINBASE_API_KEY, NM_COINBASE_API_SECRET
NM_RISK_SIGNING_SECRET
NM_CONTROL_PLANE_API_KEY
NM_AUTH_USERS_DB_PATH, NM_AUTH_VENUE_CREDENTIALS_MASTER_SECRET
NM_EXECUTION_MODE (paper|live)
NM_MARKET_DATA_SYMBOLS (comma-sep)
NM_CANONICAL_CONFIG_STRICT (0|1)
NM_LIVE_DECISION_MIN_INTERVAL_SECONDS
```

---

## 5. DATA FLOW

### 5.1 Live Trading Loop
```
Kraken WS message
  ↓
normalize_kraken_ws_message() → {TickerSnapshot, OrderBookLevel2Snapshot, TradeTick}
  ↓
enrich_bars_last_row() → merge feature overlays (returns, volatility, microstructure)
  ↓
run_decision_tick() [SHARED LIVE + REPLAY PATH]
  - pipeline.step() → state_engine → trigger_engine → auction_engine
  - risk_engine.evaluate() → RiskState → OrderIntent or None
  ↓
build_execution_context_from_decision()
  ↓
ExecutionService.submit_order() → adapter → OrderAck or error
  ↓
apply_execution_feedback() → update EMA, position, equity
  ↓
Optional: persist to QuestDB, emit metrics, log decision record
```

### 5.2 Replay / Backtesting
```
OHLCV bars (Polars DataFrame)
  ↓
replay_decisions(bars, pipeline, risk_engine, ...)
  ↓
[for each bar]
  - enrich_bars_last_row() (same as live)
  - run_decision_tick() (same entry, same logic)
  - Simulated fill (slippage + fees from simulator)
  - apply_execution_feedback() to update memory
  - Append row with decision, trade action, execution feedback
  ↓
List of decision records with full audit trail
```

---

## 6. MODULE RESPONSIBILITIES

| Module | Files | Purpose |
|--------|-------|---------|
| **app** | `config/`, `contracts/`, `runtime/` | Settings, type contracts, live service entry, system state |
| **data_plane** | `ingest/`, `bars/`, `features/`, `memory/`, `storage/` | Kraken ingestion, OHLCV rolling, feature pipeline, Qdrant/Redis, QuestDB writes |
| **decision_engine** | `run_step.py`, `pipeline.py`, `canonical_orchestrator.py`, `state_engine.py`, `trigger_engine.py`, `auction_engine.py` | Shared decision logic, APEX spec orchestration |
| **risk_engine** | `engine.py`, `canonical_sizing.py`, `signing.py` | Hard constraints, sizing with degradation, order signing |
| **execution** | `adapters/`, `router.py`, `service.py`, `execution_logic.py`, `intent_gate.py` | Venue adapters, order submission, partial fill reconciliation |
| **backtesting** | `replay.py`, `replay_core.py`, `simulator.py`, `fault_injection.py`, `portfolio.py` | Replay engine, simulated execution, fault stress testing |
| **control_plane** | `api.py`, `Home.py`, `pages/` | FastAPI routes, Streamlit dashboard, auth, asset mgmt |
| **observability** | `canonical_metrics.py`, `lag_metrics.py`, `monitoring_domain_checklist.py` | Prometheus metrics, lag tracking, domain coverage audits |
| **orchestration** | `app_scheduler.py`, `alpaca_universe_sync.py`, `coinbase_universe_sync.py`, `release_gating.py`, `rollback_validation.py` | Nightly training, universe sync, release gates, rollback playbooks |

---

## 7. KEY ALGORITHMS & DECISION LOGIC

### 7.1 APEX State Engine
- **Regime Classification:** 5-class vector (trend, range, stress, dislocated, transition) with max-confidence separability
- **Degradation Levels:** NORMAL → REDUCED → DEFENSIVE → NO_TRADE
- **Heat Score:** Weighted sum of Hf (funding), Hl (liquidation), Ho (OI), Hx (cross-ex divergence), Hv (volatility), He (execution feedback)
- **Novelty:** Out-of-distribution, HMM ambiguity, structure fragility, transition
- **Reflexivity:** RSI extremes, relative volatility, fragility, directional pressure

### 7.2 Three-Stage Trigger
1. **Setup:** Signal threshold + regime gate
2. **Pretrigger:** Confirmation within timeframe
3. **Confirm:** Final edge check
- **Missed Move Penalty:** P_mm ↓ trigger confidence if recent high/low exceeded signal
- **Output:** Candidate trigger with stage, latency, failure codes

### 7.3 Opportunity Auction
- **Top-N Selection:** rank by edge × confidence × (1 − D_corr) × (1 − thesis_overlap)
- **Diversification:** D_corr (correlation penalty), D_thesis (thesis bucket overlap), D_liq (liquidation stress)
- **Thesis Buckets:** Portfolio positions tracked by thesis key; hard reject on `thesis_overlap_cap`
- **Book Concentration Stress:** Position depth penalty if > `book_concentration_threshold`
- **Output:** Top N candidates, clustering metadata (FB-CAN-034)

### 7.4 Risk Sizing
- **Canonical Notional:** = proposal.size_fraction × edge_budget × market_size × degradation_scalar
- **Degradation Scalar:** novelty × transition × heat weighted per `apex_canonical.domains.risk_sizing`
- **Edge Budget:** Composite of heat, exposure, symbol exposure weights → minimum multiplier 0.35
- **Hard Gates:**
  1. Feed stale (threshold `risk_stale_data_seconds`)
  2. Data timestamp stale
  3. Spread > `risk_max_spread_bps`
  4. Drawdown > `risk_max_drawdown_pct`
  5. Exposure > `risk_max_total_exposure_usd` or per-symbol > `risk_max_per_symbol_usd`

---

## 8. FILE STRUCTURE

```
trading_bot/
├── README.md                          # Quick start, stack overview
├── AGENTS.md                          # Agent/operator contract (binding rules, queue)
├── pyproject.toml                     # Dependencies, package metadata
├── .env.example                       # Template for NM_* settings
├── app/
│   ├── config/
│   │   ├── settings.py               # Pydantic AppSettings loader
│   │   ├── default.yaml              # APEX canonical defaults + risk limits
│   │   ├── canonical_config.py       # CanonicalRuntimeConfig resolution
│   │   ├── signal_confidence.py      # Per-family base/freshness/reliability weights
│   │   ├── shadow_comparison.py      # Paired replay divergence config
│   │   └── ...
│   ├── contracts/                    # Type definitions (pydantic models)
│   │   ├── decisions.py
│   │   ├── canonical_state.py
│   │   ├── orders.py
│   │   ├── risk.py
│   │   ├── replay_events.py
│   │   └── ...
│   └── runtime/
│       ├── live_service.py           # Main trading loop (Kraken WS → decision → exec)
│       ├── event_loop.py             # Asyncio helpers
│       ├── asset_lifecycle_state.py  # Per-asset Initialize/Start/Stop
│       ├── asset_execution_mode.py   # Per-asset paper/live overrides
│       ├── asset_model_registry.py   # Per-asset model manifests
│       └── ...
├── data_plane/
│   ├── ingest/
│   │   ├── kraken_ws.py             # WebSocket client
│   │   ├── kraken_rest.py           # REST client (historical bars, tickers)
│   │   ├── kraken_normalizers.py    # Normalize to TickerSnapshot, TradeTick
│   │   ├── structural_signals.py    # Funding rate, OI, basis, divergence
│   │   └── ...
│   ├── bars/
│   │   └── rolling.py               # RollingBars (time-window bucketing)
│   ├── features/
│   │   ├── pipeline.py              # FeaturePipeline (Polars-based)
│   │   ├── canonical_normalize.py   # Freshness/reliability degradation
│   │   └── ...
│   ├── memory/
│   │   ├── qdrant_memory.py         # Vector DB for news context
│   │   ├── execution_feedback_memory.py  # EMA penalty per symbol
│   │   └── ...
│   └── storage/
│       ├── questdb.py               # Canonical bars persistence
│       └── ...
├── decision_engine/
│   ├── run_step.py                  # run_decision_tick() [SHARED LIVE+REPLAY]
│   ├── pipeline.py                  # DecisionPipeline.step()
│   ├── canonical_orchestrator.py    # FB-CAN-029: structure→state→trigger→auction→carry
│   ├── state_engine.py              # Regime, heat, novelty, reflexivity, degradation
│   ├── trigger_engine.py            # 3-stage trigger with missed-move memory
│   ├── auction_engine.py            # Opportunity ranking + diversification
│   ├── spec_policy_proposal.py      # Route-to-action policy
│   ├── decision_record.py           # DecisionRecord builder + audit logging
│   └── ...
├── risk_engine/
│   ├── engine.py                    # RiskEngine.evaluate() — hard gates + sizing
│   ├── canonical_sizing.py          # compute_canonical_notional() with degradation
│   ├── signing.py                   # sign_order_intent()
│   └── ...
├── execution/
│   ├── adapters/
│   │   ├── base_adapter.py          # ExecutionAdapter ABC
│   │   ├── alpaca_adapter.py        # Paper trading
│   │   ├── coinbase_adapter.py      # Live trading
│   │   └── ...
│   ├── router.py                    # create_execution_adapter()
│   ├── service.py                   # ExecutionService (entry point)
│   ├── execution_logic.py           # Intent builder, guidance prep
│   ├── intent_gate.py               # Signing requirement enforcement
│   ├── partial_fill_reconcile.py    # Partial fill tracking
│   ├── pnl_ledger.py                # PnL tracking
│   └── ...
├── backtesting/
│   ├── replay.py                    # replay_decisions() API
│   ├── replay_core.py               # run_one_replay_step()
│   ├── simulator.py                 # Slippage, fees, portfolio simulation
│   ├── fault_injection.py           # Stress test profiles
│   ├── portfolio.py                 # PortfolioTracker (position + equity)
│   └── ...
├── control_plane/
│   ├── api.py                       # FastAPI app (status, routes, modes, auth, models, governance)
│   ├── Home.py                      # Streamlit landing page
│   ├── pages/
│   │   ├── 0_Login.py              # Session-based login
│   │   ├── Asset.py                # Per-asset detail + lifecycle
│   │   ├── Dashboard.py            # Overview + PnL
│   │   ├── Account.py              # User account settings
│   │   └── ...
│   ├── auth_cookie.py              # Session management
│   ├── chart_stream.py             # SSE real-time bars
│   └── ...
├── observability/
│   ├── canonical_metrics.py        # Prometheus tb_canonical_* gauges/counters
│   ├── lag_metrics.py              # Event/decision/exec latency
│   ├── monitoring_domain_checklist.py  # FB-CAN-056: domain coverage validator
│   └── ...
├── orchestration/
│   ├── app_scheduler.py            # Nightly training scheduler
│   ├── alpaca_universe_sync.py     # Sync tradable universe
│   ├── coinbase_universe_sync.py   # Sync Coinbase products
│   ├── release_gating.py           # Promotion gates (research→sim→shadow→live)
│   ├── rollback_validation.py      # Rollback playbook validator
│   └── ...
├── models/
│   ├── registry/
│   │   ├── active_model_set.py    # Active forecaster + policy loading
│   │   ├── experiment_registry.py # Experiment metadata
│   │   └── ...
│   └── ...
├── tests/
│   ├── test_decision_risk.py       # Decision + risk path
│   ├── test_backtest_live_parity.py  # Replay == live assertion
│   ├── test_canonical_*.py         # Canonical spec compliance
│   ├── test_execution_*.py         # Adapter, router, service
│   ├── test_asset_lifecycle_*.py   # Per-asset state
│   └── ...
├── infra/
│   ├── docker-compose.yml          # QuestDB, Redis, Qdrant, Prometheus, Grafana, Loki
│   ├── docker-compose.app.yml      # Override for in-stack networking
│   ├── docker-compose.microservices.yml  # Optional feature/market data services
│   └── Dockerfile                  # OCI image (control plane + opt Streamlit)
├── scripts/
│   ├── setup.sh / setup.bat        # Clone → venv → Docker → .env
│   ├── run.sh / run.bat            # Start API + supervisor + dashboard
│   ├── ci_spec_compliance.sh       # Enforce Kraken-only market data (no Alpaca data)
│   ├── ci_mlflow_promotion_policy.sh  # Enforce manual MLflow promotion
│   ├── ci_canonical_gates.sh       # Run all APEX spec CI checks
│   ├── ci_canonical_acceptance_audit.py  # FB-CAN-078: acceptance audit JSON
│   └── ...
├── docs/
│   ├── README.MD                   # Landing page (moved; links to sections)
│   ├── QUEUE.MD                    # Queue system narrative + snapshot
│   ├── QUEUE_STACK.csv             # Task queue (task_id, agent_task, affected_files, docs_refs, status)
│   ├── QUEUE_ARCHIVE.MD            # Completed queue items (history)
│   ├── CANONICAL_SPEC_INDEX.MD     # Target APEX architecture
│   ├── CANONICAL_MODULE_MAP.MD     # Code ↔ APEX domain map
│   ├── CANONICAL_GLOSSARY.MD       # Legacy vs. canonical naming (FB-CAN-040)
│   ├── CANONICAL_TOMBSTONE_INDEX.MD  # Removed paths (commit links)
│   ├── GOVERNANCE_RELEASE_AND_EXPERIMENTS.MD  # Release gates, experiments, rollback
│   ├── MONITORING_CANONICAL.MD     # Prometheus metrics, alert thresholds
│   └── architecture/
│       ├── system_walkthrough.md   # High-level data flows
│       ├── risk_precedence.md      # Risk block order
│       ├── kraken_market_data.md   # Kraken-specific normalization
│       ├── coinbase_granularity.md # Coinbase product metadata caching
│       └── ...
├── .claude/
│   ├── settings.json               # Claude Code harness config
│   └── skills/
│       ├── add-to-queue/SKILL.md  # Add task to queue
│       ├── audit-report-to-queue/SKILL.md
│       └── ...
├── .github/
│   └── workflows/
│       └── ci.yml                  # GitHub Actions: lint, test, spec compliance, Docker, gitleaks
└── Dockerfile, .dockerignore, .cursorrules, etc.
```

---

## 9. TESTING & CI/CD

### 9.1 Local Test Commands
```bash
pip install -e ".[dev]"
python3 -m ruff check .               # Linter
python3 -m pytest tests/ -q           # Unit tests
bash scripts/ci_spec_compliance.sh    # No Alpaca data outside adapter
bash scripts/ci_mlflow_promotion_policy.sh  # No automatic MLflow promotion
bash scripts/ci_canonical_contracts.sh      # Spec contract smoke tests
bash scripts/ci_canonical_gates.sh    # Full APEX spec gating (runs all below)
bash scripts/ci_canonical_glossary.sh # Canonical naming references
bash scripts/ci_pip_audit.sh          # Dependency vulnerability check
bash scripts/ci_bandit.sh             # Security linter
```

### 9.2 GitHub Actions CI (`ci.yml`)
- **Lint / Test Job:** ruff, pip-audit, bandit, pytest, spec compliance, queue consistency, canonical gates
- **Gitleaks Job:** Secret scanning (Docker image)
- **Docker Job:** Hadolint, build, smoke import, Trivy filesystem scan
- **Integration Job (optional):** Redis, Qdrant, QuestDB services

---

## 10. KEY PATTERNS & CONVENTIONS

### 10.1 Settings Pattern
```python
from app.config.settings import AppSettings, load_settings

settings = load_settings()  # Reads .env + default.yaml, resolves env vars
print(settings.execution_mode)  # "paper" or "live"
```

### 10.2 Live Service Entry Point
```python
# app/runtime/live_service.py
async def main():
    settings = load_settings()
    pipeline = DecisionPipeline()
    risk_engine = RiskEngine(settings)
    exec_service = ExecutionService(settings)
    # Kraken WS loop → decision → execution
```

### 10.3 Replay / Backtesting
```python
# backtesting/replay.py
from backtesting.replay import replay_decisions

rows = replay_decisions(
    bars=df_ohlcv,
    pipeline=pipeline,
    risk_engine=risk_engine,
    symbol="BTC-USD",
    track_portfolio=True,
    emit_canonical_events=True,
)
```

### 10.4 Shared Decision Path
```python
# Both live_service.py and replay.py call the same function:
from decision_engine.run_step import run_decision_tick

regime, forecast, route, proposal, trade_action, risk_state = run_decision_tick(
    symbol="BTC-USD",
    feature_row={...},
    spread_bps=10.0,
    risk_state=risk_state,
    pipeline=pipeline,
    risk_engine=risk_engine,
    mid_price=42000.0,
    data_timestamp=utc_now,
    replay_deterministic=False,  # False in live, True in replay
)
```

### 10.5 Reason Codes (Stable String Prefixes)
- `trg_*` — trigger suppression reasons
- `auc_*` — auction selection reasons
- `exe_*` — execution guidance reasons
- `pip_*` — pipeline warnings
- `ovr_*` — override reasons
- `state_*` — state/degradation reasons
- `risk_*` — risk block codes (feed_stale, spread_wide, drawdown, etc.)

### 10.6 Canonical Event Types
```python
# Emitted in replay with emit_canonical_events=True
canonical_events = [
    {"family": "market", "ts": ..., "payload": {...}},
    {"family": "structural", "ts": ..., "payload": {...}},
    {"family": "safety", "ts": ..., "payload": {...}},
    {"family": "decision", "ts": ..., "payload": {...}},
    {"family": "execution", "ts": ..., "payload": {...}},
]
```

### 10.7 Non-Negotiable Rules
1. **Kraken-Only Market Data:** Never import Alpaca data client outside the execution adapter (enforced by CI)
2. **Shared Decision Step:** Live and replay **must** call `run_decision_tick` for parity
3. **Risk Is Final:** No bypass of `risk_engine.evaluate` → `signing` for execution
4. **No Automatic MLflow Promotion:** Do not add `transition_model_version_stage`
5. **No Secrets in Code:** Use `.env` (gitignored)
6. **Queue Workflow:** Use `bash scripts/queue_top.sh` to pick tasks, `bash scripts/queue_close.sh --next` to finish

---

## 11. CANONICAL SPEC COMPLIANCE (FB-CAN-*)

**Key Compliance Labels:**
- **FB-CAN-004:** CanonicalStateOutput (regime, degradation)
- **FB-CAN-015:** Typed boundary snapshots (portfolio, structural, safety)
- **FB-CAN-016:** Feature normalization with freshness/reliability degradation
- **FB-CAN-029:** Canonical orchestration sequence (structure → state → trigger → auction → carry)
- **FB-CAN-036:** DecisionRecord with reason codes, diagnostics
- **FB-CAN-037:** Fault injection profiles for stress testing
- **FB-CAN-038:** Shadow comparison (paired replay divergence)
- **FB-CAN-040:** Canonical naming glossary (legacy vs. APEX)
- **FB-CAN-045:** Composite degradation (novelty × transition × heat)
- **FB-CAN-050:** Optional feature families (options, stablecoin) with gating
- **FB-CAN-051:** Release objects + ledger (ReleaseCandidate, ReleaseRecord)
- **FB-CAN-055:** Replay event family coverage validator
- **FB-CAN-060+:** More recent compliance labels (see CANONICAL_MODULE_MAP.MD)

---

## 12. QUEUE WORKFLOW (from AGENTS.md)

**For Next Task:**
```bash
bash scripts/queue_top.sh              # Prints full next Open row (or --json)
# or
python3 scripts/print_next_queue_item.py
```

**To Close After Completion:**
```bash
bash scripts/queue_close.sh --next     # Closes top Open, regenerates CSV
# or
bash scripts/queue_close.sh --id FB-CAN-123
```

**Queue Location:** `docs/QUEUE_STACK.csv` (auto-generated from `scripts/generate_queue_stack.py`)

---

## 13. ENVIRONMENT CHECKLIST

**Pre-Run Requirements:**
1. **Python 3.11+** (test on 3.12)
2. **Docker:** QuestDB, Redis, Qdrant, Prometheus, Grafana, Loki (optional)
3. **`.env` file:** Copy from `.env.example`, fill in API keys
4. **Virtual env:** `pip install -e ".[dev]"` or `".[dashboard]"` + `".[orchestration]"` as needed

**Optional Extras:**
- `[alpaca]` — Alpaca adapter tests
- `[dashboard]` — Streamlit UI
- `[orchestration]` — Prefect + MLflow
- `[models_torch]` — PyTorch for forecaster training
- `[all]` — Everything

---

## 14. SUMMARY FOR FUTURE WORK

**Next Developer Should Know:**
1. **Shared decision path is sacred.** If live and replay diverge, bugs are subtle and costly.
2. **Config is canonical.** Most behavior lives in `apex_canonical` (YAML) or `NM_*` env vars, not hardcoded.
3. **Risk gates everything.** No trade escapes `risk_engine.evaluate()`.
4. **Reason codes are audit trail.** Suppress / no-trade decisions must include stable prefix codes.
5. **Per-symbol override pattern.** Lifecycle (Initialize/Start/Stop), execution mode (paper/live), models—all configurable per symbol via files under `data/asset_*_*`.
6. **Metrics are governance.** Canonical metrics (`tb_canonical_*`) feed release gates and rollback decisions.
7. **CI is strict.** All checks must pass before merge (no --force-push to main).
8. **Queue system is law.** Every task is tracked; agents must use `queue_top.sh` / `queue_close.sh` workflow.
9. **Specs are living.** APEX spec files under `docs/` and `docs/Specs/` define target behavior; code ↔ spec map is `CANONICAL_MODULE_MAP.MD`.
10. **Tests == safety.** No code change without regression test or explicit exemption in task.

---

**Last Updated:** 2026-06-02 by Haiku scan  
**Scope:** Complete architecture, contracts, configuration, key algorithms, testing, CI/CD, queue workflow
