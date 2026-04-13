# APEX — Config Management & Release Gating Specification v1.0

**Document Type**: Governance / Release Specification  
**Scope**: Config lifecycle, promotion rules, release gates, and safe deployment controls  
**Version**: 1.0  
**Date**: April 2026  
**Status**: Build-ready draft  
**Parent Spec**: APEX Unified Full-System Master Spec v2.0

---

## 1. Purpose

This document defines how APEX configurations and logic changes are:
- created
- versioned
- reviewed
- replayed
- shadowed
- promoted
- rolled back

The goal is to prevent:
- silent behavior drift
- unreviewed production changes
- config chaos
- releases that break replayability
- unsafe live experiments

---

## 2. Core Principles

1. No material change without a version
2. No promotion without evidence
3. No release without replayability
4. Shadow before live where practical
5. Rollback must be simple and explicit
6. Config and logic must be separately attributable
7. Small controlled releases beat big clever releases

---

## 3. Release Objects

The system distinguishes at least these release objects:

1. **Config Release**
2. **Logic Release**
3. **Model Family Release**
4. **Feature Family Release**
5. **Combined Release** (only when unavoidable)

Each must have:
- version id
- owner
- rationale
- evidence package
- release notes
- rollback target

---

## 4. Environments

Minimum logical environments:
- `research`
- `simulation`
- `shadow`
- `live`

Rules:
- research changes do not imply simulation approval
- simulation approval does not imply shadow approval
- shadow approval does not imply live approval
- live must reference immutable versions

---

## 5. Configuration Lifecycle

### 5.1 Stages
1. Draft
2. Reviewed
3. Simulated
4. Shadowed
5. Approved for live
6. Active live
7. Retired / rolled back

### 5.2 Required Metadata
- config version
- author
- reviewer(s)
- change summary
- affected domains
- expected impact
- replay evidence links
- shadow evidence links
- approval timestamp

---

## 6. Logic / Code Release Lifecycle

For any material logic change:
1. code versioned
2. replay tested
3. scenario tested
4. shadow compared
5. approved
6. released
7. monitored during probation window

---

## 7. Release Gates

### 7.1 Mandatory Gates for Config Changes
A config change may be promoted only if:
- schema valid
- diff reviewed
- replay run completed on required scenario set
- no critical unexplained regressions
- owner approval present
- rollback target defined

### 7.2 Mandatory Gates for Logic Changes
A logic change may be promoted only if:
- unit tests pass
- scenario tests pass
- replay parity/regression suite passes
- shadow divergence reviewed
- release notes complete
- rollback plan documented

### 7.3 Mandatory Gates for Model Family / Forecast Changes
A model-related change may be promoted only if:
- holdout evidence exists
- replay comparison exists
- realized-vs-theoretical assumptions reviewed
- shadow behavior reviewed
- degradation behavior unchanged or explicitly approved

---

## 8. Required Evidence Package

Every material release must attach an evidence package containing:
- version identifiers
- domain(s) changed
- replay summary
- scenario stress summary
- live shadow comparison (if applicable)
- expected benefits
- known risks
- rollback target and instructions

---

## 9. Shadow Deployment Rules

### 9.1 Purpose
Shadow allows new logic/config to see live data without affecting execution.

### 9.2 Requirements
Shadow runs must:
- use the same live event stream
- log decision outputs
- record divergence from live
- be attributable to version ids

### 9.3 Divergence Metrics
- trigger divergence rate
- candidate divergence rate
- auction divergence rate
- suppression divergence rate
- execution-guidance divergence
- realized-vs-theoretical edge delta where measurable

### 9.4 Promotion from Shadow
Promotion requires:
- divergence understood
- no severe unexplained live-risk increase
- owner/reviewer sign-off

---

## 10. Rollback Requirements

Every live release must define:
- immediate rollback target
- rollback trigger conditions
- rollback owner
- rollback validation check

Rollback should be:
- operationally simple
- fast
- version-explicit
- auditable

---

## 11. Release Severity Classes

### Minor
Examples:
- dashboard-only labels
- non-material config default comments
- replay-only tooling changes

### Moderate
Examples:
- threshold tuning
- penalty weight changes
- alert threshold changes

### Major
Examples:
- trigger math changes
- auction scoring changes
- risk cap logic changes
- execution mode logic changes
- state/degradation logic changes

Major releases require the strongest evidence and gating.

---

## 12. Release Risk Checklist

Every major release should answer:
1. Does it change trade frequency?
2. Does it change concentration behavior?
3. Does it change degradation transitions?
4. Does it alter execution aggressiveness?
5. Does it increase reliance on any fragile signal?
6. Does it change replayability assumptions?
7. Does it weaken any hard override?

Any “yes” requires explicit review.

---

## 13. Probation Windows

New live releases should have a probation window with elevated monitoring for:
- trigger behavior
- suppression behavior
- execution erosion
- degradation changes
- live-vs-expected divergence

The probation length is implementation-defined but must be explicit.

---

## 14. Freeze Rules

The system should support release freezes during:
- severe live instability
- unresolved critical alert conditions
- major venue outage periods
- unresolved config attribution gaps

---

## 15. Required Audit Trail

For every live period, it must be possible to reconstruct:
- active config version
- active logic version
- active model family version references
- active feature enablement flags
- release history around the period
- whether shadow alternatives existed

---

## 16. Unsafe Change Types (Require Extra Review)

Examples:
- raising size caps
- lowering trigger thresholds materially
- weakening degradation thresholds
- reducing diversification penalties
- enabling new fragile feature families as core
- increasing calibration epsilon materially
- changing stress execution behavior

---

## 17. Acceptance Criteria

Config management and release gating are acceptable when:
1. every live behavior is attributable to explicit versions
2. no material change reaches live without evidence
3. replay, shadow, and rollback paths exist
4. unsafe changes are review-gated
5. operators can identify what changed and why
