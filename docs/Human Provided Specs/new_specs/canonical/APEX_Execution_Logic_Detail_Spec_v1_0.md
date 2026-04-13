# APEX — Execution Logic Detail Spec v1.0

**Document Type**: Implementation Detail Specification  
**Scope**: Execution guidance, confidence, and feedback logic  
**Version**: 1.0  
**Date**: April 2026  
**Status**: Build-ready draft  
**Parent Spec**: APEX Unified Full-System Master Spec v2.0

---

## 1. Purpose

This document formalizes the **execution logic layer** for APEX.

The execution layer does not place orders directly in this specification.  
It provides the logic for:

- execution confidence estimation
- execution style recommendation
- stress posture behavior
- partial fill handling
- realized edge erosion tracking
- feedback into future decision quality

Its goal is simple:

> Preserve as much theoretical edge as possible under real crypto liquidity conditions.

---

## 2. Design Principles

1. Execution is adversarial
2. Execution confidence must influence decision quality
3. Stress conditions require pessimistic assumptions
4. Partial fills are normal
5. Good signals do not justify bad execution
6. Execution feedback must change future behavior slowly and traceably

---

## 3. Inputs

The execution layer consumes:

### 3.1 Market Microstructure Inputs
- spread bps
- depth near touch
- order book imbalance
- volume burst
- microprice if available
- venue quality / health
- latency indicators

### 3.2 Trade Context Inputs
- trade intent side
- intended urgency
- proposed size
- thesis type
- trigger strength
- trigger confidence
- max slippage tolerance

### 3.3 Historical Execution Inputs
- recent realized slippage
- fill ratio history
- fill latency history
- cancel/replace history
- venue degradation history

---

## 4. Execution Confidence

### 4.1 Purpose
Measure whether the trade can likely be monetized under current conditions.

### 4.2 Inputs
Let:
- `Qd` = depth quality score
- `Qs` = spread quality score
- `Qv` = venue quality score
- `Ql` = latency/response quality score
- `Qr` = recent realized slippage quality score

### 4.3 Formula

```text
execution_confidence_raw = (Qd + Qs + Qv + Ql + Qr) / 5
execution_confidence = clip(execution_confidence_raw, 0, 1)
```

### 4.4 Interpretation
- high → execution environment is acceptable
- medium → proceed carefully / stagger
- low → reduce size or suppress
- very low → suppress unless emergency reduction logic applies

---

## 5. Execution Style Selection

### 5.1 Supported Styles
- `passive`
- `aggressive`
- `staggered`
- `twap`

### 5.2 Style Selection Logic

Example:

```text
if execution_confidence >= high_confidence_threshold and spread_bps <= passive_spread_limit:
    style = passive
elif execution_confidence >= medium_confidence_threshold:
    style = staggered
elif urgency_high and expected_remaining_edge > emergency_entry_floor:
    style = aggressive
else:
    style = twap_or_suppress
```

### 5.3 Style Selection Requirements
Style selection must consider:
- urgency
- spread
- depth
- venue quality
- expected slippage
- trigger freshness
- remaining edge

---

## 6. Pre-Trade Worst-Case Heuristic

### 6.1 Purpose
Prevent theoretically good trades that become bad after realistic execution erosion.

### 6.2 Inputs
- expected edge
- expected slippage
- worst-case slippage multiplier
- adverse fill penalty
- spread risk penalty

### 6.3 Formula

```text
worst_case_edge = expected_edge - worst_case_slippage - adverse_fill_penalty - spread_risk_penalty
```

If:

```text
worst_case_edge < minimum_tradeable_edge
```

Then:
- suppress candidate
- or sharply reduce size

---

## 7. Stress Execution Mode

### 7.1 Activation Conditions
Stress execution mode activates when any are true:
- volatility exceeds configured threshold
- spread exceeds configured threshold
- venue quality drops below threshold
- heat score exceeds threshold
- liquidation activity / fragility exceeds threshold

### 7.2 Stress Mode Behavior
In stress mode:
- assume worse-than-normal fills
- reduce aggressiveness
- raise minimum remaining edge required
- lower allowed size
- allow valid ideas to be skipped if execution is unacceptable

### 7.3 Reason Codes
- `stress_spread_widening`
- `stress_liquidity_collapse`
- `stress_venue_degradation`
- `stress_heat_extreme`
- `stress_execution_disabled`

---

## 8. Partial Fill Handling

### 8.1 Principle
Partial fills are not exceptional. The system must reconcile them explicitly.

### 8.2 Required Behavior
If actual fill ratio is below threshold:
- recompute remaining intended exposure
- recompute expected remaining edge
- decide whether to:
  - continue
  - pause
  - cancel/abandon
  - convert to reduced exposure

### 8.3 Reconciliation Pseudocode

```text
function reconcile_partial_fill(intent, feedback):
    remaining_fraction = intent.size_fraction * (1 - feedback.fill_ratio)
    remaining_edge = recompute_remaining_edge(intent, feedback)

    if remaining_fraction <= min_remaining_fraction:
        return "done"

    if remaining_edge < minimum_tradeable_edge:
        return "abandon"

    if feedback.execution_confidence_realized < low_execution_floor:
        return "pause_or_reduce"

    return "continue_staggered"
```

---

## 9. Execution Feedback Loop

### 9.1 Purpose
Update future decisioning based on realized execution performance.

### 9.2 Inputs
- realized slippage
- fill ratio
- fill latency
- venue quality
- anomalies

### 9.3 Outputs
- updated execution trust score
- updated venue quality state
- updated trigger trust where severe mismatch occurred
- optional reduced urgency for similar contexts

### 9.4 Feedback Pseudocode

```text
function apply_execution_feedback(intent, feedback):
    erosion = compute_realized_edge_erosion(intent, feedback)
    update_execution_trust(intent.instrument_id, erosion)
    update_venue_quality(intent.instrument_id, feedback.venue_quality_score)

    if erosion > severe_erosion_threshold:
        reduce_execution_confidence_for_similar_contexts(intent)

    if feedback.partial_fill_flag and feedback.fill_ratio < partial_fill_problem_threshold:
        downweight_aggressive_styles(intent.instrument_id)
```

---

## 10. Execution Confidence and Decision Integration

Execution confidence must influence:
- candidate eligibility floor
- candidate score
- size fraction
- urgency
- order style
- suppression behavior

Execution confidence must not:
- silently fail open
- remain static after persistent slippage deterioration

---

## 11. Venue Awareness

### 11.1 Required Venue State
For each venue/context, maintain:
- quality score
- degradation status
- recent slippage
- recent fill quality
- response quality / latency
- anomaly state

### 11.2 Venue Rules
The system must:
- avoid assuming all venues are equivalent
- permit venue-specific suppression or downgrade
- avoid routing logic that depends on stale venue assumptions

---

## 12. Output Contract

Execution guidance output should include:
- `preferred_execution_style`
- `execution_confidence`
- `max_slippage_tolerance_bps`
- `stress_mode_flag`
- `venue_preference_order` if supported externally
- `execution_reason_codes`

---

## 13. Test Requirements

### Unit Tests
- execution confidence computation
- style selection
- worst-case edge suppression
- partial fill reconciliation
- stress mode transitions

### Scenario Tests
- cascading slippage event
- one venue degradation while others remain healthy
- wide spread during high urgency
- partial fill trap scenario

### Replay Tests
- historical dislocation windows
- liquidation cascades with degraded depth

---

## 14. Acceptance Criteria

Execution logic is acceptable when:
1. poor execution contexts materially reduce aggressiveness
2. realized edge erosion is measurable and fed back
3. partial fill handling avoids silent exposure mismatch
4. stress mode behaves pessimistically enough to prevent edge hallucination
5. style selection is deterministic and replayable
