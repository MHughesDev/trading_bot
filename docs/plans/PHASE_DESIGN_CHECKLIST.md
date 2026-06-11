# Phase Design Checklist
## System Architecture Review & Design Iteration Template

**Purpose:** Use this checklist when evaluating alternative designs or refactoring a phase.

---

## Pre-Design Review

### Understanding Current Design
- [ ] Read `docs/SYSTEM_SPECIFICATION.md` (complete)
- [ ] Review all input/output contracts for the target phase
- [ ] Understand constraints (latency, determinism, replay)
- [ ] Check configuration parameters (defaults vs. tunable)
- [ ] Review existing tests (`tests/test_*`)
- [ ] Check monitoring metrics (what gets measured?)

### Stakeholder Alignment
- [ ] Which phase(s) does this affect?
- [ ] Does it change interfaces with adjacent phases?
- [ ] Will it require config changes?
- [ ] Does it affect determinism / replay?
- [ ] Will it require new monitoring?

---

## PHASE 1: PRICE PREDICTION
**Focus:** Accuracy, latency, calibration, novelty detection

### Inputs ✓
- [ ] `x_obs` [L=128, F_obs]: What features? How preprocessed?
- [ ] `x_known` [H=8, Fk]: Which exogenous features? Calendar? External?
- [ ] `r_cur` [4]: How is regime computed? Is this a bottleneck?

### Output Quality
- [ ] Do quantiles make sense? (q_low ≤ q_med ≤ q_high?)
- [ ] Is confidence_score calibrated? (Does 0.9 mean 90% accuracy?)
- [ ] Coverage: Do 80% of actuals fall within [q_low, q_high]?
- [ ] Spread: Is [q_high - q_low] too wide? (losing signal)

### Architecture Changes
- [ ] Replace xLSTM with Transformer? (Pros: longer context. Cons: latency)
- [ ] Add ensemble (multiple models, aggregate)?
- [ ] Add conformal prediction (wrap existing model)?
- [ ] Change quantile levels (0.1, 0.5, 0.9 → other %s)?
- [ ] Add OOD detection layer?

### Latency Impact
```
Current: 15–50ms
Target:  < 60ms (must complete in bar interval)
Measure: Profile each layer (VSN, CNN, LSTM, Fusion, Decoder)
```

### Validation
- [ ] Backtesting accuracy (Pinball loss, Quantile loss)
- [ ] Out-of-sample calibration curve
- [ ] Stress-test on regime changes (novelty high)
- [ ] Replay determinism (same input → same output)

### Configuration
```yaml
# Current defaults in forecaster_model/config/__init__.py
history_length: 128         # <- Change for longer/shorter context?
forecast_horizon: 8         # <- Need more/less lookahead?
quantiles: (0.1, 0.5, 0.9)  # <- Different percentiles?
branch_scales: (1,5,20,100) # <- Tune per market regime?
```

### Testing Checklist
- [ ] Unit: `test_forecaster_model.py` — forward pass, shapes
- [ ] Integration: Data pipeline → model → output
- [ ] Regression: Backtested accuracy on historical bars
- [ ] Stress: Regime change, data gap, feature freeze
- [ ] Replay: Load checkpoint, run, compare to audit log

---

## PHASE 2: DECISION HANDLING
**Focus:** Signal quality, false positives, threshold tuning, route selection

### Inputs ✓
- [ ] ForecastPacket: Is confidence_score the right signal?
- [ ] CanonicalStateOutput: Heat / Novelty / Reflexivity — are these tracked?
- [ ] Feature row: What microstructure features are used? (funding, volume, spreads)
- [ ] Regime: Is regime classification working? (Can we diagnose failures?)

### Trigger Stages Quality
- [ ] Stage 1 (Setup): Are weights (A, S, C, H, N, Rfx) right?
  ```
  Current: 0.35×A + 0.25×S + 0.30×C - 0.35×H - 0.35×N - 0.25×Rfx
  Question: Should H have same weight as A?
  ```
- [ ] Stage 2 (Pre-Trigger): Is exec_conf calculated correctly?
  ```
  exec_conf = 1 - spread_stress × 0.8
  When spread_bps=50, exec_conf falls to 0.5. Is this right?
  ```
- [ ] Stage 3 (Confirm): Is trigger_type classification reliable?

### False Signal Analysis
- [ ] What % of triggers result in profitable trades?
- [ ] What % of triggers result in stop-hits?
- [ ] Which trigger_type has best signal quality?
- [ ] Are there regime-specific failure modes?

### Route Selection
- [ ] Are routes well-defined? (SCALPING vs. SWING vs. CARRY)
  ```
  SCALPING:  < 5 min, tight stops (1.2%), small size (35%)
  INTRADAY:  < 4 hours, medium stops (2%), medium size (45%)
  SWING:     > 4 hours, loose stops (4%), large size (60%)
  CARRY:     Duration ∞, funding-based, independent multiplier (55%)
  ```
- [ ] Is route selection automatic or manual?
- [ ] Can routes be disabled (settings)?

### Threshold Tuning
```python
# Current in decision_engine/trigger_engine.py
TriggerThresholds(
    setup_threshold=0.22,           # <- More conservative? 0.25+?
    setup_exec_floor=0.12,          # <- Minimum exec quality
    pretrigger_threshold=0.18,      # <- Can relax if data is good?
    confirm_threshold=0.2,          # <- Too loose? Try 0.25?
    confirm_exec_floor=0.1,         # <- Too loose? Try 0.15?
)

from settings: trigger_setup_threshold = 0.22  # Injectable override
```

### Alternative Designs
- [ ] Machine-learned thresholds (vs. hand-tuned)?
- [ ] Dynamic thresholds based on regime / heat / novelty?
- [ ] Multiple decision paths (scalping vs. swing have different logic)?

### Testing Checklist
- [ ] Unit: `test_trigger_engine.py` — each stage independently
- [ ] Integration: Forecast → Trigger → Route → Proposal
- [ ] Backtesting: Hit rate, false positive rate, profit per signal
- [ ] A/B test: Original thresholds vs. new (if changing)
- [ ] Stress: Regime change, novelty spike, execution failure

---

## PHASE 3: TRADE EXECUTION
**Focus:** Risk controls, sizing fairness, slippage, position management

### Risk Gates (Precedence)
```
Current order (first match blocks):
1. Feed stale (300s)
2. Data timestamp stale (300s)
3. Spread wide (50 bps)
4. Drawdown limit (15%)
5. Product untradable
6. Data integrity
7. System mode (MAINTENANCE)
8. No proposal

Question: Should these be reordered? Is 50 bps the right threshold?
```

### Sizing Multiplier Stack
```
Current: degradation → inertia → boost → liquidation → edge_budget → concentration

base (e.g., $1,750)
  ├─ × degradation (e.g., 0.65 for DEFENSIVE)
  ├─ × inertia (penalize flips)
  ├─ × asymmetry_boost (e.g., 1.13 if edge is clear)
  ├─ × liquidation_mode (1.05 for OFFENSE, 0.88 for DEFENSE)
  ├─ × edge_budget (throttle if heat/exposure high)
  └─ × concentration (clamp if symbol or book concentrated)
     └─ final (e.g., $942.73)

Questions:
  - Are weights correct? (Should inertia come before boost?)
  - Is 55% position inertia penalty fair? (Too harsh on flips?)
  - Should boost cap be 1.2 or 1.3?
  - When to apply edge_budget (always vs. only CARRY)?
```

### Degradation Multipliers
```
Current in CanonicalStateOutput:
  NORMAL:      1.0×
  DEFENSIVE:   0.65×
  REDUCED:     0.40×
  NO_TRADE:    0.0×

Who sets this? (Canonical state machine or hard-coded?)
When should system go DEFENSIVE? (Heat > 0.7? Drawdown > 5%?)
```

### Edge Budget
```
stress = 0.45×heat + 0.35×exposure_frac + 0.2×symbol_exposure_frac
edge_m = max(0.35, 1 - 0.62 × stress)

When heat=0.8, exposure=90%, symbol=5%:
  stress = 0.36 + 0.315 + 0.01 = 0.685
  edge_m = max(0.35, 1 - 0.62×0.685) = 0.576 (42% reduction)

Is this too aggressive? Too lenient?
```

### Alternative Designs
- [ ] Dynamic risk limits (based on live Sharpe ratio)?
- [ ] Machine-learned multiplier stack (vs. hand-tuned)?
- [ ] Per-symbol risk models (crypto vs. equities)?
- [ ] Order splitting (VWAP/TWAP instead of market)?
- [ ] Smart order routing (multiple venues)?

### Configuration Audit
```yaml
# In app/config/default.yaml
risk:
  risk_stale_data_seconds: 300       # <- Right timeout?
  risk_max_spread_bps: 50            # <- Too strict? 75?
  risk_max_drawdown_pct: 0.15        # <- 15% — agree?
  risk_max_per_symbol_usd: 5000      # <- Per asset limit
  risk_max_total_exposure_usd: 10000  # <- Portfolio limit
```

### Execution Quality Metrics
- [ ] Slippage: (fill_price - mid_price) / mid_price (bps)
- [ ] Fill rate: filled_qty / attempted_qty (%)
- [ ] Latency: order_submit_ts → fill_ts (ms)
- [ ] Position concentration: (position_notional / equity) (%)
- [ ] Edge budget headroom: 1 - edge_stress (%)

### Testing Checklist
- [ ] Unit: `test_risk_engine.py` — each gate, sizing steps
- [ ] Integration: Proposal → Risk → TradeAction → Execution
- [ ] Backtesting: Drawdown control, position limits, risk blocks
- [ ] Live shadow: Run in paper, compare to production
- [ ] Stress: Liquidation cascade, circuit breaker, recovery

---

## Cross-Phase Concerns

### Determinism & Replay
- [ ] Phase 1: Same input → same forecast (deterministic)
- [ ] Phase 2: Same trigger inputs → same proposal (deterministic)
- [ ] Phase 3: Same risk state → same sizing (deterministic)
- [ ] Can we replay a bar and get identical outputs?
- [ ] Are there any sources of randomness? (Check random.*, np.random.*)

### Latency Budget
```
Phase 1: 15–50ms  (acceptable: 20ms target)
Phase 2:  5–20ms  (acceptable: 10ms target)
Phase 3: 30–100ms (acceptable: 50ms target due to API)
────────────────────────────────
Total:   50–170ms (must fit in 60s bar interval)

If Phase 1 gets slower (e.g., ensemble), can we accept 30–70ms?
If Phase 3 gets slower (e.g., smart order routing), acceptable?
```

### Monitoring & Observability
- [ ] Are there metrics for each phase?
- [ ] Can we detect failures in production?
- [ ] Are decision records complete and queryable?
- [ ] Can we replicate a decision offline?

### Configuration Management
- [ ] Are all tunables in config (vs. hard-coded)?
- [ ] Can we change thresholds without code deploy?
- [ ] Is config version tracked in decision records?
- [ ] Can we A/B test different configs?

---

## Design Proposal Template

When proposing a design change, fill out:

```markdown
### Phase X Design Iteration: [Title]

**Problem Statement:**
- What's broken or suboptimal?
- Why does it matter? (PnL impact, reliability, latency?)

**Current Design:**
- [Copy relevant section from SYSTEM_SPECIFICATION.md]

**Proposed Design:**
- [Describe change]
- [Explain why better]

**Trade-offs:**
| Aspect | Current | Proposed | Impact |
|--------|---------|----------|--------|
| Latency | 20ms | 25ms | +5ms acceptable? |
| Accuracy | 72% | 75% | +3% win |
| Complexity | Simple | Complex | Hard to debug? |

**Implementation Plan:**
1. [ ] Update code
2. [ ] Add tests
3. [ ] Run backtest (X% improvement expected)
4. [ ] Paper trade (Y days)
5. [ ] Review decision records
6. [ ] Live (if confident)

**Success Criteria:**
- [ ] No increase in risk blocks
- [ ] Accuracy improves by >= 2%
- [ ] Latency stays < 60ms
- [ ] Determinism preserved (replay identical)

**Rollback Plan:**
- How to revert if it breaks?
- What's the manual override?
```

---

## Approved Changes Tracker

**[To be updated as designs are iterated]**

| Date | Phase | Change | Author | Status |
|------|-------|--------|--------|--------|
| | | | | |

---

## References

- Main spec: [`docs/SYSTEM_SPECIFICATION.md`](SYSTEM_SPECIFICATION.md)
- Code: `forecaster_model/`, `decision_engine/`, `risk_engine/`
- Tests: `tests/test_*.py`
- Metrics: `observability/metrics.py`, `docs/MONITORING_CANONICAL.MD`
