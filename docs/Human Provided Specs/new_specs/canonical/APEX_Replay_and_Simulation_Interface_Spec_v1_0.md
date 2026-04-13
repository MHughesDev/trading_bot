# APEX — Replay and Simulation Interface Specification v1.0

**Document Type**: Interface / Validation Specification  
**Scope**: Replay, backtesting, event simulation, and live-vs-replay alignment  
**Version**: 1.0  
**Date**: April 2026  
**Status**: Build-ready draft  
**Parent Spec**: APEX Unified Full-System Master Spec v2.0

---

## 1. Purpose

This document defines the **replay and simulation interface** for APEX.

Its purpose is to ensure that:
- live decision logic can be replayed deterministically
- trigger, auction, execution, and risk behavior can be tested under historical and synthetic scenarios
- implementation and validation stay aligned
- the system can be stress-tested without redefining semantics between research and production

This spec does not mandate:
- a specific backtesting engine
- a specific simulator framework
- a specific file format
- a specific programming language

---

## 2. Replay / Simulation Goals

The replay and simulation framework must support:

1. deterministic replay of historical decision cycles
2. scenario-based stress testing
3. synthetic fault injection
4. live-vs-shadow behavior comparison
5. realized-vs-theoretical edge analysis
6. config/version-specific re-execution

---

## 3. Core Design Principles

1. Replay should use the same logical inputs as live
2. Replay should preserve config/version fidelity
3. Simulation should be able to degrade data and execution realism deliberately
4. Stress testing is as important as nominal replay
5. Replay must reveal where edge is lost, not just where trades looked good

---

## 4. Replay Scope

Replay / simulation must be able to emulate the following domains:

- market snapshots
- structural signal snapshots
- safety / regime snapshots
- trigger progression
- candidate generation
- auction selection
- risk and sizing logic
- execution assumptions
- memory/adaptation side effects (where enabled)
- monitoring outputs

---

## 5. Replay Input Contract

A replay run must be defined by:

### Required Fields
- `replay_run_id`
- `dataset_id`
- `config_version`
- `logic_version`
- `time_range_start`
- `time_range_end`
- `instrument_scope`
- `replay_mode`
- `execution_model_profile`
- `fault_injection_profile` (optional)
- `seed` (if any stochastic layer exists)

### Replay Modes
- `historical_nominal`
- `historical_stress`
- `synthetic_fault_injected`
- `shadow_comparison`
- `trigger_debug`
- `execution_debug`

---

## 6. Canonical Replay Event Types

Replay must support the following event families:

1. `market_snapshot_event`
2. `structural_signal_event`
3. `safety_snapshot_event`
4. `execution_feedback_event`
5. `config_change_event` (only if explicitly testing config drift)
6. `fault_injection_event`
7. `decision_output_event`

---

## 7. Historical Replay Requirements

### 7.1 Deterministic Replay
Given:
- the same input stream
- the same config version
- the same logic version

The replay engine must produce:
- the same decision records
- the same trigger states
- the same auction ordering
- the same output decisions
except where explicitly non-deterministic execution models are configured

### 7.2 Historical Scenarios to Support
At minimum:
- liquidation cascade crash
- funding squeeze melt-up
- low-vol chop
- false regime transition
- weekend liquidity cliff
- venue outage / fragmentation
- stale or delayed structural feeds
- partial fill stress

---

## 8. Simulation / Fault Injection Requirements

### 8.1 Purpose
Simulation must be able to test the system against conditions that may be underrepresented or too dangerous to rely on historically.

### 8.2 Supported Fault Injections
- stale data injection
- delayed data injection
- missing field injection
- corrupted confidence injection
- venue degradation injection
- spread widening injection
- liquidity collapse injection
- trigger delay injection
- partial fill failure injection
- liquidation data confidence collapse

### 8.3 Fault Injection Profiles
Each profile should define:
- target field family
- time window
- magnitude
- persistence
- whether the fault is deterministic or sampled

---

## 9. Execution Model Interface

Replay and simulation must use configurable execution models rather than assuming perfect fills.

### 9.1 Execution Model Profiles
At minimum:
- `optimistic`
- `baseline`
- `stress`
- `cascade_stress`

### 9.2 Execution Model Inputs
- spread
- depth
- urgency
- order style
- venue quality
- stress mode flags
- expected slippage profile

### 9.3 Execution Outputs
- simulated fill price
- simulated fill ratio
- simulated fill latency
- simulated execution confidence realized

---

## 10. Replay Output Artifacts

Each replay run must be able to emit:

- decision records
- trigger states over time
- candidate ranking traces
- trade intent stream
- suppression event stream
- safety override event stream
- execution erosion metrics
- realized-vs-theoretical edge metrics
- drift / degradation traces
- fault-injection traces

---

## 11. Live-vs-Shadow Alignment

### 11.1 Shadow Comparison Requirement
The replay/simulation framework should support comparing:
- live logic outputs
- shadow logic outputs
- alternative config outputs

For the same event stream.

### 11.2 Comparison Metrics
- trigger divergence rate
- candidate divergence rate
- auction ranking divergence
- trade intent divergence
- realized-vs-theoretical edge delta
- suppression delta

---

## 12. Time Semantics

Replay time must preserve:
- original event ordering
- timestamps
- event freshness logic
- confidence decay timing
- trigger lifetime logic
- degradation transitions

No replay engine may silently alter time semantics without explicit configuration.

---

## 13. Required Metrics

Replay and simulation must produce, at minimum:

### Decision Quality
- candidate-to-intent conversion rate
- no-trade rate
- suppression rate
- average decision confidence
- trigger hit rate
- false positive rate by trigger type

### PnL / Edge Quality
- realized vs theoretical edge
- slippage erosion
- partial fill erosion
- missed move frequency
- trade quality distribution

### Safety / State
- time in normal / reduced / defensive / no-trade
- novelty trigger frequency
- heat score distribution
- transition guard activation rate

### Concentration / Auction
- average candidate count
- average selected count
- diversification penalty distribution
- overlap penalty distribution

---

## 14. Required Replay APIs / Logical Functions

The replay framework should logically support:

- `load_config(config_version)`
- `load_event_stream(dataset_id, time_range, instruments)`
- `apply_fault_profile(profile)`
- `run_decision_cycle(event_stream, config)`
- `simulate_execution(trade_intents, execution_model_profile)`
- `emit_decision_records()`
- `emit_metrics()`
- `compare_runs(run_a, run_b)`

Exact signatures are implementation-defined.

---

## 15. Replay / Simulation Configuration Hooks

The replay interface must support config toggles for:
- execution model profile
- fault injection profile
- stale data injection
- partial fill model
- venue degradation model
- trigger debug verbosity
- shadow logic comparison
- deterministic seed

---

## 16. Scenario Library Requirements

A maintained scenario library should exist with labeled scenario classes:

1. `cascade_crash`
2. `reflexive_melt_up`
3. `low_vol_chop`
4. `false_transition`
5. `data_degradation`
6. `execution_breakdown`
7. `weekend_liquidity_cliff`
8. `exchange_fragmentation`

Each scenario should define:
- relevant instruments
- expected behavior patterns
- what “good” system behavior looks like
- key metrics to inspect

---

## 17. Acceptance Criteria

Replay and simulation are acceptable when:
1. live decisions can be reconstructed deterministically
2. execution realism can be varied explicitly
3. stale/degraded data scenarios can be injected
4. trigger and auction behavior can be inspected step-by-step
5. shadow and live logic can be compared on the same stream
6. scenario tests meaningfully surface edge erosion and failure modes

---

## 18. Recommended Next Companion Specs

After this document, the most valuable adjacent specs are:
1. monitoring & alerting spec
2. config management / release gating spec
3. research experiment registry spec

The first two are most useful for keeping live operations aligned with validation.
