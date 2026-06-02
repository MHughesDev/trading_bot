# Trading Bot Quick Reference & Common Tasks for Claude Opus 4.8

---

## QUICK STARTS

### Run Tests Locally
```bash
cd /path/to/trading_bot
pip install -e ".[dev]"
python3 -m pytest tests/ -q  # Run all tests
python3 -m pytest tests/test_decision_risk.py -v  # Single file, verbose
python3 -m pytest tests/test_backtest_live_parity.py::test_replay_matches_live -v  # Single test
```

### Run CI Locally Before Push
```bash
bash scripts/ci_spec_compliance.sh      # Kraken-only market data check
bash scripts/ci_mlflow_promotion_policy.sh
bash scripts/ci_canonical_contracts.sh  # Smoke tests on canonical contracts
bash scripts/ci_canonical_gates.sh      # Full APEX gating (runs many sub-checks)
python3 -m ruff check .
python3 -m bandit -r . -ll
```

### Start Local Development Server
```bash
# Terminal 1: Start Docker stack
docker compose -f infra/docker-compose.yml up -d

# Terminal 2: Start API (FastAPI)
uvicorn control_plane.api:app --host 0.0.0.0 --port 8000 --reload

# Terminal 3: Start Streamlit dashboard
python3 -m streamlit run control_plane/Home.py

# Then:
# API: http://localhost:8000/docs (Swagger)
# Dashboard: http://localhost:8501
```

### Add a New Task to Queue
```bash
# Use the Cursor skill (if available):
# /add-to-queue

# Or manually edit docs/QUEUE_STACK.csv and run:
python3 scripts/generate_queue_stack.py
```

### Get Next Queue Item
```bash
bash scripts/queue_top.sh       # Human-readable
bash scripts/queue_top.sh --json  # JSON output
```

### Close a Queue Item After Work
```bash
bash scripts/queue_close.sh --next   # Closes current top item
bash scripts/queue_close.sh --id FB-CAN-123  # Close by ID
```

---

## COMMON CODE PATTERNS

### Load Settings and Initialize Services
```python
from app.config.settings import AppSettings, load_settings
from decision_engine.pipeline import DecisionPipeline
from risk_engine.engine import RiskEngine
from execution.service import ExecutionService

# Load YAML defaults + .env overrides
settings = load_settings()

# Initialize decision components
pipeline = DecisionPipeline()
risk_engine = RiskEngine(settings)
exec_service = ExecutionService(settings)

print(f"Mode: {settings.execution_mode}")  # "paper" or "live"
print(f"Symbols: {settings.market_data_symbols}")  # ["BTC-USD", "ETH-USD", ...]
```

### Run a Single Decision Tick (Shared Entry Point)
```python
from decision_engine.run_step import run_decision_tick
from datetime import datetime, UTC
from decimal import Decimal

# This is the same function called by live_service.py and replay.py
regime, forecast, route, proposal, trade_action, risk_state = run_decision_tick(
    symbol="BTC-USD",
    feature_row={
        "return_1": 0.002,
        "return_3": 0.005,
        "volatility_5": 0.015,
        "bid": 42000.0,
        "ask": 42010.0,
        # ... more features
    },
    spread_bps=2.38,  # (42010 - 42000) / 42005 * 10000
    risk_state=risk_state,
    pipeline=pipeline,
    risk_engine=risk_engine,
    mid_price=42005.0,
    data_timestamp=datetime.now(UTC),
    current_total_exposure_usd=50000.0,  # Current portfolio exposure
    feed_last_message_at=datetime.now(UTC),  # Last Kraken WS message time
    product_tradable=True,
    position_signed_qty=Decimal("0.5"),  # Current position (+ long, - short)
    available_cash_usd=50000.0,
    portfolio_equity_usd=100000.0,
    replay_deterministic=False,  # False in live, True in replay
)

# regime: RegimeOutput (probabilities, confidence, heat, novelty, degradation)
# forecast: ForecastOutput (returns_1, returns_3, volatility, uncertainty)
# route: RouteDecision (route_id: SCALPING/INTRADAY/SWING/CARRY/NO_TRADE)
# proposal: ActionProposal or None (direction, size, edge)
# trade_action: TradeAction or None (risk-approved, venue-ready)
# risk_state: RiskState (updated with decision info)

if trade_action:
    print(f"Submit {trade_action.side} {trade_action.quantity} {trade_action.symbol}")
else:
    print("Blocked by risk or no opportunity")
```

### Run Backtesting / Replay
```python
import polars as pl
from backtesting.replay import replay_decisions

# Load OHLCV data (Polars DataFrame)
bars_df = pl.read_parquet("data/BTC-USD_1s_ohlcv.parquet")

# Replay through decision engine
rows = replay_decisions(
    bars=bars_df,
    pipeline=pipeline,
    risk_engine=risk_engine,
    symbol="BTC-USD",
    spread_bps=5.0,
    track_portfolio=True,
    emit_canonical_events=True,  # Includes market, structural, safety, decision, exec events
    enforce_event_family_coverage=True,  # FB-CAN-055 validation
)

# Convert to DataFrame for analysis
result_df = pl.DataFrame(rows)
print(result_df.select([
    "timestamp", "close", "route_id", "trade_action", "direction", "pnl_unrealized"
]))
```

### Submit an Order via Execution Service
```python
from app.contracts.orders import OrderIntent, OrderSide, OrderType, TimeInForce
from decimal import Decimal

# Build intent (from trade_action or manually)
intent = OrderIntent(
    symbol="BTC-USD",
    side=OrderSide.BUY,
    quantity=Decimal("0.5"),
    order_type=OrderType.MARKET,
    limit_price=None,
    stop_price=None,
    time_in_force=TimeInForce.GTC,
    client_order_id="trade-20260602-abc123",
    metadata={"route": "SCALPING", "thesis_key": "momentum"}
)

# Submit via service (handles signing, adapter routing, venue call)
import asyncio
ack = asyncio.run(exec_service.submit_order(intent))
print(f"Exchange order ID: {ack.order_id}")
print(f"Status: {ack.status}")
```

### Access Per-Symbol Configuration
```python
from app.runtime.asset_lifecycle_state import effective_lifecycle_state
from app.runtime.asset_execution_mode import effective_execution_mode
from app.runtime.asset_model_registry import load_manifest

# Check if asset is initialized and trading
lifecycle = effective_lifecycle_state("BTC-USD", settings)
print(lifecycle.state)  # Initialize | Start | Stop | Unknown

# Check if per-symbol override to live mode
exec_mode = effective_execution_mode("BTC-USD", settings)
print(exec_mode)  # "paper" or "live" (can differ from global settings.execution_mode)

# Load per-asset model manifest
manifest = load_manifest("BTC-USD", registry_dir)
if manifest:
    print(f"Forecaster: {manifest.forecaster_id}")
    print(f"Policy: {manifest.policy_id}")
```

### Query Canonical Bars from QuestDB
```python
from control_plane.chart_bars import query_canonical_bars_for_chart
from datetime import datetime, UTC

# Fetch bars for a date range
bars = await query_canonical_bars_for_chart(
    symbol="BTC-USD",
    start_time=datetime(2026, 1, 1, tzinfo=UTC),
    end_time=datetime.now(UTC),
    granularity_seconds=60,
)

for bar in bars:
    print(f"{bar.timestamp}: open={bar.open}, close={bar.close}, volume={bar.volume}")
```

### Check Decision Record Diagnostics
```python
from decision_engine.decision_record import get_last_decision_record

# After a decision tick, retrieve the audit record
rec = get_last_decision_record()
if rec:
    print(f"Symbol: {rec.symbol}")
    print(f"Timestamp: {rec.timestamp}")
    print(f"Regime: {rec.regime_output.semantic}")
    print(f"Route: {rec.route_decision.route_id}")
    print(f"Risk blocks: {rec.risk_state.last_risk_block_codes}")
    print(f"Diagnostics: {rec.diagnostics}")
    # diagnostics contains: state_engine, trigger, auction, execution_guidance_preview, risk_sizing
```

### Validate Configuration
```python
from app.config.canonical_config import resolve_canonical_config

# Resolve canonical config from settings
canonical = resolve_canonical_config(settings)
print(f"Config version: {canonical.metadata.config_version}")
print(f"Environment scope: {canonical.metadata.environment_scope}")
print(f"Max total exposure: ${canonical.domains.risk_sizing.max_total_exposure_usd}")
print(f"Auction top_n: {canonical.domains.auction.top_n}")
```

---

## DEBUGGING TIPS

### Enable Verbose Logging
```python
import logging
import structlog

# Set log level
logging.basicConfig(level=logging.DEBUG)
structlog.configure(
    processors=[
        structlog.processors.JSONRenderer()
    ]
)
```

### Inspect Feature Row
```python
from decision_engine.features_live import feature_row_from_tick

# After a tick normalization
tick = ...  # TickerSnapshot or TradeTick
feature_row = feature_row_from_tick(tick, bars, settings)
print(feature_row.keys())  # All computed features
for k, v in feature_row.items():
    print(f"  {k}: {v}")
```

### Trace a Specific Symbol Through Replay
```python
# Add print statements in decision_engine/run_step.py
# Or capture decision records
rows = replay_decisions(
    bars=bars_df,
    pipeline=pipeline,
    risk_engine=risk_engine,
    symbol="BTC-USD",
    emit_canonical_events=True,
)

# Filter for trades
trades = [r for r in rows if r.get("trade_action") is not None]
print(f"Total rows: {len(rows)}, trades: {len(trades)}")

for r in trades[:5]:
    print(f"{r['timestamp']}: {r['trade_action'].side} "
          f"{r['trade_action'].quantity} @ {r['mid_price']}")
```

### Check if Feed is Stale
```python
from datetime import datetime, UTC, timedelta

# In live loop or test
feed_last_message_at = datetime.now(UTC) - timedelta(seconds=125)  # 125 seconds old
stale_threshold = 120  # settings.risk_stale_data_seconds

if (datetime.now(UTC) - feed_last_message_at).total_seconds() > stale_threshold:
    print("Feed is stale! Risk will block trading.")
```

### Review Risk Block Codes
```python
# After run_decision_tick
risk_state = ...
if risk_state.last_risk_block_codes:
    print(f"Risk blocks: {risk_state.last_risk_block_codes}")
    # Expected codes:
    # - risk_feed_stale
    # - risk_spread_wide
    # - risk_drawdown_limit
    # - risk_product_untradable
    # - risk_available_cash
    # - etc (see risk_engine/engine.py RISK_BLOCK_* constants)
```

### Test Regime Classification
```python
# Create a mock feature row
feature_row = {
    "rsi_14": 25.0,  # Extreme
    "volatility_5": 0.045,  # High
    "return_1": -0.008,  # Downtrend
    "funding_rate": 0.0001,
    "liquidation_long_pct": 0.02,
    # ... fill in more features
}

# Run through pipeline
regime, forecast, _, _, _, _ = run_decision_tick(
    symbol="BTC-USD",
    feature_row=feature_row,
    # ... other required args
)

print(f"Regime probabilities: {regime.canonical_regime_probabilities}")
# [trend, range, stress, dislocated, transition]
print(f"Regime confidence: {regime.confidence}")
print(f"Heat score: {regime.heat_score}")
print(f"Novelty: {regime.novelty_score}")
print(f"Degradation: {regime.degradation}")  # NORMAL, REDUCED, DEFENSIVE, NO_TRADE
```

---

## COMMON ISSUES & FIXES

| Issue | Cause | Fix |
|-------|-------|-----|
| `ImportError: cannot import name 'legacy'` from outside `legacy/` | Non-negotiable rule violation | Remove import; use main V3 modules instead |
| `OrderIntent rejected: metadata contains forbidden keys` | Raw news/text in metadata | Remove 'headline', 'raw_text', 'article' etc. |
| `Risk blocks with risk_feed_stale` | Kraken WS disconnected or lagging | Check network, restart WS client, increase threshold |
| `Trade action is None` but proposal exists | Stopped by risk engine | Check `risk_state.last_risk_block_codes` |
| `Replay rows mismatch live output` | Non-deterministic element in live path | Ensure `replay_deterministic=True` in replay, set seed |
| `CanonicalStateOutput validation fails` | Regime probabilities don't sum to ~1 | Check state_engine.py probability computation |
| `Alpaca order fails` | Paper adapter not initialized or no cash | Check Alpaca API keys in `.env`, portfolio equity |
| `QuestDB connection refused` | Docker QuestDB not running | `docker compose -f infra/docker-compose.yml up -d` |
| `pytest hangs on async tests` | Event loop issue on Windows | Already handled in `control_plane/api.py` (SelectorEventLoop); should auto-detect |

---

## FILE EDITS CHECKLIST

When modifying key modules:

### Decision Logic Changes
- [ ] Update `decision_engine/run_step.py` or `canonical_orchestrator.py`
- [ ] Add test in `tests/test_decision_risk.py` or new `test_*_decision.py`
- [ ] Verify replay parity in `tests/test_backtest_live_parity.py`
- [ ] Update diagnostics in `decision_engine/decision_record.py` if needed
- [ ] Run: `bash scripts/ci_canonical_gates.sh`

### Risk Engine Changes
- [ ] Update `risk_engine/engine.py` or `canonical_sizing.py`
- [ ] Add test in `tests/test_decision_risk.py`
- [ ] Update reason codes in `risk_engine/engine.py` (RISK_BLOCK_*)
- [ ] Verify no bypass of `signing.py` for execution
- [ ] Run: `python3 -m pytest tests/test_decision_risk.py -v`

### Execution Adapter Changes
- [ ] Update adapter in `execution/adapters/alpaca_adapter.py` or `coinbase_adapter.py`
- [ ] Do NOT modify `execution/service.py` unless routing logic changes
- [ ] Add test in `tests/test_execution_*.py`
- [ ] Verify `require_execution_allowed` is still called
- [ ] Run: `python3 -m pytest tests/test_execution_*.py -v`

### Configuration Changes
- [ ] Update `app/config/default.yaml` for APEX canonical defaults
- [ ] Update `app/config/settings.py` if new NM_* env var added
- [ ] Update `.env.example` with example value
- [ ] Update `README.md` "⚙️" section if user-facing
- [ ] Run: `python3 -m pytest tests/test_config_*.py -v`

### Feature Pipeline Changes
- [ ] Update `data_plane/features/pipeline.py` or `canonical_normalize.py`
- [ ] Ensure freshness/reliability degradation applied
- [ ] Test with `tests/test_feature_service_pipeline_integration.py`
- [ ] Verify feature names propagate to decision record
- [ ] Run: `bash scripts/ci_spec_compliance.sh`

### Control Plane API / Dashboard Changes
- [ ] Update `control_plane/api.py` for new endpoints
- [ ] Update `control_plane/pages/*.py` for Streamlit UI changes
- [ ] Test with `tests/test_control_plane_*.py`
- [ ] Verify auth middleware (session cookie) still enforced
- [ ] Run: `uvicorn control_plane.api:app --reload` and visit `http://localhost:8000/docs`

### Documentation Changes
- [ ] Update `docs/*.MD` files (uppercase `.MD` extension required)
- [ ] If task involves queue work, update `docs/QUEUE_STACK.csv`
- [ ] If architectural change, update `docs/CANONICAL_MODULE_MAP.MD`
- [ ] If new reason code, update `docs/CANONICAL_GLOSSARY.MD`
- [ ] If new FB-CAN-* compliance label, update `docs/CANONICAL_SPEC_INDEX.MD`
- [ ] Run: `python3 scripts/ci_queue_consistency.py` (if queue changed)

---

## GIT WORKFLOW (from AGENTS.md)

### New Feature Branch
```bash
git fetch origin main
git checkout -b claude/feature-description-XXXXX main
# ... edit files
git add .
git commit -m "Description of change

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
# ... run tests (see above)
git push origin claude/feature-description-XXXXX
```

### After Tests Pass
```bash
git checkout main
git merge claude/feature-description-XXXXX
git push origin main
git push origin --delete claude/feature-description-XXXXX
git branch -d claude/feature-description-XXXXX
```

### Close Queue Item
```bash
bash scripts/queue_close.sh --next  # Updates QUEUE_STACK.csv, QUEUE_ARCHIVE.MD
git add docs/QUEUE_*.* scripts/generate_queue_stack.py
git commit -m "Close queue item [ID] - [description]

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
git push origin main
```

---

## GLOSSARY OF ABBREVIATIONS

| Acronym | Meaning |
|---------|---------|
| **APEX** | Target canonical architecture (specs under `docs/Human Provided Specs/new_specs/canonical/`) |
| **FB-CAN-*** | Canonical spec compliance label (e.g., FB-CAN-004 = regime/degradation) |
| **FB-AP-*** | Application capability label |
| **FB-CONT-*** | Container/infrastructure label |
| **FB-AUD-*** | Audit/governance label |
| **FB-UX-*** | User experience / control plane label |
| **OHLCV** | Open, High, Low, Close, Volume (standard bar format) |
| **RNG** | Random number generator (for stochastic replay) |
| **EMA** | Exponential moving average (execution feedback penalty) |
| **D_corr** | Diversification penalty: correlation |
| **D_thesis** | Diversification penalty: thesis overlap |
| **D_liq** | Diversification penalty: liquidation stress |
| **SSE** | Server-sent events (real-time chart streaming) |
| **WS** | WebSocket (Kraken market data) |
| **REST** | HTTP-based API (Kraken historical bars) |

---

## EXTERNAL LINKS & REFERENCES

- **Kraken Public API:** https://docs.kraken.com/rest/
- **Alpaca Trading API:** https://alpaca.markets/docs/trading/
- **Coinbase Advanced Trade:** https://docs.cloud.coinbase.com/advanced-trade/
- **Polars:** https://pola-rs.github.io/polars/
- **FastAPI:** https://fastapi.tiangolo.com/
- **Streamlit:** https://streamlit.io/
- **Pydantic:** https://docs.pydantic.dev/
- **QuestDB:** https://questdb.io/docs/
- **Qdrant:** https://qdrant.tech/documentation/
- **Prometheus:** https://prometheus.io/docs/

---

**Last Updated:** 2026-06-02 by Haiku scan  
**For:** Claude Opus 4.8 and future developers
