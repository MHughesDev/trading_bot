# Trading Bot: Module Inventory & Architecture

**Generated:** 2026-06-04  
**Purpose:** High-level overview of all modules, their responsibilities, and dependencies

---

## Core Trading Logic (Phase 1–3)

### 1. **forecaster_model** 
**Purpose:** Phase 1 — Price prediction via deep learning  
**Responsibility:**
- Takes historical OHLCV data + regime vector → outputs probabilistic price forecasts
- Implements: VSN → Latent CNN → Multi-Resolution xLSTM → Regime Fusion → Quantile Decoder
- Produces: quantile forecasts (q_low, q_med, q_high) over horizon [0, H)

**Key Submodules:**
- `models/` — neural network layers (forecaster_model.py, vsn.py, latent_encoder.py, xlstm_cell.py, fusion.py, decoder.py)
- `config/` — hyperparameter defaults (ForecasterConfig)
- `inference/` — runtime inference loop
- `training/` — offline model training
- `calibration/` — conformal prediction / uncertainty calibration
- `features/` — input feature engineering
- `regime/` — regime classification

**Dependencies:** numpy, talib (technical indicators)

---

### 2. **decision_engine**
**Purpose:** Phase 2 — Convert forecasts → trading signals & proposals  
**Responsibility:**
- Evaluates 3-stage trigger (setup → pre-trigger → confirmed)
- Computes trigger_valid, trigger_type, trigger_confidence
- Routes to strategy (SCALPING | INTRADAY | SWING | CARRY)
- Outputs: ActionProposal (direction, size_fraction, stop_distance, expiry)

**Key Files:**
- `trigger_engine.py` — 3-stage trigger evaluation (asymmetry, state alignment, confidence)
- `bar_event_trigger.py` — event subscription (listens for market.bar.closed.v1)
- `run_step.py` — main decision tick entry point

**Related:**
- `carry_sleeve/engine.py` — CARRY route logic (funding-rate-based trades)

**Dependencies:** app.contracts (decision types), risk_engine (for state)

---

### 3. **risk_engine**
**Purpose:** Phase 3 — Risk gates + position sizing  
**Responsibility:**
- Enforces 8 hard risk gates (feed stale, spread, drawdown, etc.) with precedence ordering
- Computes canonical sizing via multiplier stack (degradation, inertia, boost, edge_budget, concentration)
- Converts ActionProposal → TradeAction (risk-approved)
- Tracks peak equity, drawdown, positions

**Key Files:**
- `engine.py` — RiskEngine class; main evaluate() method
- `canonical_sizing.py` — compute_canonical_notional() + all multiplier functions
- `signing.py` — order cryptographic signing

**Dependencies:** app.contracts (decisions, risk state), app.config (thresholds)

---

## Data & Feature Pipeline

### 4. **data_plane**
**Purpose:** Market data ingestion, normalization, feature engineering  
**Responsibility:**
- Pulls raw market data (Kraken WebSocket → QuestDB)
- Normalizes into canonical OHLCV bars (60-second default)
- Computes technical features (EMA, RSI, ATR, volume, funding rate, etc.)
- Maintains data quality checks & integrity alerts

**Key Submodules:**
- `ingest/` — data source adapters (Kraken, Coinbase, Alpaca)
- `bars/` — OHLCV aggregation and storage
- `features/` — technical indicator computation
- `storage/` — QuestDB integration (persistent bars)
- `health/` — data quality monitoring
- `memory/` — Redis cache for hot data

**Dependencies:** QuestDB, Redis, websockets, talib

---

### 5. **policy_model**
**Purpose:** (Currently exploratory) Reinforcement learning policy for sizing/routing  
**Responsibility:**
- Alternative to hand-tuned Phase 2/3 logic
- Learns to optimize sizing given market state
- Candidate for future automation of threshold tuning

**Key Submodules:**
- `policy/` — policy network
- `risk/` — risk constraints for RL
- `training/` — offline training
- `integration/` — hooks into decision pipeline (if enabled)

**Status:** Not in production hot path (experimental)

**Dependencies:** torch (or numpy), gym-like environment

---

## Execution & Orders

### 6. **execution**
**Purpose:** Order placement and exchange connectivity  
**Responsibility:**
- Takes TradeAction → creates OrderIntent → signs → submits to exchange
- Manages connections to Alpaca (paper) and Coinbase (live)
- Tracks order fills, slippage, execution quality

**Key Submodules:**
- `adapters/` — exchange-specific adapters (alpaca_adapter.py, coinbase_adapter.py, etc.)
- `execution_logic.py` — order routing and submission

**Dependencies:** alpaca-py, coinbase SDK, signing keys

---

### 7. **orchestration**
**Purpose:** Service coordination and startup  
**Responsibility:**
- Starts/stops all services in dependency order
- Manages configuration overrides and mode switching (paper ↔ live)
- Scheduling and periodic tasks

**Key Files:**
- `app_scheduler.py` — periodic job scheduler
- `startup_canonical_backfill.py` — cold-start data bootstrap

**Dependencies:** All of the above (orchestrates them)

---

## Application Core

### 8. **app**
**Purpose:** Central application contracts, configuration, and state  
**Responsibility:**
- Defines all Pydantic data models (ForecastPacket, ActionProposal, TradeAction, TriggerOutput, RiskState, etc.)
- Holds configuration (YAML → AppSettings)
- Runtime asset registry and canonical state computation

**Key Submodules:**
- `contracts/` — all data types (decisions.py, orders.py, trigger.py, forecast_packet.py, risk.py, etc.)
- `config/` — configuration loading and defaults (default.yaml)
- `runtime/` — asset model registry, canonical state machine

**Dependencies:** pydantic, yaml

---

## Observability & Monitoring

### 9. **observability**
**Purpose:** Metrics, logging, and diagnostics  
**Responsibility:**
- Prometheus metrics emission (gauge, counter, histogram)
- Structured logging to stdout / Promtail / Loki
- Performance monitoring (latency, throughput)
- Governance metrics (FB-CAN-065: config drift, promotion gates, probation status)

**Key Files:**
- `metrics.py` — all Prometheus metric definitions
- Collectors for: forecast quality, trigger rates, order counts, risk blocks, edge budget headroom, etc.

**Dependencies:** prometheus-client, logging

---

## Infrastructure & Deployment

### 10. **infra**
**Purpose:** Docker, monitoring, and deployment configuration  
**Responsibility:**
- Docker Compose stacks (QuestDB, Redis, Grafana, Prometheus, Loki/Promtail)
- Grafana dashboards
- Prometheus alerting rules
- Caddy reverse proxy config (if using)

**Key Submodules:**
- `docker/` — docker-compose files, Dockerfiles
- `grafana/` — dashboards, data source configs
- `prometheus/` — scrape configs, alerts
- `promtail/` — log shipping config
- `caddy/` — reverse proxy (optional)

**Dependencies:** Docker, Docker Compose

---

## API & Control Plane

### 11. **control_plane**
**Purpose:** FastAPI REST API and future Streamlit dashboard  
**Responsibility:**
- HTTP endpoints for: manual orders, mode switching, position queries, decision records
- Serves decision logs, audit trail, risk state
- Dashboard (Streamlit, in development)

**Key Files:**
- `main.py` or equivalent — FastAPI app
- Route handlers for: /trade/order, /risk/state, /decisions, /positions, etc.

**Dependencies:** fastapi, uvicorn

---

### 12. **charts**
**Purpose:** Reusable trading chart widget  
**Responsibility:**
- Renders OHLCV bars, technical indicators, price bands, orders
- Integrates with control plane UI

**Key Files:**
- Chart rendering components (likely JavaScript/React or Python Plotly)

**Dependencies:** plotly or d3.js (depending on impl)

---

## Testing & Utilities

### 13. **backtesting**
**Purpose:** Historical simulation of trading logic  
**Responsibility:**
- Replays historical bars through the decision pipeline
- Computes PnL, Sharpe ratio, max drawdown, hit rate, etc.
- A/B testing of config changes

**Key Files:**
- Backtesting engine
- Historical data loader
- PnL reporter

**Dependencies:** pandas, numpy

---

### 14. **tests**
**Purpose:** Unit and integration tests  
**Responsibility:**
- test_*.py files covering: trigger engine, risk engine, forecaster, execution adapters, etc.
- Fixtures for mock data, market state, order responses

**Key Tests:**
- `test_trigger_engine.py` — 3-stage trigger evaluation
- `test_risk_engine.py` — risk gates, sizing
- `test_forecaster_model.py` — model forward pass
- `test_active_model_set.py` — model registry
- `test_execution_*.py` — order placement

**Dependencies:** pytest

---

### 15. **research**
**Purpose:** Exploratory notebooks and prototypes  
**Responsibility:**
- Data analysis, feature exploration, model development experiments
- Not in production hot path; for R&D

**Dependencies:** jupyter, pandas, numpy, scikit-learn

---

## Shared Infrastructure

### 16. **shared**
**Purpose:** Cross-module utilities  
**Responsibility:**
- Messaging bus (pub/sub for events)
- Envelope types (EventEnvelope for bar.closed, order.filled, etc.)
- Topic definitions

**Key Submodules:**
- `messaging/bus.py` — in-memory event bus
- `messaging/envelope.py` — event wrapper
- `messaging/topics.py` — canonical topic names

**Dependencies:** pydantic

---

## Packaging & Operator

### 17. **operator_packaging**
**Purpose:** Desktop app & operator experience  
**Responsibility:**
- Windows/macOS desktop app shell
- Login, session management, notification
- Shortcuts and launcher

**Key Submodules:**
- `desktop_app/` — Python/Qt wrapper
- `desktop_shell/` — OS-level launcher

**Status:** Optional; for end-user convenience

---

### 18. **packaging**
**Purpose:** Release and distribution  
**Responsibility:**
- Setup.py, wheel building
- Platform-specific installers

---

## Documentation & Scripts

### 19. **docs**
**Purpose:** All documentation (specs, guides, runbooks)  
**Responsibility:**
- Architecture docs, system specs, design decisions
- Operations runbooks, troubleshooting guides

**Key Docs:**
- `SYSTEM_SPECIFICATION.md` — three-phase spec (Phase 1, 2, 3)
- `PHASE_DESIGN_CHECKLIST.md` — design iteration template
- `PHASE_QUICK_REFERENCE.md` — quick lookup card
- `architecture/` — detailed component walkthroughs
- `CANONICAL_SPEC_INDEX.MD` — APEX system spec

---

### 20. **scripts**
**Purpose:** CLI utilities and automation  
**Responsibility:**
- queue_top.sh — queue management
- ci_*.py — CI/CD checks and gates
- doctor.sh — environment validation

---

## Legacy & Research

### 21. **legacy**
**Purpose:** Old codebase (pre-refactor)  
**Responsibility:**
- Historical reference; not in use
- Contains cryptobot v1 (superseded)

**Status:** Archived; do not use

---

## MCP (Model Context Protocol)

### 22. **mcp_server**
**Purpose:** Claude integration via MCP  
**Responsibility:**
- Exposes trading bot as an LLM tool server
- Allows Claude to query positions, recent decisions, market data, etc.

**Key Tools:**
- Query decision history
- Place manual orders (with risk approval)
- Fetch market snapshots
- Get asset list

**Dependencies:** mcp SDK

---

---

## Dependency Graph (Simplified)

```
                         orchestration
                               ↑
          ┌────────────────────┼────────────────────┐
          ↓                    ↓                     ↓
    data_plane           decision_engine        risk_engine
          ↑                    ↑                     ↑
          │            ┌───────┴────────┐           │
          │            ↓                ↓           │
         app ←────────→ carry_sleeve    ←───────────┤
          ↑                                         │
          │                                         ↓
       shared ←──────────────────────────────── execution
          ↑                                         ↑
          │                                         │
     control_plane, observability, backtesting ────┘
          ↑
          │
   operator_packaging, charts, mcp_server (UI/API layers)
```

---

## Module Maturity & Stability

| Module | Status | Notes |
|--------|--------|-------|
| forecaster_model | ✅ Production | NumPy baseline; xLSTM can be swapped |
| decision_engine | ✅ Production | 3-stage trigger is stable |
| risk_engine | ✅ Production | Hard gates + sizing multipliers; well-tested |
| data_plane | ✅ Production | Kraken, Alpaca, Coinbase ingest working |
| execution | ✅ Production | Alpaca (paper) stable; Coinbase (live) ready |
| control_plane | ⚠️ Partial | FastAPI core done; Streamlit dashboard in dev |
| policy_model | 🧪 Experimental | RL policy not yet in hot path |
| backtesting | ✅ Stable | Used for A/B testing configs |
| mcp_server | 🆕 New | Allows Claude integration |
| operator_packaging | ⚠️ Optional | Desktop app available but not required |

---

## Key Interfaces (How They Talk)

### Event Flow
```
Market (Kraken) 
  → data_plane (normalize)
  → shared.messaging (bar.closed event)
  → decision_engine (read state)
  → carry_sleeve (if CARRY route)
  → risk_engine (evaluate proposal)
  → execution (place order)
  → control_plane (log decision record)
```

### Data Flow
```
x_obs [128, F] ─────────────┐
x_known [8, Fk] ────────────┤
r_cur [4] ──────────────────┤
                             ↓
                    forecaster_model
                             ↓
                    ForecastPacket ──→ decision_engine
                                            ↓
                                      ActionProposal ──→ risk_engine
                                                             ↓
                                                       TradeAction ──→ execution
```

---

## Where Each Phase Lives

| Phase | Core Module | Related |
|-------|-------------|---------|
| **Phase 1** (Prediction) | forecaster_model | data_plane (input features), app (config) |
| **Phase 2** (Decision) | decision_engine | carry_sleeve (CARRY route), app.contracts |
| **Phase 3** (Execution) | risk_engine | execution, control_plane (record), observability |

---

## Where to Add New Features

| Feature | Module | Why |
|---------|--------|-----|
| New forecasting model | forecaster_model/models/ | Plugs into Phase 1 contract |
| New trigger signal | decision_engine/ | Feeds into 3-stage logic |
| New route/strategy | decision_engine/ or carry_sleeve/ | Emits ActionProposal |
| New risk gate | risk_engine/engine.py | Adds to precedence chain |
| New sizing multiplier | risk_engine/canonical_sizing.py | Feeds into stack |
| New exchange adapter | execution/adapters/ | Implements place_order() |
| New technical indicator | data_plane/features/ | Feeds feature row |
| New API endpoint | control_plane/ | Exposes internal state |
| New monitoring metric | observability/metrics.py | Emits Prometheus gauge/counter |

---

**Last Updated:** 2026-06-04
