# APEX — Monitoring & Alerting Specification v1.0

**Document Type**: Operational Specification  
**Scope**: Live monitoring, alerting, service health, decision-quality health, and operator visibility  
**Version**: 1.0  
**Date**: April 2026  
**Status**: Build-ready draft  
**Parent Spec**: APEX Unified Full-System Master Spec v2.0

---

## 1. Purpose

This document defines the monitoring and alerting requirements for APEX.

Its purpose is to ensure that:
- live system behavior is visible
- silent degradation is detectable
- risk and decision-quality issues are surfaced early
- alerting is informative without becoming noise
- operators can distinguish:
  - data problems
  - logic problems
  - execution problems
  - market-state problems

This specification is:
- product-agnostic
- tooling-agnostic
- storage-agnostic

It does not require any particular monitoring vendor, dashboard stack, or alert transport.

---

## 2. Monitoring Philosophy

1. Monitor **decision quality**, not just uptime
2. Monitor **realized-vs-theoretical edge**, not just PnL
3. Monitor **state transitions and suppressions**, not just trades
4. Alert on **meaningful changes**, not every anomaly
5. Support both:
   - real-time operational awareness
   - slower forensic diagnosis

---

## 3. Monitoring Domains

APEX monitoring is divided into these domains:

1. System health
2. Data quality and freshness
3. State / safety health
4. Trigger health
5. Decision / auction health
6. Risk health
7. Execution health
8. Carry sleeve health
9. Drift / calibration health
10. Replay / shadow divergence health
11. Operator / governance health

---

## 4. Required Metric Families

## 4.1 System Health Metrics
- service availability
- event processing lag
- decision cycle duration
- snapshot ingestion success rate
- decision output rate
- replay job status (if applicable)
- shadow run status

## 4.2 Data Quality Metrics
- freshness by feature family
- reliability by feature family
- missing field rate
- degraded field rate
- source disagreement rate
- stale signal rate
- liquidation confidence distribution
- OI confidence distribution
- funding confidence distribution
- options-context availability rate

## 4.3 State / Safety Metrics
- regime probability distribution over time
- regime confidence distribution
- transition probability distribution
- novelty score distribution
- crypto heat score distribution
- reflexivity score distribution
- degradation state occupancy
- degradation transition counts
- hard override counts by type
- weekend / low-liquidity mode occupancy

## 4.4 Trigger Metrics
- setup pass rate
- pre-trigger pass rate
- confirmed trigger pass rate
- trigger hit rate by type
- missed move rate
- trigger suppression rate
- trigger confidence distribution
- trigger latency from setup to confirm
- false positive rate by trigger type

## 4.5 Decision / Auction Metrics
- candidate count per cycle
- candidate eligibility rate
- auction selection count
- auction suppression count
- top-N saturation rate
- diversification penalty distribution
- overlap penalty distribution
- edge budget proxy distribution
- average auction score of selected vs suppressed
- decision confidence distribution

## 4.6 Risk Metrics
- gross exposure
- net exposure
- exposure by instrument
- exposure by thesis type
- concentration metrics
- correlation-cluster usage
- drawdown state
- CVaR estimates
- size multiplier distributions
- no-trade occupancy

## 4.7 Execution Metrics
- realized slippage bps
- fill ratio
- fill latency
- partial fill rate
- execution confidence distribution
- venue quality distribution
- stress execution mode activation rate
- realized-vs-expected fill divergence
- execution erosion by venue / regime / thesis

## 4.8 Carry Sleeve Metrics
- carry activation rate
- carry exposure
- carry sleeve isolated PnL
- carry risk utilization
- carry funding capture realized vs expected

## 4.9 Drift / Calibration Metrics
- feature drift alerts
- output drift alerts
- realized-vs-theoretical edge divergence
- false positive memory activation rate
- opportunity cost tracking metrics
- calibration epsilon utilization
- shadow/live divergence metrics

## 4.10 Governance / Audit Metrics
- config version currently active
- shadow config version
- number of config changes in lookback window
- number of hard overrides by human or governance policy
- replay coverage for recent config releases
- alert acknowledgement time

---

## 5. Required Dashboards / Views

The system should support at least the following logical dashboards.

## 5.1 Operations Overview
Shows:
- service health
- event lag
- degradation state
- current heat
- current novelty
- open risk posture
- major alerts

## 5.2 Data Quality View
Shows:
- freshness and reliability by feature family
- stale/degraded signal families
- venue/source disagreement
- signal confidence collapse patterns

## 5.3 State & Safety View
Shows:
- regime probabilities
- transition probability
- novelty
- heat
- degradation timeline
- exchange risk

## 5.4 Trigger & Decision View
Shows:
- setup / pre-trigger / trigger flows
- candidate counts
- auction scores
- suppression reasons
- missed move rate

## 5.5 Execution View
Shows:
- slippage
- fill ratio
- fill latency
- venue quality
- stress execution activation
- realized-vs-theoretical edge erosion

## 5.6 Portfolio / Risk View
Shows:
- exposure
- concentration
- correlation clusters
- drawdown state
- size multiplier changes
- no-trade state occupancy

## 5.7 Shadow / Validation View
Shows:
- live vs shadow divergence
- alternative config comparison
- trigger divergence
- auction divergence
- edge delta

---

## 6. Alerting Philosophy

Alerts should be:
- actionable
- categorized
- severity-ranked
- deduplicated
- rate-limited when appropriate

Alerts must avoid:
- flooding operators with repeated low-value notifications
- ambiguity about what domain failed
- alerting on expected noise that is already absorbed by the system

---

## 7. Alert Severity Levels

### Info
- useful context
- no action required immediately

### Warning
- degraded behavior
- increased operator awareness required

### Critical
- significant live risk
- protective or operational action may be required immediately

---

## 8. Required Alert Categories

## 8.1 Data Alerts
- stale core market snapshot
- stale structural snapshot
- collapse in liquidation data confidence
- OI data unavailable or highly inconsistent
- funding data desync
- options data unavailable (warning only if options enabled materially)
- structural source disagreement spike

## 8.2 State / Safety Alerts
- novelty critical
- degradation state changed to defensive or no-trade
- heat exceeds defensive threshold
- exchange risk high/critical
- transition probability spike
- persistent reduced/defensive state

## 8.3 Trigger Alerts
- trigger false positive spike
- trigger pass rate collapse
- missed move spike
- trigger latency abnormality
- trigger type breakdown anomaly

## 8.4 Decision / Auction Alerts
- candidate generation collapse
- auction selecting zero when opportunities expected
- diversification penalties saturating unusually often
- overlap penalty spike
- edge budget hard limit frequently exceeded

## 8.5 Risk Alerts
- concentration breach warning
- exposure cap warning
- CVaR threshold breach
- unusual no-trade occupancy
- rapid size shrink due to risk cascade

## 8.6 Execution Alerts
- slippage exceeds threshold
- fill ratio collapse
- venue quality collapse
- stress execution mode persistent
- realized-vs-theoretical erosion breach

## 8.7 Governance Alerts
- config change without replay evidence
- shadow/live divergence too high after release
- alert acknowledgement overdue
- audit logging gap

---

## 9. Alert Rules (Logical)

Each alert should define:
- `alert_id`
- `name`
- `domain`
- `severity`
- `condition`
- `evaluation_window`
- `suppression_window`
- `dedup_key`
- `required_context_fields`
- `recommended_operator_action`

### Example Logical Alert
```text
alert_id: execution_slippage_breach
domain: execution
severity: critical
condition:
  rolling_realized_slippage_bps > configured_limit
  for N consecutive decision windows
required_context:
  instrument_id
  venue_group
  degradation_level
  heat_score
recommended_action:
  inspect venue degradation and execution stress mode
```

---

## 10. Alert Routing

The monitoring system should support routing based on:
- domain
- severity
- environment
- instrument scope
- persistence

At minimum:
- info alerts → log/dashboard
- warning alerts → operator notification + dashboard
- critical alerts → immediate escalation path

---

## 11. Required Reason Codes in Alerts

Alerts must include:
- primary reason code
- major contributing metrics
- relevant config version
- affected instrument(s) or scope
- current degradation level
- current heat and novelty state when applicable

---

## 12. Derived Health Indicators

The system should compute higher-level health indicators such as:
- `decision_health_score`
- `data_integrity_score`
- `execution_health_score`
- `portfolio_heat_health_score`
- `shadow_alignment_score`

These are operator aids only; they do not replace underlying metrics.

---

## 13. Monitoring Windows

The system should support multiple windows:
- near-real-time (seconds to minutes)
- tactical (hourly)
- operational daily
- regime-cycle / release review

No single window is sufficient for all diagnoses.

---

## 14. Required Retention / Replay Support

Monitoring data should support:
- recent operational debugging
- replay-linked forensic analysis
- release/regression comparisons
- drift analysis over longer windows

Retention periods are implementation-defined, but the system must preserve enough context to compare live anomalies against replay and release history.

---

## 15. Acceptance Criteria

Monitoring and alerting are acceptable when:
1. major failure domains are visible before they become catastrophic
2. alert fatigue is controlled
3. state/trigger/auction/execution decisions are explainable from dashboards/logs
4. live-vs-shadow misalignment is detectable
5. release regressions can be diagnosed from retained metrics and logs
