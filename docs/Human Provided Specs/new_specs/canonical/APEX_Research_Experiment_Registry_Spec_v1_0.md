# APEX — Research Experiment Registry Specification v1.0

**Document Type**: Research Governance Specification  
**Scope**: Tracking, comparing, and governing research experiments that may influence live system behavior  
**Version**: 1.0  
**Date**: April 2026  
**Status**: Build-ready draft  
**Parent Spec**: APEX Unified Full-System Master Spec v2.0

---

## 1. Purpose

This document defines the experiment registry for APEX research.

It exists to ensure that:
- experiments are not lost
- results are comparable
- failed ideas are remembered
- live-impacting research is attributable
- signal and logic evolution stays disciplined

The registry is not a specific tool. It is a required logical capability.

---

## 2. Core Principles

1. Every experiment must have a stable id
2. Every experiment must declare its hypothesis
3. Every experiment must define success metrics before evaluation
4. Every experiment must record failure modes, not just wins
5. No experiment may be promoted without traceable evidence
6. Negative results are valuable and must be stored
7. Experiment results must connect to release candidates cleanly

---

## 3. Required Experiment Record Fields

Each experiment record must include:

- `experiment_id`
- `title`
- `owner`
- `created_at`
- `status`
- `domain`
- `hypothesis`
- `motivation`
- `change_type`
- `affected_components`
- `datasets_used`
- `config_versions_used`
- `logic_versions_used`
- `metrics_defined_before_run`
- `replay_runs`
- `scenario_tests`
- `shadow_results` (if any)
- `summary_result`
- `success_decision`
- `failure_modes_observed`
- `notes`
- `linked_release_candidate` (optional)

---

## 4. Experiment Status Lifecycle

Minimum statuses:
- `draft`
- `running`
- `completed`
- `rejected`
- `candidate_for_shadow`
- `candidate_for_release`
- `archived`

---

## 5. Experiment Domains

Experiments should be classified into at least:
- signal research
- trigger research
- auction research
- risk / sizing research
- execution research
- state / regime research
- degradation / safety research
- monitoring / alerting research
- replay / simulation methodology

---

## 6. Change Types

Each experiment should declare whether it is testing:
- a new feature family
- a new threshold set
- a new weighting scheme
- a new trigger rule
- a new penalty/constraint
- a new execution heuristic
- a new degradation rule
- a new monitoring rule

---

## 7. Hypothesis Requirement

Every experiment must state, before running:

1. what is being changed
2. why it should improve live PnL or robustness
3. what might break
4. how success will be measured
5. what evidence would cause rejection

---

## 8. Required Metrics

Experiment records must define metrics in advance from categories such as:

### Decision Quality
- trigger hit rate
- false positive rate
- missed move rate
- candidate conversion rate
- suppression quality

### PnL / Edge
- realized-vs-theoretical edge
- slippage erosion
- trade quality
- drawdown impact
- concentration impact

### Safety / Behavior
- no-trade occupancy
- defensive state frequency
- degradation transitions
- heat-score sensitivity
- live-vs-shadow divergence

### Operational
- data dependency increase
- compute/latency impact
- explainability cost
- replay complexity

---

## 9. Failure Mode Logging

Each experiment must explicitly record:
- known failure modes observed
- conditions where it underperformed
- whether it increased fragility
- whether it made the system too inactive
- whether it improved backtests but worsened execution realism

This is mandatory even for “successful” experiments.

---

## 10. Replay and Scenario Requirements

Any experiment proposing production-impacting change must be tested on:
- nominal historical runs
- stress scenario set
- at least one degraded-data scenario
- at least one execution-stress scenario

If relevant, it should also be shadowed.

---

## 11. Comparison Requirements

The registry must support comparing:
- baseline vs candidate
- multiple candidate versions
- config-only changes vs logic changes
- replay vs shadow outcomes

Comparison should include:
- delta in key metrics
- confidence intervals or stability indicators if available
- qualitative notes on why differences occurred

---

## 12. Rejection Discipline

Experiments should be explicitly rejected when they:
- improve theory but hurt realized edge assumptions
- increase fragility disproportionately
- add too much complexity for too little gain
- create unexplained divergence
- depend on brittle or unavailable data
- materially worsen replayability or explainability

Rejected experiments must remain searchable.

---

## 13. Promotion Criteria

An experiment may become a release candidate only if:
- the hypothesis was clear
- metrics were defined in advance
- replay evidence exists
- stress evidence exists
- failure modes were reviewed
- expected production impact is understood

Promotion should reference:
- config candidates
- logic candidates
- release notes drafts

---

## 14. Canonical Experiment Review Template

Every experiment review should answer:
1. What changed?
2. Why did we expect improvement?
3. What happened in replay?
4. What happened in stress scenarios?
5. What failure modes appeared?
6. Did it improve realized edge or only paper metrics?
7. Does it deserve shadow?
8. Does it deserve release candidacy?

---

## 15. Experiment Registry Queries That Must Be Possible

The registry should support queries such as:
- show all experiments that changed trigger logic
- show all rejected auction experiments
- show all experiments involving liquidation structure
- show all experiments that improved replay but hurt shadow
- show all experiments associated with a live release
- show all experiments whose main failure mode was over-throttling

---

## 16. Acceptance Criteria

The research experiment registry is acceptable when:
1. all production-relevant experiments are traceable
2. negative results are preserved
3. release candidates can be linked back to evidence
4. repeated rediscovery of failed ideas is reduced
5. experiment quality improves over time because hypotheses and failures are explicit
