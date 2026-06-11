# Three-Phase Trading System: Quick Reference Card

**Print or bookmark this page for quick lookup during development & design reviews.**

---

## Visual Flow

```
MARKET DATA (60s candles)
    ↓
┌───────────────────────────────┐
│  PHASE 1: PRICE PREDICTION     │
│  (15–50ms)                      │
│  • VSN: Feature selection      │
│  • CNN: Latent encoding        │
│  • xLSTM: 4 branches (1,5,20,100)
│  • Fusion: Regime-weighted     │
│  • Decoder: 3 quantiles [8 steps]
│  Output: q_low, q_med, q_high  │
└──────────┬────────────────────┘
           │
       ForecastPacket
           │
┌──────────▼────────────────────┐
│  PHASE 2: DECISION HANDLING    │
│  (5–20ms)                      │
│  • Setup (scores): A,S,C,H,N,Rfx
│  • Pre-Trigger (exec quality)  │
│  • Confirm (trigger type)      │
│  • Route selection             │
│  Output: ActionProposal        │
│  (route, direction, size%, stoploss)
└──────────┬────────────────────┘
           │
       ActionProposal
           │
┌──────────▼────────────────────┐
│  PHASE 3: RISK & EXECUTION    │
│  (30–100ms)                    │
│  • 8 hard gates (feed, stale, │
│    spread, drawdown, etc.)     │
│  • Canonical sizing:           │
│    × degradation              │
│    × inertia                  │
│    × asymmetry_boost          │
│    × liquidation_mode         │
│    × edge_budget              │
│    × concentration            │
│  • Quantity conversion         │
│  • Order signing & submission  │
│  Output: TradeAction → Exchange│
└────────────────────────────────┘
           │
       OrderIntent (Signed)
           │
      EXCHANGE API
      (Alpaca / Coinbase)
```

---

## Phase 1: Price Prediction (15–50ms)

### Input
- `x_obs [128, F]` — historical price/volume features
- `x_known [8, Fk]` — future exogenous features
- `r_cur [4]` — market regime vector

### Output
- `q_low [8]` — 10th percentile (pessimistic)
- `q_med [8]` — 50th percentile (median/expected)
- `q_high [8]` — 90th percentile (optimistic)
- `confidence_score` — overall quality [0,1]

### Configuration
| Param | Value | Notes |
|-------|-------|-------|
| history_length | 128 | ~2 hours @ 60s |
| forecast_horizon | 8 | ~8 minutes ahead |
| quantiles | (0.1, 0.5, 0.9) | Percentiles |
| branch_scales | (1,5,20,100) | Temporal resolutions |
| latent_width | 32 | Bottleneck size |
| recurrent_hidden_width | 128 | LSTM hidden dim |

### Code
- File: `forecaster_model/models/forecaster_model.py`
- Class: `ForecasterModel`
- Method: `forward(x_obs, x_known, r_cur)`

---

## Phase 2: Decision Handling (5–20ms)

### Input
- `ForecastPacket` from Phase 1
- `CanonicalStateOutput` (regime, heat, novelty, reflexivity)
- `feature_row` (microstructure: funding, volume, spreads)
- `spread_bps`, `data_timestamp`

### 3-Stage Trigger

#### Stage 1: Setup
```
setup_score = 0.35×A + 0.25×S + 0.30×C - 0.35×H - 0.35×N - 0.25×Rfx
where:
  A = asymmetry_score       (quantile skew)
  S = state_alignment       (regime certainty)
  C = confidence            (model agreement)
  H = heat                  (systemic risk)
  N = novelty               (OOD flag)
  Rfx = reflexivity         (self-reference)

Check: setup_score >= 0.22 (tunable via settings)
Blocks: TRG_LOW_SETUP_SCORE, TRG_NOVELTY_BLOCK, TRG_DEGRADATION_BLOCK
```

#### Stage 2: Pre-Trigger
```
exec_conf = 1 - (spread_bps / 80) × 0.8
pretrigger_score = setup_score - freshness_penalty

Check: pretrigger_score >= 0.18 AND exec_conf >= 0.12
Blocks: TRG_STALE_PRETRIGGER_INPUTS, TRG_POOR_EXECUTION_CONTEXT
```

#### Stage 3: Confirmed Trigger
```
trigger_type = {imbalance_spike | volume_burst | structure_break | composite}
trigger_confidence = setup_score
direction = sign(q_med - q_low)

Check: trigger_strength >= 0.2 AND trigger_confidence >= 0.1
Blocks: TRG_TRIGGER_STRENGTH_LOW, TRG_MOVE_ALREADY_EXTENDED
```

### Route Selection
| Route | Timeframe | Size | Stop | Route ID |
|-------|-----------|------|------|----------|
| SCALPING | < 5min | 35% | 1.2% | `SCALPING` |
| INTRADAY | < 4h | 45% | 2% | `INTRADAY` |
| SWING | > 4h | 60% | 4% | `SWING` |
| CARRY | ∞ | 25% | 2.5% | `CARRY` |

### Output: ActionProposal
```python
{
  "symbol": "BTC/USD",
  "route_id": RouteId.SCALPING,
  "direction": +1 (long) or -1 (short),
  "size_fraction": 0.35,
  "stop_distance_pct": 0.012,
  "order_type": "market",
  "expiry_seconds": 300
}
```

### Code
- File: `decision_engine/trigger_engine.py`
- Function: `evaluate_trigger(pkt, feature_row, ...)`
- Class: `TriggerThresholds` (tunable defaults)

---

## Phase 3: Risk & Execution (30–100ms)

### Input
- `ActionProposal` from Phase 2
- Market state: `mid_price`, `spread_bps`, position, equity, cash
- Risk state: `mode`, `canonical_size_multiplier`, various scores

### 8 Hard Risk Gates (Precedence)
| # | Gate | Threshold | Block Code |
|---|------|-----------|-----------|
| 1 | Feed stale | > 300s | `RISK_BLOCK_FEED_STALE` |
| 2 | Data stale | > 300s | `RISK_BLOCK_DATA_TIMESTAMP_STALE` |
| 3 | Spread wide | > 50 bps | `RISK_BLOCK_SPREAD_WIDE` |
| 4 | Drawdown | > 15% | `RISK_BLOCK_DRAWDOWN` |
| 5 | Product | untradable | `RISK_BLOCK_PRODUCT_UNTRADABLE` |
| 6 | Data health | integrity_alert | `RISK_BLOCK_DATA_HEALTH` |
| 7 | System mode | MAINTENANCE | `RISK_BLOCK_MAINTENANCE` |
| 8 | Proposal | None | `RISK_BLOCK_NO_PROPOSAL` |

### Canonical Sizing (Multiplier Stack)

```
base = size_fraction × max_per_symbol_usd

Step 1: Degradation
├─ NORMAL: ×1.0
├─ DEFENSIVE: ×0.65
├─ REDUCED: ×0.40
└─ NO_TRADE: ×0.0 (blocks)

Step 2: Position Inertia
├─ Flipping direction? Yes → ×(1 - 0.55 × position_fraction)
└─ No → ×1.0

Step 3: Asymmetry Boost
├─ If asymmetry > 0.55 AND trigger_conf >= 0.22 AND ...
├─ boost = min(1.2, 1.0 + (asymmetry - 0.5) × 0.85)
└─ Else → ×1.0

Step 4: Liquidation Mode
├─ OFFENSE: ×1.05
├─ NEUTRAL: ×1.0
└─ DEFENSE: ×0.88

Step 5: Edge Budget
├─ stress = 0.45×heat + 0.35×exposure% + 0.2×symbol%
└─ edge_m = max(0.35, 1 - 0.62×stress)

Step 6: Concentration
├─ symbol_m = max(0.25, 1 - (sym_frac - 0.38) × 2.2) if sym_frac > 0.38
└─ book_m = max(0.4, 1 - (book_frac - 0.82) × 3.0) if book_frac > 0.82

Final: final = min(max_slot, base × [all multipliers])
```

### Quantity & Checks
```python
quantity = round(final_notional / mid_price, 8)

Checks:
  ✓ quantity > 0              (else RISK_BLOCK_QTY_ZERO)
  ✓ if BUY: notional <= cash  (else RISK_BLOCK_AVAILABLE_CASH)
  ✓ if REDUCE_ONLY: qty <= |position|  (else clamped or blocked)
```

### Output: TradeAction
```python
{
  "symbol": "BTC/USD",
  "side": "buy" | "sell",
  "quantity": Decimal("9.4273"),
  "order_type": "market",
  "time_in_force": "gtc",
  "route_id": RouteId.SCALPING
}
```

### Configuration
```yaml
risk:
  risk_stale_data_seconds: 300
  risk_max_spread_bps: 50
  risk_max_drawdown_pct: 0.15
  risk_max_per_symbol_usd: 5000
  risk_max_total_exposure_usd: 10000

apex_canonical.domains.risk_sizing:
  quantile_asymmetry_boost_cap: 1.2
  position_inertia_penalty_weight: 0.55
  edge_budget_weight_heat: 0.45
  edge_budget_strength: 0.62
  symbol_concentration_threshold: 0.38
  book_concentration_threshold: 0.82
```

### Code
- File: `risk_engine/engine.py`
- Class: `RiskEngine`
- Method: `evaluate(symbol, proposal, risk, ...)`
- Function: `compute_canonical_notional(proposal, risk, ...)`

---

## Key Metrics

### Phase 1: Prediction
- `tb_forecast_confidence_avg` — average model confidence
- `tb_forecast_q_spread` — (q_high - q_low) / q_med (%)
- `tb_forecast_ood_score` — out-of-distribution flag
- **Goal:** q_spread narrow (clear signal), confidence high, OOD rare

### Phase 2: Decision
- `tb_trigger_setup_score_avg` — average setup score
- `tb_trigger_valid_rate` — % bars where trigger fires
- `tb_asymmetry_score_avg` — average directional bias
- **Goal:** Triggers selective (3–5% of bars), high asymmetry when firing

### Phase 3: Risk & Execution
- `tb_risk_blocks_total` — # orders blocked by each gate
- `tb_order_fill_rate` — filled / submitted (%)
- `tb_order_slippage_bps` — (fill_price - mid) / mid (bps)
- `tb_edge_budget_headroom` — 1 - edge_stress
- **Goal:** Low slippage (<5 bps), high fill rate (>90%), headroom positive

---

## Checklist: "Did I Break Anything?"

After a change to any phase:

- [ ] All tests pass: `pytest tests/ -q`
- [ ] Determinism check: replay same bar → identical decision
- [ ] Latency check: Phase timing still < 60ms total
- [ ] Decision records valid: No null fields, reason_codes populated
- [ ] Risk gates still work: Verify blocks in logs
- [ ] Config still reads: No YAML parse errors
- [ ] No new randomness: Check `random.*` and `np.random.*` calls
- [ ] Backtest: Run on historical data, PnL reasonable

---

## Common Failure Modes & Diagnostics

### Phase 1 Failures
| Symptom | Likely Cause | Fix |
|---------|-------------|-----|
| Confidence always 0 | NaN in model | Check feature pipeline |
| Quantiles equal | Decoder broken | Check tanh activation |
| Coverage << 80% | Model OOD | Retrain or add conformal |
| Latency > 60ms | Model bloat | Profile and optimize |

### Phase 2 Failures
| Symptom | Likely Cause | Fix |
|---------|-------------|-----|
| Triggers never fire | Thresholds too high | Lower setup_threshold |
| Triggers always fire | Thresholds too low | Raise thresholds |
| Wrong direction | Asymmetry inverted | Check q_low vs q_high |
| No route assigned | Route function broken | Debug route_decision logic |

### Phase 3 Failures
| Symptom | Likely Cause | Fix |
|---------|-------------|-----|
| Orders blocked often | Too many risk gates failing | Check which gate dominates |
| Size too small | Multipliers stacking too much | Reduce penalty weights |
| Slippage high | Spread threshold too low | Increase risk_max_spread_bps |
| Position concentration | Edge budget too weak | Increase edge_budget_strength |

---

## File Locations (TL;DR)

| What | Where |
|------|-------|
| Phase 1 model | `forecaster_model/models/forecaster_model.py` |
| Phase 1 config | `forecaster_model/config/__init__.py` |
| Phase 2 trigger | `decision_engine/trigger_engine.py` |
| Phase 2 events | `decision_engine/bar_event_trigger.py` |
| Phase 3 risk | `risk_engine/engine.py` |
| Phase 3 sizing | `risk_engine/canonical_sizing.py` |
| Config (all) | `app/config/default.yaml` |
| Contracts | `app/contracts/*.py` (decisions.py, orders.py, trigger.py) |
| Monitoring | `observability/metrics.py` |
| Tests | `tests/test_*.py` |

---

## Quick Links

- **Full Spec:** [`docs/SYSTEM_SPECIFICATION.md`](SYSTEM_SPECIFICATION.md)
- **Design Checklist:** [`docs/PHASE_DESIGN_CHECKLIST.md`](PHASE_DESIGN_CHECKLIST.md)
- **Architecture:** [`docs/architecture/system_walkthrough.md`](architecture/system_walkthrough.md)
- **APEX Spec:** [`docs/CANONICAL_SPEC_INDEX.MD`](CANONICAL_SPEC_INDEX.MD)
- **Contributor Contract:** [`AGENTS.md`](../AGENTS.md)

---

**Last Updated:** 2026-06-03  
**Maintainer:** AI Engineering Team  
**Print & Post:** Yes (use this as your desk reference)
