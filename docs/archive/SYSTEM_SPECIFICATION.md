# Trading Bot: System Specification
## Three-Phase Architecture (Current Implementation)

**Version:** 1.0  
**Last Updated:** 2026-06-03  
**Status:** Current Production Architecture  
**Purpose:** Unified reference for system design, refactoring, and iteration

---

## Table of Contents

1. [System Overview](#system-overview)
2. [Phase 1: Price Prediction](#phase-1-price-prediction)
3. [Phase 2: Decision Handling](#phase-2-decision-handling)
4. [Phase 3: Trade Execution](#phase-3-trade-execution)
5. [Data Flow Contracts](#data-flow-contracts)
6. [Design Constraints & Assumptions](#design-constraints--assumptions)
7. [Monitoring & Observability](#monitoring--observability)

---

## System Overview

### High-Level Architecture

```
Market Data (Kraken WebSocket)
    ↓
[PHASE 1] Price Prediction Model
    ↓ ForecastPacket
[PHASE 2] Decision & Trigger Engine
    ↓ ActionProposal
[PHASE 3] Risk & Execution Engine
    ↓ TradeAction
Exchange API (Alpaca / Coinbase)
```

### Execution Frequency
- **Cadence:** Runs every 60 seconds (on bar close)
- **Trigger:** `market.bar.closed.v1` event published by live service
- **Handler:** `BarDecisionTrigger` → `run_decision_tick()`
- **Total E2E Latency:** ~50-170ms (well within 60s budget)

### Key Files by Phase

| Phase | Core Module | Key Files |
|-------|-------------|-----------|
| 1 | `forecaster_model/` | `models/forecaster_model.py`, `config/__init__.py` |
| 2 | `decision_engine/` | `trigger_engine.py`, `bar_event_trigger.py`, `run_step.py` |
| 3 | `risk_engine/` | `engine.py`, `canonical_sizing.py`, `signing.py` |

---

## Phase 1: Price Prediction

### Purpose
Generate probabilistic price forecasts (quantiles) at multiple horizons using a deep learning pipeline.

### Input Contract
```python
# Market snapshot (x_obs = observed history)
x_obs: np.ndarray          # Shape: [L=128, F_obs]
                           # 128 past timesteps of observed features
                           # F_obs = number of price/volume features

# Known future (x_known = exogenous features)
x_known: np.ndarray        # Shape: [H=8, Fk]
                           # 8 future timesteps of known exogenous data
                           # (e.g., calendar features, external signals)

# Market regime
r_cur: np.ndarray          # Shape: [4]
                           # 4-dimensional regime vector
                           # (trend strength, volatility, conviction, etc.)
```

### Output Contract
```python
# ForecastPacket
{
  "q_low": list[float],    # 10th percentile [8 values, one per horizon step]
  "q_med": list[float],    # 50th percentile (median)
  "q_high": list[float],   # 90th percentile
  "confidence_score": float, # [0, 1] overall forecast quality
}
```

### Pipeline Steps

#### Step 1: Variable Selection Network (VSN) — Spec §10
**Purpose:** Learn feature importance at each timestep

**Process:**
```
For each timestep t in [0, L):
  gates[t] = softmax(W @ x_obs[t])        # [F_obs] → [F_obs]
  x_vsn[t] = gates[t] ⊙ x_obs[t]         # element-wise multiply
```

**Parameters:**
- `W`: [F_obs, F_obs] learned weight matrix
- Softmax ensures gates ∈ [0, 1] and sum to 1 (probability distribution)

**Output:** `x_vsn` [L, F_obs], `gates` [L, F_obs] (for interpretability)

---

#### Step 2: Latent Encoder (Causal 1D CNN) — Spec §11
**Purpose:** Compress features into latent space while preserving causality (no future leak)

**Architecture:** 3-layer CNN stack

| Layer | Input | Kernel k | Channels Out | Activation |
|-------|-------|----------|-------------|------------|
| 1 | x_vsn [L, F_obs] | 3 | 32 | tanh |
| 2 | h1 [L, 32] | 5 | 64 | tanh |
| 3 | h2 [L, 64] | 7 | 32 (latent_dim) | tanh |

**Causal Padding:** For each layer, prepend (k-1) zero rows to input before windowing.

**Process per Layer:**
```
For each timestep t in [0, L):
  window = padded[t:t+k, :].flatten()     # k × C_in window
  h[t] = tanh(window @ W)                 # [k×C_in] @ [k×C_in, C_out] → [C_out]
```

**Output:** `z_seq` [L, 32] (latent representation)

---

#### Step 3: Multi-Resolution xLSTM — Spec §12
**Purpose:** Capture temporal patterns at multiple timescales

**Configuration:**
```
scales = (1, 5, 20, 100)
hidden_dim = 128
```

**Process for Each Scale:**

1. **Downsample** z_seq:
   - Scale 1: use all timesteps [128 → 128]
   - Scale 5: sample every 5th [128 → ~26]
   - Scale 20: sample every 20th [128 → ~6]
   - Scale 100: sample every 100th [128 → 1-2]

2. **LSTM Cell** (standard, NumPy baseline):
   ```
   For each timestep in downsampled sequence:
     xh = concat(x_t, h_{t-1})            # [Fin + hidden_dim]
     i_gate = sigmoid(xh @ W_i)           # input gate
     f_gate = sigmoid(xh @ W_f)           # forget gate
     o_gate = sigmoid(xh @ W_o)           # output gate
     c_tilde = tanh(xh @ W_c)             # candidate cell
     
     c_t = f_gate * c_{t-1} + i_gate * c_tilde
     h_t = o_gate * tanh(c_t)
   ```

3. **Upsample** back to original length L:
   - Nearest-neighbor repeat-align
   - Output: `branches[scale]` [L, 128]

**Output:** `branches` dict with 4 entries, each [L, 128]

---

#### Step 4: Regime-Conditioned Fusion — Spec §13
**Purpose:** Adaptively weight branches based on current market regime

**Process:**
```
alpha = softmax(r_cur @ W)               # [4] regime vector → [4] weights
                                         # W: [4, 4] learned matrix

fused = zeros([L, 128])
for i, scale in enumerate(sorted_scales):
  fused += alpha[i] * branches[scale]    # weighted sum across all timesteps
```

**Output:** 
- `fused` [L, 128] (adaptive blend of all branches)
- `alpha` [4] (branch importance weights)
- `h_last = fused[-1]` [128] (final timestep, market state summary)

---

#### Step 5: Quantile Decoder — Spec §14
**Purpose:** Generate probabilistic price forecasts at 3 quantiles for H=8 horizon steps

**Process for Each Forecast Step h in [0, H):**
```
inp = concat(h_last, x_known[h])         # [128 + Fk]
q = inp @ W                              # [128+Fk] @ [128+Fk, 3] → [3]

# Enforce monotonic ordering: q_low ≤ q_med ≤ q_high
med = q[1]
out[h, 0] = med - exp(q[0])             # q_low = median - positive spread
out[h, 1] = med                          # q_med = median
out[h, 2] = med + exp(q[2])             # q_high = median + positive spread

# Ensure ordering with sort
sorted_output = sort(out[h])
```

**Output:** `y_hat_q` [H=8, Qn=3]

### Configuration
```python
# forecaster_model/config/__init__.py
@dataclass
class ForecasterConfig:
    base_interval_seconds: int = 60
    history_length: int = 128             # L
    forecast_horizon: int = 8             # H
    quantiles: tuple = (0.1, 0.5, 0.9)   # q_low, q_med, q_high
    feature_windows: tuple = (4, 16, 64)
    num_regime_dims: int = 4
    latent_width: int = 32
    recurrent_hidden_width: int = 128
    branch_scales: tuple = (1, 5, 20, 100)
```

### Reference Implementation
- **File:** `forecaster_model/models/forecaster_model.py`
- **Class:** `ForecasterModel`
- **Method:** `forward(x_obs, x_known, r_cur) → dict`

---

## Phase 2: Decision Handling

### Purpose
Convert probabilistic forecasts into trading signals and action proposals using a multi-stage trigger framework.

### Input Contract
```python
# From Phase 1
ForecastPacket: {
  "q_low", "q_med", "q_high", "confidence_score"
}

# From canonical state (regime, health)
CanonicalStateOutput: {
  "regime_probabilities",     # probability of each regime
  "heat_score",               # systemic risk [0, 1]
  "novelty",                  # out-of-distribution flag [0, 1]
  "reflexivity_score",        # self-reference [0, 1]
  "degradation",              # system health level
  ...
}

# Feature row (market microstructure)
feature_row: dict[str, float]
  - "funding_rate"
  - "funding_rate_zscore"
  - "spread_bps"
  - "volume"
  - ...
```

### Output Contract
```python
# TriggerOutput
{
  "setup_valid": bool,
  "setup_score": float,        # [0, 1]
  "pretrigger_valid": bool,
  "pretrigger_score": float,   # [0, 1]
  "trigger_valid": bool,
  "trigger_type": str,         # imbalance_spike | volume_burst | structure_break | composite
  "trigger_strength": float,   # [0, 1]
  "trigger_confidence": float, # [0, 1]
  "missed_move_flag": bool,
  "trigger_reason_codes": list[str]
}

# ActionProposal (if trigger_valid)
{
  "symbol": str,
  "route_id": RouteId,         # SCALPING | INTRADAY | SWING | CARRY
  "direction": int,            # +1 (long) | -1 (short) | 0 (flat)
  "size_fraction": float,      # [0, 1] fraction of max slot
  "stop_distance_pct": float,  # e.g., 0.012 = 1.2%
  "order_type": str,           # market | limit
  "expiry_seconds": int
}
```

### Stage 1: Setup Validation

**Purpose:** Initial viability check (model agrees, market aligned, system healthy)

**Computation:**
```
A = asymmetry_score(ForecastPacket)      # [0, 1]
    # Directional bias from quantiles
    # A = 0 → symmetric (no edge)
    # A → 1 → skewed (strong edge)

S = state_alignment_score(apex)          # [0, 1]
    # Regime probability concentration
    # S → 1 when regime is certain

C = confidence_score(apex, ForecastPacket)
    # Blend of forecast quality + model agreement
    
H = apex.heat_score                      # [0, 1]
    # Systemic risk (volatility, drawdown proximity)
    
N = apex.novelty                         # [0, 1]
    # OOD detection (market breaking)
    
Rfx = apex.reflexivity_score             # [0, 1]
    # Self-referential behavior

setup_raw = 0.35×A + 0.25×S + 0.30×C - 0.35×H - 0.35×N - 0.25×Rfx
setup_score = clip(setup_raw, [0, 1])
```

**Thresholds:**
```
setup_threshold = 0.22        # default (injectable from settings)
setup_exec_floor = 0.12       # execution confidence floor

Check: setup_score >= setup_threshold
```

**Failure Codes:**
- `TRG_LOW_SETUP_SCORE` → setup_score < threshold
- `TRG_NOVELTY_BLOCK` → novelty >= 0.98 (hard gate)
- `TRG_DEGRADATION_BLOCK` → system degraded

---

### Stage 2: Pre-Trigger Validation

**Purpose:** Data freshness & execution quality check

**Computation:**
```
spread_stress = clip(spread_bps / 80, [0, 1])
exec_conf = clip(1 - spread_stress × 0.8, [0, 1])

data_age_penalty = data_age_seconds / stale_threshold
pretrigger_score = setup_score - data_age_penalty

Check: pretrigger_score >= pretrigger_threshold (default: 0.18)
       AND exec_conf > setup_exec_floor
       AND data freshness OK (< 300 seconds)
```

**Failure Codes:**
- `TRG_STALE_PRETRIGGER_INPUTS` → data too old
- `TRG_POOR_EXECUTION_CONTEXT` → spread too wide or exec_conf low

---

### Stage 3: Confirmed Trigger

**Purpose:** Final confirmation; determine trade type and direction

**Computation:**
```
trigger_strength = magnitude(asymmetry_score)
trigger_confidence = setup_score    # reuse from stage 1
trigger_type = classify_by_microstructure(features)
  # Returns: imbalance_spike | volume_burst | structure_break | composite

Check: trigger_strength >= confirm_threshold (default: 0.2)
       AND trigger_confidence >= confirm_exec_floor (default: 0.1)
       AND trigger_valid checks pass
```

**Direction Logic:**
```
If setup_score HIGH (>= 0.35):
  direction = sign(q_med - q_low)
  # If q_med > q_low → more downside risk → we buy (long)
  # If q_med < q_low → more upside risk → we sell (short)
```

**Failure Codes:**
- `TRG_TRIGGER_STRENGTH_LOW` → strength < threshold
- `TRG_MOVE_ALREADY_EXTENDED` → entry window closed
- `TRG_INSUFFICIENT_REMAINING_EDGE` → risk/reward unfavorable

---

### Route Selection & Proposal Building

**After trigger_valid = True:**

```python
route_decision = evaluate_route(trigger_output, apex_state)
# Returns: RouteId in [SCALPING, INTRADAY, SWING, CARRY, NO_TRADE]

# Per-route action proposal (e.g., SCALPING):
ActionProposal(
  symbol=symbol,
  route_id=RouteId.SCALPING,
  direction=+1 if asymmetry > 0.55 else -1,
  size_fraction=0.35,                 # tunable per route
  stop_distance_pct=0.012,            # 1.2% hard stop
  order_type="market",
  expiry_seconds=300                  # close if unfilled in 5 min
)
```

**CARRY Route Example:**
```python
funding_signal = abs(funding_rate_zscore) / 4.0
if funding_signal > funding_threshold:
  ActionProposal(
    route_id=RouteId.CARRY,
    direction=sign(funding_rate),
    size_fraction=0.25,
    stop_distance_pct=0.025,
    expiry_seconds=3600
  )
```

### Configuration
```python
# decision_engine/trigger_engine.py
@dataclass(frozen=True)
class TriggerThresholds:
    setup_threshold: float = 0.22
    setup_exec_floor: float = 0.12
    pretrigger_threshold: float = 0.18
    freshness_floor: float = 0.08
    confirm_threshold: float = 0.2
    confirm_exec_floor: float = 0.1
    entry_extension_limit: float = 0.85
    min_remaining_edge: float = 0.03
```

### Reference Implementation
- **Files:**
  - `decision_engine/trigger_engine.py` — 3-stage evaluation
  - `decision_engine/bar_event_trigger.py` — event subscription
  - `carry_sleeve/engine.py` — carry trade logic
- **Key Functions:**
  - `evaluate_trigger()` → TriggerOutput
  - `BarDecisionTrigger` → publishes decision events

---

## Phase 3: Trade Execution

### Purpose
Apply risk controls, compute position sizing, and execute orders through regulated channels.

### Input Contract
```python
# From Phase 2
ActionProposal: {
  "symbol", "route_id", "direction", "size_fraction",
  "stop_distance_pct", "order_type", "expiry_seconds"
}

# Market & account state
{
  "mid_price": float,
  "spread_bps": float,
  "data_timestamp": datetime,
  "feed_last_message_at": datetime,
  "position_signed_qty": Decimal,
  "available_cash_usd": float,
  "portfolio_equity_usd": float,
  "current_total_exposure_usd": float,
}

# Risk state
RiskState: {
  "mode": SystemMode,                 # NORMAL | MAINTENANCE | FLATTEN_ALL | PAUSE_NEW_ENTRIES | REDUCE_ONLY
  "canonical_size_multiplier": float,
  "risk_asymmetry_score": float,
  "risk_trigger_confidence": float,
  "risk_heat_score": float,
  ...
}
```

### Output Contract
```python
# TradeAction (if approved) OR None (if blocked)
{
  "symbol": str,
  "side": str,                         # "buy" | "sell"
  "quantity": Decimal,
  "order_type": str,                   # "market" | "limit"
  "limit_price": Decimal | None,
  "stop_price": Decimal | None,
  "time_in_force": str,               # "gtc" | "ioc" | "fok"
  "route_id": RouteId,
}

# Plus updated RiskState with diagnostics
RiskState: {
  "last_risk_block_codes": list[str], # empty if approved
  "last_risk_sizing": dict,           # sizing diagnostics
  ...
}
```

### Stage 1: Hard Risk Gates (Precedence Order)

Each gate is evaluated; **first match blocks execution.**

| # | Gate | Condition | Block Code |
|---|------|-----------|-----------|
| 1 | Feed Stale | feed_last_message_at age > risk_stale_data_seconds (300s) | `RISK_BLOCK_FEED_STALE` |
| 2 | Data Timestamp Stale | data_timestamp age > risk_stale_data_seconds | `RISK_BLOCK_DATA_TIMESTAMP_STALE` |
| 3 | Spread Too Wide | spread_bps > risk_max_spread_bps (50 bps) | `RISK_BLOCK_SPREAD_WIDE` |
| 4 | Drawdown Limit | current_drawdown% > risk_max_drawdown_pct (15%) | `RISK_BLOCK_DRAWDOWN` |
| 5 | Product Untradable | product_tradable == False | `RISK_BLOCK_PRODUCT_UNTRADABLE` |
| 6 | Data Integrity | data_integrity_alert == True | `RISK_BLOCK_DATA_HEALTH` |
| 7 | System Mode | MAINTENANCE mode | `RISK_BLOCK_MAINTENANCE` |
| 8 | Proposal Validity | proposal is None | `RISK_BLOCK_NO_PROPOSAL` |

**Mode-Specific Checks** (if mode != NORMAL):

- **FLATTEN_ALL:** Only allow sells (close all positions)
- **PAUSE_NEW_ENTRIES:** Block new entries, allow exits
- **REDUCE_ONLY:** Only allow trades that reduce position

---

### Stage 2: Canonical Sizing (Multiplier Stack)

**Base Calculation:**
```
base_notional = size_fraction × max_per_symbol_usd
Example: 0.35 × $5,000 = $1,750
```

**Multiplier Stack (Layered):**

#### A. Degradation Multiplier
```
composite_m ∈ [0.0, 1.0]
Examples:
  - NORMAL: 1.0×
  - DEFENSIVE: 0.65×
  - REDUCED: 0.40×
  - NO_TRADE: 0.0× (blocks)

after_degradation = base × composite_m
```

#### B. Position Inertia Penalty
```
If flipping position direction:
  position_fraction = abs(position_notional) / equity
  inertia_m = max(0.35, 1 - 0.55 × position_fraction)
Else:
  inertia_m = 1.0

after_inertia = after_degradation × inertia_m
```

#### C. Asymmetry Boost
```
If asymmetry > 0.55 AND trigger_confidence >= 0.22 AND exec_conf >= 0.18
  AND heat < 0.75 AND reflexivity < 0.82:
  
  boost = min(1.2, 1.0 + (asymmetry - 0.5) × 0.85)
Else:
  boost = 1.0

after_boost = after_inertia × boost
```

#### D. Liquidation Mode Multiplier
```
mode = classify_liquidation_mode(trigger_conf, heat, asymmetry, atr, degradation)
       Returns: "offense" (1.05×) | "defense" (0.88×) | "neutral" (1.0×)

after_liquidation = after_boost × mode_multiplier
```

#### E. Edge Budget Multiplier
```
stress = 0.45×heat + 0.35×exposure_frac + 0.2×symbol_exposure_frac
edge_m = max(0.35, 1 - 0.62 × stress)

after_edge = after_liquidation × edge_m
```

#### F. Concentration Multiplier
```
symbol_frac = (existing_notional + proposed) / equity
book_frac = total_exposure / max_total

If symbol_frac > 0.38:
  sym_m = max(0.25, 1 - (symbol_frac - 0.38) × 2.2)
Else:
  sym_m = 1.0

If book_frac > 0.82:
  book_m = max(0.4, 1 - (book_frac - 0.82) × 3.0)
Else:
  book_m = 1.0

concentration_m = sym_m × book_m
final_notional = after_edge × concentration_m
```

### Stage 3: Quantity Conversion & Final Checks

```python
quantity = round(final_notional / mid_price, 8)

# Checks:
if quantity <= 0:
  Block: RISK_BLOCK_QTY_ZERO
  
if side == BUY and notional > available_cash_usd:
  Block: RISK_BLOCK_AVAILABLE_CASH

if mode == REDUCE_ONLY:
  qty = min(qty, abs(position_signed_qty))
  if qty <= 0:
    Block: RISK_BLOCK_REDUCE_ONLY_QTY
```

### Stage 4: TradeAction Construction

```python
TradeAction(
  symbol=proposal.symbol,
  side="buy" if proposal.direction > 0 else "sell",
  quantity=quantity,
  order_type=proposal.order_type,
  limit_price=None,
  stop_price=None,
  time_in_force="gtc",
  route_id=proposal.route_id,
)
```

### Stage 5: Order Intent Signing & Execution

```python
# Convert TradeAction → OrderIntent
OrderIntent(
  symbol=symbol,
  side=OrderSide.BUY / OrderSide.SELL,
  quantity=quantity,
  order_type=OrderType.MARKET,
  time_in_force=TimeInForce.GTC,
  client_order_id=f"{route_id}-{timestamp}",
  metadata={
    "route_id": route_id.value,
    "trigger_reason": trigger_output.trigger_type,
    "trigger_confidence": trigger_output.trigger_confidence,
    "stop_distance_pct": proposal.stop_distance_pct,
  }
)

# Sign & submit
signed_intent = sign_order_intent(order_intent, private_key)
response = execution_adapter.place_order(signed_intent)

# Update position state
position_signed_qty += quantity_transacted
current_equity -= notional (if buy)
```

### Configuration
```yaml
# app/config/default.yaml
risk:
  risk_stale_data_seconds: 300
  risk_max_spread_bps: 50
  risk_max_drawdown_pct: 0.15
  risk_max_per_symbol_usd: 5000
  risk_max_total_exposure_usd: 10000

apex_canonical:
  domains:
    risk_sizing:
      quantile_asymmetry_boost_cap: 1.2
      carry_asymmetry_boost_cap: 1.15
      position_inertia_penalty_weight: 0.55
      edge_budget_weight_heat: 0.45
      edge_budget_weight_exposure: 0.35
      edge_budget_strength: 0.62
      symbol_concentration_threshold: 0.38
      symbol_concentration_strength: 2.2
      book_concentration_threshold: 0.82
      book_concentration_strength: 3.0
```

### Reference Implementation
- **Files:**
  - `risk_engine/engine.py` — RiskEngine.evaluate()
  - `risk_engine/canonical_sizing.py` — compute_canonical_notional()
  - `risk_engine/signing.py` — order signing
  - `execution/execution_logic.py` — order placement
- **Key Functions:**
  - `RiskEngine.evaluate()` → TradeAction | None
  - `compute_canonical_notional()` → CanonicalNotionalResult

---

## Data Flow Contracts

### Events & Messaging

```
1. market.bar.closed.v1
   Payload: BarClosedEvent { symbol, ts, open, high, low, close, volume }
   
2. decision.completed.v1
   Payload: DecisionRecord { symbol, timestamp, trigger, proposal, action, ... }
   
3. order.submitted.v1
   Payload: { order_id, symbol, side, quantity, status, ... }
   
4. order.filled.v1
   Payload: { order_id, fill_price, fill_qty, commission, ... }
```

### Contract Types (Pydantic)

| Type | File | Responsibility |
|------|------|-----------------|
| `ForecastPacket` | `app/contracts/forecast_packet.py` | Phase 1 output |
| `TriggerOutput` | `app/contracts/trigger.py` | Phase 2 stage results |
| `ActionProposal` | `app/contracts/decisions.py` | Phase 2 → 3 handoff |
| `TradeAction` | `app/contracts/decisions.py` | Phase 3 output |
| `OrderIntent` | `app/contracts/orders.py` | Exchange execution |
| `DecisionRecord` | `app/contracts/decision_record.py` | Audit trail |

---

## Design Constraints & Assumptions

### Determinism & Replay
- All three phases are **purely deterministic** given inputs
- No randomness post-initialization (seed only for numpy generators during weight sampling)
- Every decision is **replayable** (DecisionRecord contains full inputs)
- Phase 1 can be run offline; Phases 2–3 must be live

### Latency Budget
```
Phase 1 (inference):    15–50ms   (model forward pass)
Phase 2 (trigger):      5–20ms    (threshold evaluation)
Phase 3 (risk+exec):    30–100ms  (API roundtrip)
───────────────────────────────────
Total:                  50–170ms  (<<< 60s bar interval)
```

### State Management
- **Single-threaded event loop** (bar-close events serialized)
- **Position state** updated only after order confirmation
- **Equity tracking** via mark-to-market on each bar
- **Risk state** carries full diagnostics for every decision

### Execution Model
- **Primary:** Paper trading (Alpaca)
- **Optional:** Live trading (Coinbase) — requires explicit keys + mode flag
- **Risk is final:** Even System 2 (LLM agent, if added) routes through RiskEngine
- **No skipping gates** (--no-verify pattern forbidden)

### Data Assumptions
1. Market data is 60-second candles (OHLCV)
2. Funding rate (for carry) available where applicable
3. Spreads < 100 bps (panic threshold)
4. Exchange latency < 500ms (typical for Alpaca/Coinbase)

---

## Monitoring & Observability

### Key Metrics by Phase

#### Phase 1: Prediction
```
tb_forecast_latency_ms
tb_forecast_confidence_avg
tb_forecast_q_spread                    # q_high - q_low
tb_forecast_ood_score
```

#### Phase 2: Decision
```
tb_trigger_setup_score_avg
tb_trigger_pretrigger_score_avg
tb_trigger_valid_rate                   # % of bars where trigger fires
tb_trigger_type                         # histogram
tb_asymmetry_score_avg
```

#### Phase 3: Risk & Execution
```
tb_risk_blocks_total                    # by block code
tb_order_submission_latency_ms
tb_order_fill_rate                      # filled / submitted
tb_order_slippage_bps
tb_edge_budget_headroom
tb_position_concentration
tb_drawdown_pct
```

### Decision Record Output
```json
{
  "symbol": "BTC/USD",
  "timestamp": "2026-06-03T12:34:56Z",
  "forecast": {
    "q_low": [99.50, ...],
    "q_med": [100.00, ...],
    "q_high": [100.50, ...],
    "confidence_score": 0.72
  },
  "trigger": {
    "setup_valid": true,
    "setup_score": 0.45,
    "trigger_valid": true,
    "trigger_type": "imbalance_spike",
    "reason_codes": []
  },
  "proposal": {
    "route_id": "SCALPING",
    "direction": 1,
    "size_fraction": 0.35
  },
  "risk": {
    "status": "APPROVED",
    "last_risk_block_codes": [],
    "sizing_diagnostics": {
      "base_notional_usd": 1750.0,
      "final_notional_usd": 942.73
    }
  },
  "action": {
    "symbol": "BTC/USD",
    "side": "buy",
    "quantity": "9.4273"
  }
}
```

### Audit Trail
- **All decisions stored** in QuestDB (immutable log)
- **Reason codes** (FB-CAN-063) for every block/approval
- **Diagnostics snapshots** for sizing internals
- **Run binding** (FB-CAN-077) for replay validation

---

## Glossary & Cross-Reference

| Term | Definition | Location |
|------|-----------|----------|
| ForecasterModel | Phase 1 core (VSN→CNN→xLSTM→Fusion→Decoder) | `forecaster_model/models/forecaster_model.py` |
| TriggerEngine | Phase 2 evaluation (3-stage) | `decision_engine/trigger_engine.py` |
| RiskEngine | Phase 3 gating + sizing | `risk_engine/engine.py` |
| ActionProposal | Trade intent before risk (Phase 2 output) | `app/contracts/decisions.py` |
| TradeAction | Risk-approved trade (Phase 3 output) | `app/contracts/decisions.py` |
| OrderIntent | Signed order ready for exchange | `app/contracts/orders.py` |
| DecisionRecord | Full audit trail of one bar's decision | `app/contracts/decision_record.py` |
| CanonicalStateOutput | Market regime + health snapshot | `app/contracts/canonical_state.py` |
| ForecastPacket | Phase 1 output (quantiles + confidence) | `app/contracts/forecast_packet.py` |

---

## Design Iteration Areas

> This section tracks potential refactoring / design exploration targets

### Alternative Phase 1 Designs
- [ ] Transformer-based forecaster (vs. xLSTM)
- [ ] Conformal prediction calibration
- [ ] Ensemble methods (multiple models)

### Alternative Phase 2 Designs
- [ ] Machine-learned trigger thresholds (vs. hand-tuned)
- [ ] LLM reasoning layer (System 2 as per `legacy/decision_pipeline/docs/AI_TRADING_AGENT_DESIGN.MD`)
- [ ] Multi-asset regime classification

### Alternative Phase 3 Designs
- [ ] Dynamic risk limits (based on live Sharpe ratio)
- [ ] Order type optimization (market vs. limit timing)
- [ ] Execution splitting (VWAP, TWAP strategies)

---

## References

- [`docs/CANONICAL_SPEC_INDEX.MD`](CANONICAL_SPEC_INDEX.MD) — APEX system spec
- [`docs/architecture/system_walkthrough.md`](architecture/system_walkthrough.md) — detailed walkthrough
- [`legacy/decision_pipeline/docs/AI_TRADING_AGENT_DESIGN.MD`](../legacy/decision_pipeline/docs/AI_TRADING_AGENT_DESIGN.MD) — System 2 (LLM layer) design
- [`docs/architecture/risk_precedence.md`](architecture/risk_precedence.md) — risk gate ordering
- [`AGENTS.md`](../AGENTS.md) — contributor contract
- [`app/contracts/reason_codes.py`](../app/contracts/reason_codes.py) — all block/approval codes

---

**Last Updated:** 2026-06-03  
**Next Review:** After Phase 1 model update or Phase 3 risk rule change
