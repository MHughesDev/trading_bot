# APEX — Auction Scoring & Constraint Detail Spec v1.0

**Document Type**: Implementation Detail Specification  
**Scope**: Candidate trade scoring, selection, diversification, and throttling  
**Version**: 1.0  
**Date**: April 2026  
**Status**: Build-ready draft  
**Parent Spec**: APEX Unified Full-System Master Spec v2.0

---

## 1. Purpose

This document formalizes the **trade opportunity auction**.

The auction is responsible for:
- taking all valid candidate trades
- scoring them consistently
- enforcing trade quality and diversification
- selecting only the highest-quality opportunities
- preventing overtrading and hidden correlation concentration

It is the main mechanism that enforces:

> **Trade quality > trade quantity**

---

## 2. Auction Design Requirements

The auction must be:

- deterministic
- replayable
- configurable
- portfolio-aware
- bounded
- explainable

It must avoid:
- opaque ranking
- unlimited candidate throughput
- correlated pileups
- silent concentration in the same structural thesis

---

## 3. Inputs

Each candidate trade entering the auction must already have:

- asymmetry score
- state alignment score
- confidence score
- trigger score
- execution confidence score
- OI structure class
- liquidation opportunity score
- proposed size fraction
- hard reject reasons
- soft penalties

The auction also consumes:
- current portfolio exposure
- concentration state
- current heat score
- current degradation level
- edge budget proxy
- correlation estimates across candidates and current positions
- overlap/thesis clustering indicators

---

## 4. Candidate Eligibility

A candidate is eligible for auction only if all are true:

- no hard reject reason present
- trigger valid
- trigger confidence above minimum
- decision confidence above minimum
- execution confidence above minimum
- missed move flag is false
- degradation level allows new trade creation

If any fail:
- the candidate is excluded from ranking
- the exclusion reason must be logged

---

## 5. Canonical Auction Score

### 5.1 Positive Inputs

Let:

- `A` = asymmetry score
- `S` = state alignment score
- `C` = confidence score
- `T` = trigger score
- `E` = execution confidence score
- `O` = OI structure contribution
- `L` = liquidation opportunity contribution

### 5.2 Negative Inputs

Let:

- `D` = diversification penalty
- `M` = model overlap penalty
- `P` = false positive memory penalty
- `G` = degradation penalty
- `B` = edge budget penalty
- `R` = concentration/risk penalty

### 5.3 Base Formula

```text
auction_score_raw =
    wA*A +
    wS*S +
    wC*C +
    wT*T +
    wE*E +
    wO*O +
    wL*L
    -
    wD*D -
    wM*M -
    wP*P -
    wG*G -
    wB*B -
    wR*R
```

Then:

```text
auction_score = clip(auction_score_raw, auction_min_bound, auction_max_bound)
```

---

## 6. Component Definitions

## 6.1 Asymmetry Score (`A`)
Measures expected right-tail vs left-tail favorability.

Higher means:
- better payoff profile
- larger theoretical edge per unit risk

## 6.2 State Alignment (`S`)
Measures compatibility with:
- current regime
- heat state
- reflexivity conditions
- transition risk
- degradation level

## 6.3 Confidence Score (`C`)
Composite of:
- signal confidence
- freshness
- data reliability
- structural coherence

## 6.4 Trigger Score (`T`)
Derived from trigger strength and trigger confidence.

## 6.5 Execution Confidence (`E`)
Measures whether the trade can likely be monetized under current spread/depth/liquidity.

## 6.6 OI Structure Contribution (`O`)
Positive when OI structure supports the thesis.
Examples:
- healthy trend continuation for trend trade
- squeeze potential for squeeze trade
- fragile buildup for downside cascade thesis

## 6.7 Liquidation Opportunity Contribution (`L`)
Measures whether liquidation geometry adds favorable edge to the candidate.

---

## 7. Penalty Definitions

## 7.1 Diversification Penalty (`D`)
Penalizes candidates that increase clustering.

Components may include:
- candidate-to-candidate correlation
- candidate-to-book correlation
- same-thesis repetition
- same-liquidation-cluster dependence
- same-funding-imbalance dependence

Example:

```text
D = d1*corr_penalty + d2*thesis_overlap_penalty + d3*liq_overlap_penalty
```

## 7.2 Model Overlap Penalty (`M`)
Penalizes cases where apparent agreement is inflated by highly correlated model behavior.

## 7.3 False Positive Memory Penalty (`P`)
Temporary penalty when the current candidate resembles recently failed setups.

## 7.4 Degradation Penalty (`G`)
Penalty based on current degradation level:
- normal → minimal/zero
- reduced → moderate
- defensive → strong
- no-trade → ineligible

## 7.5 Edge Budget Penalty (`B`)
A throttle using practical proxies:
- heat
- overlap
- concentration
- confidence-adjusted notional already deployed

## 7.6 Concentration / Risk Penalty (`R`)
Penalizes:
- too much exposure to one instrument
- too much exposure to one structural theme
- excessive net directional concentration

---

## 8. Candidate Ranking Procedure

### 8.1 Initial Filtering
Remove:
- ineligible candidates
- duplicate candidates with identical thesis/instrument/side if policy requires deduplication

### 8.2 Score Computation
For each eligible candidate:
- compute penalties
- compute final auction score
- store explanation components

### 8.3 Sorting
Sort by:
1. `auction_score` descending
2. secondary tie-breakers if needed:
   - higher trigger confidence
   - higher execution confidence
   - lower concentration contribution

### 8.4 Selection
Walk the sorted list:
- add candidate if it does not violate portfolio/global constraints
- stop when top-N or top-notional budget reached

---

## 9. Top-N and Notional Limits

The auction may enforce:
- maximum number of selected candidates
- maximum selected notional
- per-symbol selected count
- per-thesis selected count

At least one explicit global limiter must exist.

---

## 10. Diversification Constraint

### 10.1 Purpose
Prevent the auction from selecting different expressions of the same risk.

### 10.2 Required Penalty Families
The auction must consider at least:
- rolling correlation
- same instrument family overlap
- same structural thesis overlap
- same liquidation dependence
- same crowding/funding imbalance dependence

### 10.3 Diversification Pseudocode

```text
function compute_diversification_penalty(candidate, selected, portfolio_state):
    corr_penalty = max_correlation(candidate, selected, portfolio_state)
    thesis_overlap_penalty = compute_thesis_overlap(candidate, selected)
    liquidation_overlap_penalty = compute_liq_overlap(candidate, selected)
    return d1*corr_penalty + d2*thesis_overlap_penalty + d3*liquidation_overlap_penalty
```

---

## 11. Edge Budgeting Constraint

### 11.1 Purpose
Prevent over-deployment into noisy or already crowded opportunity sets.

### 11.2 Edge Budget Proxy

Let:

- `H` = portfolio heat
- `Cq` = concentration proxy
- `Ov` = overlap proxy
- `N` = confidence-adjusted deployed notional

Then:

```text
edge_budget_proxy = a1*H + a2*Cq + a3*Ov + a4*N
```

### 11.3 Constraint Behavior
As edge budget rises:
- reduce size on new candidates
- lower rank through penalty
- optionally reduce allowed top-N count

It must not be treated as an exact edge oracle.

---

## 12. Candidate Size Proposal

### 12.1 Baseline Size

```text
base_size = f(asymmetry, confidence, trigger_confidence, execution_confidence)
```

### 12.2 Final Size

```text
size_fraction =
    base_size
    * degradation_multiplier
    * diversification_multiplier
    * edge_budget_multiplier
    * execution_multiplier
    * risk_cap_multiplier
```

### 12.3 Hard Caps
Final size must respect:
- per-trade size cap
- per-instrument cap
- per-thesis cap
- portfolio-level cap

---

## 13. Hard Rejection Rules

A candidate must be hard-rejected when:
- novelty hard override active
- no-trade degradation active
- execution confidence below hard floor
- missed move flag true
- invalid core data
- concentration breach non-recoverable
- prohibited thesis type in current state

---

## 14. Suppression vs Rejection

### Rejection
The candidate is invalid and should not be considered.

### Suppression
The candidate may be structurally valid but intentionally blocked by:
- budget
- diversification
- degradation
- insufficient remaining edge

Both must produce explicit reason codes.

---

## 15. Pseudocode

```text
function run_auction(candidates, portfolio_state):
    eligible = []

    for c in candidates:
        if not is_eligible(c, portfolio_state):
            log_rejection(c)
            continue

        c.diversification_penalty = compute_diversification_penalty(c, eligible, portfolio_state)
        c.edge_budget_penalty = compute_edge_budget_penalty(c, portfolio_state)
        c.concentration_penalty = compute_concentration_penalty(c, portfolio_state)
        c.auction_score = compute_auction_score(c)

        eligible.append(c)

    ranked = sort_descending(eligible, key=auction_score)

    selected = []
    for c in ranked:
        if violates_global_constraints(c, selected, portfolio_state):
            log_suppression(c)
            continue

        selected.append(c)

        if reached_top_n_limit(selected) or reached_notional_limit(selected):
            break

    return selected
```

---

## 16. Required Logging

For each candidate, log:
- eligibility result
- hard reject reasons
- all score components
- all penalties
- final auction score
- selected/suppressed/rejected status

---

## 17. Test Requirements

### Unit Tests
- score computation
- diversification penalty correctness
- edge budget penalty correctness
- ranking stability
- hard reject behavior

### Scenario Tests
- many highly correlated candidates
- many weak candidates vs one strong candidate
- high heat but strong asymmetry candidate
- liquidation cluster overlap scenario

### Replay Tests
- real periods with many simultaneous signals
- stress windows with concentration risk

---

## 18. Acceptance Criteria

The auction is acceptable when:
1. top-ranked trades are consistently higher-quality than median eligible trades
2. correlation clustering is materially reduced
3. suppression reasons are explainable
4. edge budget and degradation penalties do not collapse the system into inactivity
5. ranking remains stable under replay
