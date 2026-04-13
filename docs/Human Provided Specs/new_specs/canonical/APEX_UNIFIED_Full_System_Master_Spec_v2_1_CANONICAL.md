# APEX — Unified Full-System Master Specification v2.1 (Canonical)

**Document Type**: Unified Full-System Master Specification  
**Scope**: Entire trading system, including data, features, state, forecasting, trigger/timing, decisioning, risk, execution, carry, monitoring, replay, governance, and research controls  
**Version**: 2.1  
**Date**: April 2026  
**Status**: Canonical  
**Primary Objective**: Maximize long-run PnL in crypto perpetual futures through selective exploitation of leverage-flow imbalances, disciplined risk control, robust execution, and bounded adaptation.

---

## 1. Executive Summary

APEX is a **crypto-native leverage-flow exploitation system**. It is not designed around generic fair-value forecasting. Its core belief is that crypto perpetual futures are primarily shaped by:

- leverage buildup and unwind
- forced liquidation flows
- funding pressure and positioning asymmetry
- perp/spot basis distortions
- exchange fragmentation and liquidity cliffs
- microstructure-confirmed acceleration

The system therefore seeks to answer:

1. Where is leverage concentrated?
2. Where is the market structurally fragile?
3. When is forced flow likely?
4. Has the move actually started?
5. Is execution quality sufficient to monetize the edge?
6. Is this trade better than the other available trades?
7. Should the system reduce aggression or stop?

---

## 2. Final System Identity and Philosophy

### 2.1 Identity
APEX is:

> A constrained opportunistic leverage-flow exploitation system for crypto perpetual futures.

### 2.2 Core Principles
1. Leverage-flow exploitation over generic price prediction  
2. State drives structure  
3. Outcome only whispers for calibration  
4. Every signal is uncertain  
5. Timing is gated, not assumed  
6. Trade quality dominates trade count  
7. Execution realism is non-negotiable  
8. Mitigations must improve realized PnL, not just theoretical safety  
9. Low confidence should usually reduce size, not create paralysis  
10. Hard overrides exist only for genuine danger conditions  
11. Every material behavior must be explainable, testable, and replayable

### 2.3 Primary Edge Sources
- liquidation structure
- OI structure
- funding pressure
- perp vs spot / basis
- cross-exchange dislocations
- L2 microstructure and trigger confirmations
- execution quality differentials
- session/liquidity conditions
- selective opportunity ranking

### 2.4 Explicit Rejections
The system explicitly rejects or defers:
- hierarchical RL for v1
- dedicated opponent modeling as core
- Kelly sizing variants
- streak-based size boosts
- fast performance-chasing adaptation
- fully autonomous per-trade learning
- heavyweight sentiment as a primary driver
- complexity that improves “theory” but not realized PnL

---

## 3. Full System Architecture

```text
Data Ingestion
→ Data Normalization
→ Feature Engineering
→ State & Safety
→ Forecast / Structure
→ Trigger / Timing
→ Decision / Opportunity Auction
→ Risk / Portfolio Constraints
→ Execution Guidance
→ Trade Intent Output
→ Execution Feedback
→ Memory / Adaptation / Monitoring
```

### 3.1 Major Domains
1. Market Data Domain  
2. Structural Data Domain  
3. Feature Engineering Domain  
4. State & Safety Domain  
5. Forecast / Structure Domain  
6. Trigger / Timing Domain  
7. Decision / Opportunity Selection Domain  
8. Risk / Portfolio Domain  
9. Execution Domain  
10. Carry Sleeve Domain  
11. Feedback / Adaptation Domain  
12. Replay / Simulation Domain  
13. Monitoring / Governance Domain

---

## 4. Canonical Runtime Pipeline

```text
Raw Inputs
→ Normalize / Validate / Timestamp
→ Build effective signals
→ Build state snapshot
→ Generate structure forecast
→ Evaluate trigger stages
→ Generate candidate trades
→ Run opportunity auction
→ Apply constraints and sizing
→ Emit trade intent / suppression / no-trade
→ Collect execution feedback
→ Update memory and metrics
```

### 4.1 Multi-Speed Runtime
- Fast loop (1–5 sec): safety, degradation, exchange risk, heat, spread/liquidity collapse
- Medium loop (10–60 sec): trigger checks, microstructure refresh, execution quality refresh, candidate refresh
- Slow loop (1–5 min): structure/forecast refresh, quantiles, OI structure, funding/basis state
- Very slow loop (hours–days): calibration review, memory decay, drift review, shadow comparison

---

## 5. Data Domain

### 5.1 Required Data Families
Core market data:
- trades
- best bid/ask
- order book summaries
- depth measures
- spread
- OHLCV (if derived internally)

Structural data:
- open interest
- OI deltas
- funding rates
- funding velocity
- perp vs spot basis
- cross-exchange divergence
- liquidation structure
- cascade estimates if available

Context:
- session/weekend indicators
- venue health / exchange risk
- options context when reliable
- macro/calendar flags
- optional stablecoin flow proxies

### 5.2 Data Principles
- multi-source redundancy should improve confidence, not freeze the system
- all timestamps explicit and UTC
- every time-sensitive field must carry freshness meaning
- stale data should degrade confidence, not create undefined behavior
- no exotic signal may become a hidden single point of failure

### 5.3 Universal Signal Rule
```text
effective_signal = normalized_value × confidence × freshness
```

### 5.4 Decay Rule
```text
decayed_confidence(t) = base_confidence × exp(-λ × age)
```

---

## 6. Feature Engineering Domain

### 6.1 Core Feature Families
Liquidation:
- cluster density
- cluster asymmetry
- cluster proximity
- cascade magnitude estimate
- liquidation confidence / freshness

OI structure:
- OI level
- OI delta
- OI/price interaction class
- OI concentration
- exchange leverage skew if available

Funding:
- funding rate
- funding z-score
- funding velocity
- funding persistence
- funding cross-exchange spread

Basis / perp-spot:
- basis bps
- basis velocity
- perp/spot divergence score
- cross-exchange basis split

Microstructure:
- order book imbalance
- imbalance change
- near-touch depth
- spread state
- volume burst
- structure-break score

Safety/context:
- novelty/OOD
- heat score components
- weekend mode
- exchange health
- liquidity regime
- volatility regime
- transition probability

### 6.2 Rules
- all features versioned
- all features traceable to inputs
- all features support confidence/freshness application
- optional features may be absent without breaking the system

---

## 7. State, Safety, and Regime Domain

### 7.1 Required Outputs
- regime probabilities
- regime confidence
- transition probability
- novelty score
- degradation level
- reflexivity score
- crypto heat score
- exchange risk level
- liquidity and spread state
- confidence vector

### 7.2 Regime Classes
- trend
- range/chop
- stress
- dislocated
- transition/uncertain

### 7.3 Crypto Heat Score
Should include:
- funding extremity
- liquidation proximity
- OI concentration / fragility
- cross-exchange divergence
- volatility state
- execution stress
- crowding context

### 7.4 Degradation Hierarchy
- Normal
- Reduced
- Defensive
- No-Trade

### 7.5 Weekend / Low-Liquidity Throttle
Throttle, not automatic shutdown.

---

## 8. Forecasting and Structure Domain

### 8.1 Purpose
Produce:
- quantile distribution
- asymmetry
- continuation probability
- fragility
- weak directional bias
- agreement score
- model overlap penalty

### 8.2 Logical Model Roles
1. trend-sensitive sequential model  
2. chop/range-sensitive tabular model  
3. volatility specialist

### 8.3 Weak Outcome Calibration
- epsilon typically 0.03–0.05
- max 0.08
- only in stable conditions
- half-life 12–24h
- reset on regime shift or novelty

### 8.4 OI Structure Engine
At minimum:
- OI up + price up → healthier continuation
- OI up + price flat → fragile buildup
- OI down + price up → squeeze potential
- OI down + price down → deleveraging

### 8.5 Liquidation Handling
Always probabilistic, freshness-sensitive, uncertainty-bounded, and confidence-weighted.

### 8.6 Local Extrema
Auxiliary only, heavily downweighted in chop/noise.

---

## 9. Trigger and Timing Domain

### 9.1 Goal
Solve the timing problem: has the move actually started?

### 9.2 Trigger Minimalism
Must be:
- small
- mechanical
- backtestable
- replayable
- explainable

### 9.3 Multi-Stage Trigger
Stage 1 — Setup  
Stage 2 — Pre-Trigger  
Stage 3 — Confirmed Trigger

### 9.4 Allowed Trigger Families
- imbalance spike
- volume burst
- structure break
- bounded composite trigger

### 9.5 Missed Move Acceptance
The system must explicitly avoid chasing extended moves.

---

## 10. Decision and Opportunity Selection Domain

### 10.1 Candidate Generation
Generate candidates only when:
- setup valid
- no hard override
- minimum confidence met

### 10.2 Trade Opportunity Auction
Rank candidates globally by:
- asymmetry
- state alignment
- confidence
- trigger quality
- execution confidence
- OI structure
- liquidation opportunity
- penalties for overlap/diversification/edge budget

### 10.3 Diversification Constraint
Penalize:
- correlated instruments
- repeated structural thesis
- clustered liquidation dependence
- concentrated funding/crowding expression

### 10.4 Edge Budgeting
Use proxy-based throttles:
- concentration
- overlap
- heat
- exposure
- confidence-adjusted notional

---

## 11. Risk, Portfolio, and Exposure Domain

### 11.1 Position Sizing Inputs
- asymmetry
- confidence
- trigger confidence
- degradation level
- execution confidence
- heat
- book concentration
- edge budget state

### 11.2 Position Inertia
A max position delta or penalty term must limit flipping.

### 11.3 Quantile-Asymmetry Boost
Allowed only when:
- right tail strongly favorable
- trigger confirmed
- execution confidence acceptable
- heat/reflexivity not extreme

Hard cap:
- about 1.20× baseline

### 11.4 Prohibited
- Kelly
- streak boosts
- fast performance-chasing leverage increases

### 11.5 Liquidation Exploitation Mode
Allowed only when:
- structure aligns
- trigger is real
- execution viable
- risk bounds hold

---

## 12. Execution Domain

### 12.1 Core Principles
- execution is adversarial
- execution quality must influence decisioning
- cascades require pessimistic assumptions
- partial fills are normal
- venue quality matters materially

### 12.2 Core Capabilities
- dynamic order style selection
- passive/aggressive/TWAP/staggered logic
- partial fill handling
- slippage-aware behavior
- venue selection guidance
- execution confidence scoring
- stress execution posture

### 12.3 Stress Execution Posture
- assume worse fills
- reduce aggressiveness
- allow trade abandonment if execution quality collapses

### 12.4 Execution Feedback Loop
Feed back:
- realized slippage
- fill quality
- fill latency
- venue degradation
into:
- trigger trust
- execution confidence
- candidate rank
- future style selection

---

## 13. Carry Sleeve Domain

### 13.1 Purpose
Provide isolated neutral carry capture when directional opportunity is low-conviction.

### 13.2 Rules
- fully isolated from directional attribution
- independent risk engine
- independent sizing
- independent PnL attribution
- used primarily when directional opportunity quality is low

---

## 14. Feedback, Memory, and Adaptation Domain

### 14.1 False Positive Memory
Track recently failed setup types and downweight similar patterns temporarily.

### 14.2 Opportunity Cost Tracking
Only for slow review/calibration; never for real-time chasing.

### 14.3 Regime Transition Guard
During transitions:
- higher confidence required
- smaller initial size
- more reliance on trigger confirmation

### 14.4 Drift Detection
Lightweight support for:
- feature drift
- output drift
- realized-vs-theoretical divergence

### 14.5 Slow Adaptation Only
No:
- per-trade updates
- fast self-rewriting
- hidden recursive behavior

---

## 15. Monitoring, Replay, and Governance Domain

### 15.1 Monitoring
Must cover:
- data freshness/reliability
- regime/heat/degradation
- trigger behavior
- auction behavior
- execution erosion
- drift / shadow divergence
- config versions
- alert acknowledgement

### 15.2 Replay / Simulation
Must support:
- deterministic historical replay
- stress scenarios
- fault injection
- live-vs-shadow comparison
- realized-vs-theoretical edge analysis

### 15.3 Governance
Must support:
- immutable config versions
- replay before promotion
- shadow before live where practical
- rollback targets
- experiment registry for research changes

---

## 16. Canonical Supporting Specs

The canonical supporting specs paired with this master spec are:

1. APEX Decision Service — Feature Schema & Data Contracts  
2. APEX — Trigger Math / Pseudocode Detail Spec  
3. APEX — Auction Scoring & Constraint Detail Spec  
4. APEX — Execution Logic Detail Spec  
5. APEX — State / Regime Logic Detail Spec  
6. APEX — Canonical Configuration Spec  
7. APEX — Replay and Simulation Interface Spec  
8. APEX — Monitoring & Alerting Spec  
9. APEX — Config Management & Release Gating Spec  
10. APEX — Research Experiment Registry Spec

---

## 17. Known Hard Risks

The system does not eliminate:
- imperfect timing
- execution friction
- data imperfection
- regime misclassification
- edge decay

Its goal is to remain profitable despite them.

---

## 18. Final Build Decision

This is the **canonical** full-system master spec.

It is complete enough to act as the single source of truth for:
- system identity
- major architectural decisions
- domain boundaries
- runtime flow
- what is in scope vs rejected

Implementation should now proceed using the supporting canonical specs, not older overlapping drafts.
