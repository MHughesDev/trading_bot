# APEX Decision System — Final Master Specification v1.1

**Title**: APEX — Anti-Fragile Leverage-Flow Exploitation System  
**Version**: 1.1 (Final reconciled after Grok + GPT comparison)  
**Date**: April 2026  
**Core Objective**: Maximize long-run PnL in crypto perpetual futures by detecting leverage imbalances, waiting for high-conviction triggers, and exploiting forced flows (especially liquidation cascades and funding pressure) while failing small and recovering fast.  
**Fundamental Principle**: The system is a **leverage-flow detector with tight risk controls**, not a generic price predictor. It prioritizes survival under imperfection and selective aggression when asymmetry is clear.

---

## 1. What changed after comparing both final specs

The compared specs were already highly aligned. The reconciled version below keeps the shared core and explicitly folds in the few practical tightenings that matter most:

- **Kept from both**:
  - 5-layer architecture + isolated carry sleeve
  - probabilistic signals (`value × confidence × freshness`)
  - signal decay
  - degradation hierarchy
  - 3-model forecast ensemble
  - OI structure engine
  - trade opportunity auction
  - multi-stage trigger
  - liquidation as both risk and opportunity
  - execution realism and feedback
  - no Kelly / no streak sizing / no per-trade policy updates

- **Explicitly tightened in this reconciled version**:
  - **Action under uncertainty curve**: low-confidence signals reduce size instead of causing unnecessary paralysis, except for hard override conditions
  - **Trigger minimalism**: trigger logic must stay mechanically defined and small
  - **Worst-case execution posture in stress**: cascades and dislocations default to pessimistic fill assumptions
  - **Empirical overlap handling**: signal overlap and model overlap are handled with rolling correlation / dependency tracking, not static assumptions
  - **PnL-first mitigation rule**: safeguards stay only if they improve net realized PnL, not just theoretical safety

---

## 2. System Architecture Overview

**Five Layers + One Isolated Sleeve**

- **Layer 1** — State & Safety Engine (hard overrides)
- **Layer 2** — Forecasting Engine
- **Layer 3** — Decision Policy (Trade Opportunity Auction + Trigger Layer)
- **Layer 4** — Execution Engine (realism-focused)
- **Layer 5** — Anti-Fragile Risk & Portfolio Engine
- **Separate Sleeve** — Carry Trading (fully isolated)

**Cross-Cutting Rules**
- Every signal is probabilistic: `value × confidence × freshness`
- Hard signal decay: `confidence(t) = e^(-λt)` with λ tuned by signal family
- Degradation hierarchy: `Normal → Reduced → Defensive → No-Trade`
- **Action under uncertainty curve**:
  - low confidence usually means **smaller size**, not automatic rejection
  - only hard overrides can fully block trading
- All mitigations must justify themselves by **net live PnL improvement**

---

## 3. Layer 1 — State & Safety Engine

### Purpose
Build the current market state, measure uncertainty, and enforce hard safety overrides.

### Core Components
- **Probabilistic regime classifier**  
  Soft probabilities across: trend / range / stress / dislocated / transition
- **Novelty / OOD detector**  
  Hard defensive override when the market moves outside familiar structure
- **Volatility, liquidity, and spread gates**
- **Weekend / low-liquidity throttle**
- **Reflexivity / crowding modulator**  
  Uses funding rate, funding velocity, open interest concentration, and cross-exchange pressure
- **Crypto-specific portfolio heat score**  
  Combines funding extremes, liquidation proximity, OI concentration, cross-exchange divergence, volatility, and execution stress
- **Feature reliability scoring**
- **Signal alignment & freshness layer**
- **Exchange risk awareness**  
  Outages, degraded APIs, abnormal spreads, localized venue failure
- **Signal decay awareness**  
  Applied to funding, OI, liquidation clusters, basis, and other time-sensitive inputs

### Output
- state vector
- degradation level
- override flags
- decayed signal confidences
- regime confidence / transition confidence

---

## 4. Layer 2 — Forecasting Engine

### Purpose
Estimate asymmetry, continuation, fragility, and volatility using a compact ensemble.

### Ensemble (3 models)
- **Sequential model** (CryptoMamba / SSM-style) for trend capture
- **Tabular model** (XGBoost / LightGBM-style) for range / chop
- **Volatility specialist**

### Outputs
- quantile distribution: `P5, P25, P50, P75, P95`
- volatility forecast

### Weighting
- dominant: regime prior + volatility/liquidity
- tiny damped outcome calibration:
  - default `ε = 0.03–0.05`
  - max `ε = 0.08` only in stable regimes
  - half-life `12–24h`
  - reset on regime shift or novelty event

### Crypto-Native Feature Families
- **Liquidation structure**
  - proximity to clusters
  - estimated cascade magnitude
  - treated as **probabilistic**, not deterministic
  - includes explicit decay / error bounds
- **Funding pressure**
  - rate
  - velocity
  - cross-exchange differences
- **Perp vs spot and cross-exchange basis**
- **OI Structure Engine**
  - `OI ↑ + price ↑` → healthier trend / higher continuation
  - `OI ↑ + price flat` → fragile buildup / lower asymmetry
  - `OI ↓ + price ↑` → squeeze potential / higher asymmetry
- **Local extrema prediction**
  - auxiliary only
  - strongly downweighted in chop and noisy regimes

### Data Tiers
- **Tier 1 (core)**: L2 microstructure, funding/basis/cross-exchange, liquidation structure
- **Tier 2 (conditional)**: options (GEX/skew/max pain, weighted by market depth and freshness), OI structure
- **Tier 3 (support)**: macro/calendar flags

### Deferred / Excluded for v1
- whale heuristics
- complex sentiment pipelines
- GNN / dynamic cross-asset graphs
- dedicated opponent modeling

---

## 5. Layer 3 — Decision Policy

### Purpose
Turn state + forecasts into a **small set of high-quality trades**.

### Primary Engine
**Constrained optimizer** with:
- **Trade Opportunity Auction**
- **Multi-Stage Trigger Layer**

### Trade Opportunity Auction
- generate all candidate trades
- rank by:
  - quantile asymmetry
  - state alignment
  - confidence
  - trigger strength
  - OI structure classification
  - liquidation exploitation potential
- execute only the **top N** opportunities

### Diversification Constraint
Auction must include:
- correlation penalty
- exposure balancing
- anti-clustering logic  
This prevents “top N” from becoming the same trade expressed repeatedly.

### Multi-Stage Trigger Layer
The trigger must remain **minimal, mechanical, and backtestable**.

#### Stage 1 — Setup
- asymmetry is favorable
- leverage-flow structure is aligned
- state is permissive

#### Stage 2 — Pre-Trigger
- pressure is building
- microstructure tightens
- volume / imbalance start to shift

#### Stage 3 — Confirmed Trigger
Allowed trigger types:
- order book imbalance spike
- volume expansion burst
- break of local structure / breakout confirmation

### Trigger Rules
- trigger logic must stay small
- no vague pattern soup
- trigger confidence can scale size, but only within bounded ranges
- explicit **missed move acceptance**: no chasing after poor entry location

### Hard Constraints
- novelty / degradation override
- reflexivity + heat score ceiling
- drawdown state + correlation limits
- position inertia
- model disagreement penalty + explicit confidence ceiling
- model correlation awareness
- early-trade penalty
- **Liquidation exploitation mode**
  - enabled only when strong alignment + trigger + bounded risk all agree

### v2+ Option
- constrained PPO on top, only after shadow validation

---

## 6. Layer 4 — Execution Engine

### Purpose
Protect realized edge under crypto-specific liquidity conditions.

### Core Requirements
- dynamic order selection:
  - limit vs market vs TWAP
- partial fill handling + immediate re-evaluation
- execution confidence scoring
- smart multi-exchange routing
- slippage-aware behavior

### Stress Execution Posture
In cascades, dislocations, or sudden liquidity collapse:
- assume **worse-than-expected fills**
- reduce aggressiveness
- favor capital preservation over theoretical perfect entry

### Execution Feedback Loop
Feed back:
- actual slippage
- fill quality
- venue degradation
into:
- trigger trust
- decision confidence
- execution mode selection

---

## 7. Layer 5 — Anti-Fragile Risk & Portfolio Engine

### Purpose
Bound losses, prevent clustering, and size based on asymmetry under uncertainty.

### Mechanisms
- exponential drawdown scaling
- correlation-aware limits (`≥ 0.85` treated as one position)
- exposure ceilings
- real-time CVaR
- quantile-asymmetry sizing
  - modest boost only
  - hard max around `1.20×`
  - only when right tail is strong **and** reflexivity / heat are not extreme
- reflexivity + crypto heat score as hard ceilings
- liquidation proximity influences:
  - defense when conditions are poor
  - offense only when alignment is strong and triggers confirm

### Explicitly Prohibited
- Kelly variants
- streak-based sizing
- fast performance-chasing sizing

---

## 8. Separate Sleeve — Carry Trading

- fully isolated delta-neutral funding capture
- independent risk engine
- independent sizing
- independent attribution
- used mainly when the directional book is neutral / low-conviction

---

## 9. Cross-Cutting Elements & Long-Term Safeguards

### Continuous Monitoring
- feature health
- model disagreement
- signal decay
- heat
- realized vs theoretical edge

### Scheduled Slow Retrains
- strict holdout
- shadow deployment before promotion
- no per-trade or fast policy updates

### Empirical Overlap Handling
Use rolling dependency tracking for:
- signal overlap
- model overlap
- hidden correlation  
Avoid static “these are independent” assumptions.

### Edge Budgeting
Track total deployed edge using practical proxies:
- risk
- correlation
- heat
- concentration  
Do not rely on a magical exact edge meter.

### Opportunity Cost Tracking
Used for **slow calibration only**, not real-time decisions.

### False Positive Memory
Recent failed pattern types can temporarily downweight similar setups.

### Regime Transition Guard
Special conservative handling during regime shifts:
- smaller initial size
- tighter confidence requirements
- but not a total shutdown

### Human / Audit Layer
All major changes and promotions must be:
- logged
- reviewable
- attributable

---

## 10. Implementation Roadmap (Phased)

### Phase 1 — Foundation
1. data pipeline + signal alignment + feature reliability + signal decay
2. risk engine + degradation hierarchy + crypto heat score
3. single quantile model + basic optimizer + auction skeleton
4. novelty + volatility circuit breaker + basic trigger layer

### Phase 2
5. full ensemble + tiny ε calibration + OI Structure Engine
6. reflexivity + funding pressure + liquidation (probabilistic + defensive/offensive)
7. trade opportunity auction + diversification constraint

### Phase 3
8. enhanced execution layer
9. full constraint tuning:
   - inertia
   - disagreement ceiling
   - early-trade penalty
   - liquidation exploitation bounds
   - empirical overlap penalties

### Phase 4
10. constrained PPO (optional)
11. heavier GEX usage only if data quality is proven

---

## 11. Non-Negotiable Design Principles

1. leverage-flow exploitation over generic price prediction
2. state drives structure; outcome is only a whisper
3. novelty, heat, and signal decay can trigger hard overrides
4. trade quality over trade quantity
5. liquidation is both risk and opportunity, but always probabilistic
6. execution realism is non-negotiable
7. every mitigation must earn its place through net PnL improvement
8. fail small, recover fast, debug easily

---

## 12. Success Metrics

- risk-adjusted PnL across full market cycles
- maximum drawdown control
- recovery speed
- trade quality of **executed** trades
- realized vs theoretical edge
- time in reduced / defensive / no-trade states
- concentration / overlap control effectiveness

---

## 13. Known Hard Risks (Accepted, Not Eliminated)

These cannot be removed entirely:
- imperfect trigger timing
- execution friction
- data imperfection
- regime misclassification
- edge decay

The system’s job is not to eliminate them.  
It is to **remain profitable despite them**.

---

## 14. Final System Identity

APEX v1.1 is:

- crypto-native
- leverage-flow aware
- selective
- timing-gated
- execution-realistic
- uncertainty-aware

It wins by being disciplined when others are forced.

---

## 15. Final Build Decision

This system is now **fully specified enough to build**.

The next work should not be more architecture debate.  
It should be one of these:
1. Feature schema & data contracts
2. Trigger math / pseudocode
3. Data pipeline design
4. Execution logic definition
