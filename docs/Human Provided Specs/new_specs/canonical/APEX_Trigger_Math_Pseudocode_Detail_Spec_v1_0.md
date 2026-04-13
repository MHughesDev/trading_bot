# APEX — Trigger Math / Pseudocode Detail Spec v1.0

**Document Type**: Implementation Detail Specification  
**Scope**: Trigger and timing logic  
**Version**: 1.0  
**Date**: April 2026  
**Status**: Build-ready draft  
**Parent Spec**: APEX Unified Full-System Master Spec v2.0

---

## 1. Purpose

This document formalizes the **trigger system** for APEX.

The trigger system exists to answer one narrow but critical question:

> Has the move actually started in a way that is worth trading now?

It is not responsible for:
- determining the whole thesis
- overriding hard safety controls
- replacing risk or execution logic
- forecasting price direction on its own

Its role is to provide **mechanical timing confirmation** after:
- state is acceptable
- structure/asymmetry is favorable
- no hard override blocks action

---

## 2. Design Requirements

The trigger system must be:

- compact
- deterministic
- replayable
- configurable
- explainable
- bounded in influence

It must avoid:
- vague discretionary pattern logic
- giant feature stacks
- dependence on one fragile signal
- implicit “always chase momentum” behavior

---

## 3. Trigger Architecture

The trigger is a **three-stage gate**.

```text
Setup → Pre-Trigger → Confirmed Trigger
```

A candidate trade may only proceed if all required stages pass.

---

## 4. Inputs

The trigger system consumes the following normalized inputs from upstream layers.

### 4.1 State Inputs
- regime probabilities
- regime confidence
- transition probability
- degradation level
- novelty score
- crypto heat score
- reflexivity score

### 4.2 Structure Inputs
- asymmetry score
- continuation probability
- fragility score
- liquidation opportunity score
- OI structure class
- confidence score
- directional bias

### 4.3 Market / Microstructure Inputs
- order book imbalance
- imbalance delta
- spread bps
- near-touch depth
- volume burst score
- local structure break score
- recent return / impulse score
- market freshness
- market reliability

### 4.4 Execution Inputs
- execution confidence estimate
- venue quality score
- expected slippage estimate

---

## 5. Stage 1 — Setup

### 5.1 Purpose
The setup stage decides whether the system should even care about a possible trade.

### 5.2 Inputs
- asymmetry score
- state alignment
- confidence
- novelty penalty
- heat penalty
- degradation level

### 5.3 Core Formula

Let:

- `A` = asymmetry score in `[0,1]`
- `S` = state alignment score in `[0,1]`
- `C` = composite confidence in `[0,1]`
- `H` = heat penalty in `[0,1]`
- `N` = novelty penalty in `[0,1]`

Then:

```text
setup_score_raw = wA*A + wS*S + wC*C - wH*H - wN*N
setup_score = clip(setup_score_raw, 0, 1)
```

### 5.4 Setup Validity Rules

Setup is valid if:

```text
setup_score >= setup_threshold
and degradation_level != no_trade
and novelty_hard_override == false
and execution_confidence_estimate >= setup_execution_floor
```

### 5.5 Setup Failure Behavior
If setup is invalid:
- no trigger evaluation continues
- candidate is not created
- reason code must be logged

### 5.6 Setup Reason Codes
Examples:
- `low_asymmetry`
- `weak_state_alignment`
- `insufficient_confidence`
- `excessive_heat`
- `novelty_block`
- `degradation_block`
- `poor_execution_context`

---

## 6. Stage 2 — Pre-Trigger

### 6.1 Purpose
The pre-trigger stage confirms that pressure is **building**, not merely statically present.

### 6.2 Inputs
- imbalance delta
- volume burst score
- microstructure tightening score
- structure pressure score
- liquidation proximity / fragility
- signal freshness

### 6.3 Core Formula

Let:

- `I` = imbalance shift score
- `V` = volume expansion score
- `T` = tightening / depth compression score
- `F` = freshness-adjusted structural pressure score

Then:

```text
pretrigger_score_raw = wI*I + wV*V + wT*T + wF*F
pretrigger_score = clip(pretrigger_score_raw, 0, 1)
```

### 6.4 Pre-Trigger Validity Rules

```text
pretrigger_valid = (
    pretrigger_score >= pretrigger_threshold
    and signal_freshness >= pretrigger_freshness_floor
)
```

### 6.5 Pre-Trigger Purpose Notes
This stage exists to reduce:
- entering too early
- static “good-looking” but inactive setups
- false positives from stale structural pressure

### 6.6 Pre-Trigger Reason Codes
Examples:
- `pressure_not_building`
- `volume_not_confirmed`
- `imbalance_not_shifting`
- `stale_pretrigger_inputs`

---

## 7. Stage 3 — Confirmed Trigger

### 7.1 Purpose
The confirmed trigger stage checks whether the move has actually started.

### 7.2 Allowed Trigger Families
The system should support a small, configurable set of trigger families:

- `imbalance_spike`
- `volume_burst`
- `structure_break`
- `composite_confirmed`

### 7.3 Atomic Trigger Components

Let:

- `B` = imbalance spike score
- `U` = volume burst score
- `K` = local structure break confirmation score

One possible composite:

```text
composite_score = c1*B + c2*U + c3*K
trigger_strength_raw = max(B, U, K, composite_score)
trigger_strength = clip(trigger_strength_raw, 0, 1)
```

### 7.4 Confirmed Trigger Rules

```text
trigger_valid = (
    trigger_strength >= trigger_threshold
    and execution_confidence_estimate >= trigger_execution_floor
)
```

### 7.5 Trigger Confidence

Let:

- `Cf` = structural confidence
- `Cm` = market / microstructure confidence
- `Ce` = execution confidence
- `Cd` = decay-adjusted freshness confidence

Then:

```text
trigger_confidence_raw = (Cf + Cm + Ce + Cd) / 4
trigger_confidence = clip(trigger_confidence_raw, 0, 1)
```

### 7.6 Trigger Confidence Bounds
Trigger confidence may:
- influence candidate score
- influence size fraction
- influence urgency

It may not:
- override hard risk caps
- create unbounded size escalation

---

## 8. Missed Move Acceptance

### 8.1 Purpose
The system must explicitly avoid chasing low-quality late entries.

### 8.2 Inputs
- current entry distance from preferred location
- remaining expected edge
- expected execution erosion
- current trigger freshness

### 8.3 Rule

Let:

- `E` = entry extension score
- `R` = remaining right-tail opportunity estimate
- `X` = expected execution erosion

If:

```text
E > entry_extension_limit
or R - X < minimum_remaining_edge
```

Then:

- `missed_move_flag = true`
- trigger is suppressed for new entry
- candidate may be dropped or converted into observation-only state

### 8.4 Reason Codes
- `move_already_extended`
- `insufficient_remaining_edge`
- `late_confirmation`
- `execution_too_degraded`

---

## 9. Trigger Output Contract

The trigger engine must emit:

- `setup_valid`
- `setup_score`
- `pretrigger_valid`
- `pretrigger_score`
- `trigger_valid`
- `trigger_type`
- `trigger_strength`
- `trigger_confidence`
- `missed_move_flag`
- `trigger_reason_codes`

---

## 10. Trigger State Machine

### 10.1 Logical States

```text
idle
→ setup_pending
→ setup_valid
→ pretrigger_pending
→ pretrigger_valid
→ trigger_pending
→ trigger_valid
→ expired
→ missed
→ suppressed
```

### 10.2 Transition Rules
- `idle → setup_pending`: new structural opportunity appears
- `setup_pending → setup_valid`: setup threshold met
- `setup_valid → pretrigger_valid`: building pressure confirmed
- `pretrigger_valid → trigger_valid`: confirmed trigger fires
- any active state → `suppressed`: hard override, degradation, or execution block
- any active state → `missed`: move too extended
- any active state → `expired`: conditions decayed before trigger completion

---

## 11. Trigger Lifetime and Expiry

Each trigger context must have:
- creation time
- max lifetime
- freshness decay

If:
- setup persists too long without pre-trigger
- pre-trigger persists too long without confirmed trigger
- underlying structure confidence collapses

Then:
- expire context
- emit expiry reason

---

## 12. Pseudocode

```text
function evaluate_trigger(decision_snapshot):
    setup_score = compute_setup_score(decision_snapshot)

    if setup_score < setup_threshold:
        return TriggerState(
            setup_valid=false,
            pretrigger_valid=false,
            trigger_valid=false,
            trigger_reason_codes=["low_setup_score"]
        )

    pretrigger_score = compute_pretrigger_score(decision_snapshot)

    if pretrigger_score < pretrigger_threshold:
        return TriggerState(
            setup_valid=true,
            setup_score=setup_score,
            pretrigger_valid=false,
            trigger_valid=false,
            trigger_reason_codes=["pretrigger_not_confirmed"]
        )

    trigger_strength = compute_confirmed_trigger_strength(decision_snapshot)

    if trigger_strength < trigger_threshold:
        return TriggerState(
            setup_valid=true,
            setup_score=setup_score,
            pretrigger_valid=true,
            pretrigger_score=pretrigger_score,
            trigger_valid=false,
            trigger_reason_codes=["confirmed_trigger_not_met"]
        )

    trigger_confidence = compute_trigger_confidence(decision_snapshot)

    if is_missed_move(decision_snapshot):
        return TriggerState(
            setup_valid=true,
            setup_score=setup_score,
            pretrigger_valid=true,
            pretrigger_score=pretrigger_score,
            trigger_valid=false,
            trigger_strength=trigger_strength,
            trigger_confidence=trigger_confidence,
            missed_move_flag=true,
            trigger_reason_codes=["missed_move"]
        )

    return TriggerState(
        setup_valid=true,
        setup_score=setup_score,
        pretrigger_valid=true,
        pretrigger_score=pretrigger_score,
        trigger_valid=true,
        trigger_type=select_trigger_type(decision_snapshot),
        trigger_strength=trigger_strength,
        trigger_confidence=trigger_confidence,
        missed_move_flag=false,
        trigger_reason_codes=["trigger_confirmed"]
    )
```

---

## 13. Configuration Domains

The trigger system must expose configuration for:

- setup threshold
- setup weights
- pre-trigger threshold
- pre-trigger weights
- trigger threshold
- trigger component weights
- execution floors
- freshness floors
- entry extension limit
- minimum remaining edge
- stage lifetime limits

---

## 14. Test Requirements

### Unit Tests
- setup score boundaries
- pre-trigger score boundaries
- trigger strength clipping
- missed move logic
- reason code completeness

### Scenario Tests
- early setup with no move
- proper trigger before cascade
- false breakout in chop
- late trigger after extension
- trigger suppression under no-trade degradation

### Replay Tests
- historical cascade events
- squeeze ignition events
- weekend fakeouts
- regime transition fake starts

---

## 15. Acceptance Criteria

The trigger system is acceptable when:

1. it is replayable
2. it is compact and explainable
3. it reduces early-entry bleed
4. it avoids excessive late chasing
5. it materially improves realized edge after execution assumptions
