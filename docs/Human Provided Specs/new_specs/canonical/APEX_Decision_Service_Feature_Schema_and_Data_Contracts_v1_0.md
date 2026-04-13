# APEX Decision Service — Feature Schema & Data Contracts v1.0

**Document Type**: Data Contract / Interface Specification  
**Scope**: Decision Service inputs, outputs, and internal snapshot schema  
**Version**: 1.0  
**Date**: April 2026  
**Status**: Final  
**Related Document**: APEX Decision Service — Formal Master Code Specification v1.0

---

## 1. Purpose

This document defines the **feature schema** and **data contracts** for the APEX Decision Service.

It specifies:

- the canonical input payloads the Decision Service consumes
- the normalized feature fields required by the service
- field meanings, types, nullability, freshness expectations, and confidence expectations
- the canonical output payloads the Decision Service emits
- validation rules and contract behavior under missing, stale, or degraded data

This document is:

- **product-agnostic**
- **language-agnostic**
- **transport-agnostic**
- **storage-agnostic**

It does **not** mandate:
- a message bus
- a database
- a serialization library
- an exchange SDK
- a programming language

---

## 2. Contract Design Principles

1. **Every field must be interpretable**
2. **Every time-sensitive field must carry freshness context**
3. **Confidence and reliability are first-class**
4. **Missing data must degrade behavior, not create undefined behavior**
5. **The Decision Service consumes normalized inputs, not raw exchange-specific payloads**
6. **Field semantics must remain stable across implementations**
7. **No upstream provider is assumed to be perfect**

---

## 3. Contract Families

The Decision Service uses the following contract families:

1. **Input Contracts**
   - Market Snapshot
   - Structural Signal Snapshot
   - Safety / Regime Snapshot
   - Execution Feedback Snapshot
   - Service Configuration Snapshot

2. **Output Contracts**
   - Trade Intent
   - Reduce Exposure Intent
   - No-Trade Decision
   - Suppression Event
   - Safety Override Event
   - Decision Record

3. **Internal Canonical Snapshot**
   - Decision Snapshot

---

## 4. Common Field Conventions

## 4.1 Primitive Types

Use these semantic types regardless of implementation language:

- `string`
- `integer`
- `float`
- `boolean`
- `datetime`
- `duration_ms`
- `map<string,float>`
- `list<string>`
- `enum`

## 4.2 Time Conventions

All timestamps must:
- be in UTC
- be explicit
- never rely on local exchange timezone assumptions

## 4.3 Confidence Conventions

All confidence-like fields must be normalized to:

```text
0.0 = unusable / no trust
1.0 = high confidence
```

## 4.4 Freshness Conventions

Freshness-like fields must be normalized to:

```text
0.0 = stale / unusable
1.0 = fresh
```

Freshness must reflect:
- signal age
- update regularity
- transport latency

## 4.5 Nullability Rules

Each field must be one of:
- **Required**
- **Optional**
- **Derived optional**
- **Conditional required**

Where:
- **Required** means the payload is invalid without it
- **Optional** means absence is acceptable
- **Derived optional** means the field may be omitted if upstream does not compute it
- **Conditional required** means required only when a related feature family is enabled

---

## 5. Canonical Input Contract: Market Snapshot

## 5.1 Purpose

Represents the normalized current market state for one decision cycle.

## 5.2 Schema

### Required Fields

| Field | Type | Description |
|---|---|---|
| `snapshot_id` | string | Unique identifier for this input snapshot |
| `timestamp` | datetime | UTC timestamp for the snapshot |
| `instrument_id` | string | Canonical instrument identifier |
| `venue_group` | string | Canonical venue grouping or source family |
| `last_price` | float | Latest tradable price |
| `mid_price` | float | Midpoint between best bid and ask |
| `best_bid` | float | Current best bid |
| `best_ask` | float | Current best ask |
| `spread_bps` | float | Current spread in basis points |
| `realized_vol_short` | float | Short-horizon realized volatility |
| `realized_vol_medium` | float | Medium-horizon realized volatility |
| `book_imbalance` | float | Normalized order book imbalance |
| `depth_near_touch` | float | Aggregate near-touch liquidity |
| `trade_volume_short` | float | Recent trade volume over short window |
| `volume_burst_score` | float | Recent abnormal volume score |
| `market_freshness` | float | Normalized freshness of this market snapshot |
| `market_reliability` | float | Reliability estimate for market snapshot |
| `session_mode` | enum | `regular`, `weekend`, `low_liquidity`, `stressed` |

### Optional Fields

| Field | Type | Description |
|---|---|---|
| `microprice` | float | Optional microprice |
| `depth_bid_1pct` | float | Optional depth within bid-side tolerance |
| `depth_ask_1pct` | float | Optional depth within ask-side tolerance |
| `trade_count_short` | integer | Optional recent trade count |
| `price_return_short` | float | Optional short-horizon return |
| `price_return_medium` | float | Optional medium-horizon return |
| `local_structure_break_score` | float | Optional breakout/breakdown measure |
| `exchange_health_score` | float | Optional normalized exchange health |
| `source_latency_ms` | duration_ms | Optional observed transport latency |

## 5.3 Validation Rules

- `best_ask >= best_bid`
- `spread_bps >= 0`
- `market_freshness ∈ [0,1]`
- `market_reliability ∈ [0,1]`
- `last_price > 0`
- reject as **invalid** only if core prices are missing or nonsensical
- otherwise downgrade confidence instead of rejecting

---

## 6. Canonical Input Contract: Structural Signal Snapshot

## 6.1 Purpose

Represents leverage-flow, positioning, liquidation, and structural market features.

## 6.2 Schema

### Required Fields

| Field | Type | Description |
|---|---|---|
| `snapshot_id` | string | Snapshot identifier |
| `timestamp` | datetime | UTC timestamp |
| `instrument_id` | string | Canonical instrument identifier |
| `funding_rate` | float | Current funding rate |
| `funding_rate_zscore` | float | Funding standardized relative to history |
| `funding_velocity` | float | Change rate of funding |
| `open_interest` | float | Current OI |
| `open_interest_delta_short` | float | Short-horizon OI change |
| `basis_bps` | float | Perp vs spot or synthetic basis |
| `cross_exchange_divergence` | float | Divergence across major venues |
| `liquidation_proximity_long` | float | Distance/proximity to long-side liquidation zone |
| `liquidation_proximity_short` | float | Distance/proximity to short-side liquidation zone |
| `liquidation_cluster_density_long` | float | Long-side liquidation density |
| `liquidation_cluster_density_short` | float | Short-side liquidation density |
| `liquidation_data_confidence` | float | Confidence in liquidation structure |
| `signal_freshness_structural` | float | Freshness of this structural bundle |
| `signal_reliability_structural` | float | Reliability of this structural bundle |

### Optional / Conditional Fields

| Field | Type | Description |
|---|---|---|
| `cascade_magnitude_estimate_long` | float | Estimated long-side cascade size |
| `cascade_magnitude_estimate_short` | float | Estimated short-side cascade size |
| `oi_concentration_score` | float | Concentration of leverage buildup |
| `oi_price_structure_class` | enum | Preclassified OI/price pattern |
| `perp_spot_divergence_score` | float | Divergence between perp and spot pressure |
| `funding_cross_exchange_dispersion` | float | Spread of funding across venues |
| `gex_score` | float | Options gamma exposure proxy |
| `iv_skew_score` | float | Options skew proxy |
| `options_freshness` | float | Freshness of options-derived fields |
| `options_reliability` | float | Reliability of options-derived fields |
| `stablecoin_flow_proxy` | float | Optional capital flow proxy |
| `exchange_leverage_skew_score` | float | Exchange-specific leverage imbalance |
| `signal_source_count` | integer | Count of contributing structural sources |

## 6.3 Validation Rules

- all confidence and freshness fields in `[0,1]`
- liquidation fields may be missing, but if present must be confidence-weighted
- OI may be stale; stale OI must lower confidence, not hard-fail
- options fields are **conditional** and must not block decisioning if unavailable

---

## 7. Canonical Input Contract: Safety / Regime Snapshot

## 7.1 Purpose

Provides high-level safety state, regime probabilities, and degradation controls.

## 7.2 Schema

### Required Fields

| Field | Type | Description |
|---|---|---|
| `snapshot_id` | string | Snapshot identifier |
| `timestamp` | datetime | UTC timestamp |
| `instrument_id` | string | Canonical instrument identifier or scope |
| `regime_probabilities` | map<string,float> | Probabilities across regime classes |
| `regime_confidence` | float | Confidence in regime estimate |
| `transition_probability` | float | Probability of regime transition |
| `novelty_score` | float | OOD / novelty score |
| `crypto_heat_score` | float | Composite heat metric |
| `reflexivity_score` | float | Crowding / reflexivity metric |
| `degradation_level` | enum | `normal`, `reduced`, `defensive`, `no_trade` |
| `weekend_mode` | boolean | Weekend / low-liquidity flag |
| `exchange_risk_level` | enum | `low`, `elevated`, `high`, `critical` |

### Optional Fields

| Field | Type | Description |
|---|---|---|
| `degradation_reason_codes` | list<string> | Reasons for current degradation |
| `volatility_circuit_breaker_active` | boolean | Optional active breaker state |
| `data_integrity_alert` | boolean | Optional input integrity warning |
| `transition_guard_active` | boolean | Optional special transition handling |

## 7.3 Validation Rules

- regime probabilities should sum approximately to 1.0
- degradation level must be one of the supported enums
- novelty score, heat score, reflexivity score must be normalized and documented

---

## 8. Canonical Input Contract: Execution Feedback Snapshot

## 8.1 Purpose

Provides realized execution quality relevant to future decisioning.

## 8.2 Schema

### Required Fields

| Field | Type | Description |
|---|---|---|
| `feedback_id` | string | Unique execution feedback event id |
| `timestamp` | datetime | Event time |
| `instrument_id` | string | Instrument id |
| `related_intent_id` | string | Trade intent being evaluated |
| `expected_fill_price` | float | Expected fill price at decision time |
| `realized_fill_price` | float | Actual realized fill price |
| `realized_slippage_bps` | float | Slippage in basis points |
| `fill_ratio` | float | Fraction of requested quantity filled |
| `fill_latency_ms` | duration_ms | Time to fill |
| `execution_confidence_realized` | float | Realized execution confidence outcome |
| `venue_quality_score` | float | Venue quality score at execution time |

### Optional Fields

| Field | Type | Description |
|---|---|---|
| `partial_fill_flag` | boolean | Indicates partial fill |
| `cancel_replace_count` | integer | Number of modifications |
| `order_style_used` | enum | `limit`, `market`, `twap`, `staggered` |
| `execution_stress_flag` | boolean | Whether stress posture applied |
| `execution_anomaly_codes` | list<string> | Optional anomaly indicators |

## 8.3 Validation Rules

- `fill_ratio ∈ [0,1]`
- `execution_confidence_realized ∈ [0,1]`
- missing feedback must not break decisioning; it only weakens adaptation quality

---

## 9. Canonical Internal Object: Decision Snapshot

## 9.1 Purpose

This is the canonical object the Decision Service should build before running trigger, auction, and risk logic.

## 9.2 Schema

| Field | Type | Description |
|---|---|---|
| `decision_snapshot_id` | string | Internal identifier |
| `timestamp` | datetime | Decision timestamp |
| `instrument_id` | string | Instrument |
| `market_snapshot` | object | Normalized market snapshot |
| `structural_snapshot` | object | Normalized structural snapshot |
| `safety_snapshot` | object | Safety/regime snapshot |
| `effective_signal_map` | map<string,float> | Fully confidence/freshness-adjusted signals |
| `forecast_quantiles` | map<string,float> | P5/P25/P50/P75/P95 |
| `volatility_forecast` | float | Forecast vol |
| `asymmetry_score` | float | Reward/risk imbalance |
| `continuation_probability` | float | Continuation likelihood |
| `fragility_score` | float | Forced-move potential |
| `directional_bias` | float | Weak directional bias |
| `model_agreement_score` | float | Agreement across models |
| `model_overlap_penalty` | float | Correlation/dependency penalty |
| `execution_confidence_estimate` | float | Current execution confidence |
| `false_positive_memory_penalty` | float | Downweight from recent failures |
| `edge_budget_score` | float | Current deployed-edge proxy |

---

## 10. Forecast / Structure Contract

The forecasting stage must produce the following canonical outputs, whether generated internally or provided by a model layer.

## 10.1 Required Fields

| Field | Type | Description |
|---|---|---|
| `p05` | float | Lower tail estimate |
| `p25` | float | Lower-mid estimate |
| `p50` | float | Median estimate |
| `p75` | float | Upper-mid estimate |
| `p95` | float | Upper tail estimate |
| `volatility_forecast` | float | Forecast volatility |
| `asymmetry_score` | float | Relative right-tail vs left-tail favorability |
| `continuation_probability` | float | Probability move continues |
| `fragility_score` | float | Probability of forced move / breakdown in structure |
| `directional_bias` | float | Weak directional pressure |
| `model_agreement_score` | float | Confidence from agreement |
| `model_correlation_penalty` | float | Penalty from correlated models |
| `calibration_weight` | float | Effective epsilon weight used |

## 10.2 OI Structure Class Contract

Allowed enum values:
- `healthy_trend`
- `fragile_buildup`
- `squeeze_potential`
- `deleveraging`
- `unknown`

---

## 11. Trigger Contract

## 11.1 Purpose

Represents the outcome of the multi-stage trigger pipeline.

## 11.2 Schema

| Field | Type | Description |
|---|---|---|
| `setup_valid` | boolean | Stage 1 valid |
| `setup_score` | float | Setup strength |
| `pretrigger_valid` | boolean | Stage 2 valid |
| `pretrigger_score` | float | Pre-trigger strength |
| `trigger_valid` | boolean | Stage 3 valid |
| `trigger_type` | enum | Trigger family |
| `trigger_strength` | float | Trigger intensity |
| `trigger_confidence` | float | Confidence in trigger |
| `missed_move_flag` | boolean | Move already too advanced |
| `trigger_reason_codes` | list<string> | Explainability codes |

## 11.3 Allowed Trigger Types

- `imbalance_spike`
- `volume_burst`
- `structure_break`
- `composite_confirmed`
- `none`

---

## 12. Candidate Trade Contract

## 12.1 Purpose

Represents one potential trade before auction selection.

## 12.2 Schema

| Field | Type | Description |
|---|---|---|
| `candidate_id` | string | Candidate identifier |
| `instrument_id` | string | Instrument |
| `side` | enum | `long`, `short`, `flat_reduction` |
| `thesis_type` | enum | `trend`, `squeeze`, `liquidation_exploitation`, `defensive_reduction`, `neutral` |
| `entry_style` | enum | `passive`, `aggressive`, `staggered` |
| `asymmetry_score` | float | Asymmetry |
| `state_alignment_score` | float | Alignment to state |
| `confidence_score` | float | Confidence |
| `trigger_score` | float | Trigger quality |
| `execution_confidence_score` | float | Execution quality estimate |
| `oi_structure_class` | enum | OI class |
| `liquidation_opportunity_score` | float | Opportunity from liquidation structure |
| `diversification_penalty` | float | Anti-clustering penalty |
| `auction_score` | float | Final ranking score |
| `proposed_size_fraction` | float | Proposed notional fraction |
| `hard_reject_reasons` | list<string> | Hard fail reasons |
| `soft_penalties` | list<string> | Penalty reasons |

---

## 13. Output Contract: Trade Intent

## 13.1 Schema

| Field | Type | Description |
|---|---|---|
| `intent_id` | string | Trade intent identifier |
| `timestamp` | datetime | Emission time |
| `instrument_id` | string | Instrument |
| `side` | enum | `long`, `short`, `reduce`, `flat` |
| `urgency` | enum | `low`, `medium`, `high` |
| `size_fraction` | float | Requested size fraction |
| `preferred_execution_style` | enum | `passive`, `aggressive`, `staggered`, `twap` |
| `decision_confidence` | float | Final decision confidence |
| `trigger_confidence` | float | Trigger confidence |
| `execution_confidence` | float | Execution confidence |
| `degradation_level` | enum | Current degradation level |
| `max_slippage_tolerance_bps` | float | Tolerance for execution |
| `reason_codes` | list<string> | Explainability reasons |

---

## 14. Output Contract: Suppression / No-Trade / Safety Events

## 14.1 Suppression Event

Represents a trade that was valid enough to consider but intentionally blocked.

Required fields:
- `event_id`
- `timestamp`
- `instrument_id`
- `suppression_type`
- `reason_codes`
- `blocked_candidate_id`
- `degradation_level`

## 14.2 No-Trade Decision

Represents a completed cycle with no trade.

Required fields:
- `event_id`
- `timestamp`
- `instrument_id`
- `no_trade_reason_codes`
- `state_summary`

## 14.3 Safety Override Event

Represents a hard override.

Required fields:
- `event_id`
- `timestamp`
- `override_type`
- `reason_codes`
- `affected_instruments`

---

## 15. Decision Record Contract

## 15.1 Purpose

Full replayable audit object for one decision cycle.

## 15.2 Required Contents

A Decision Record must include:
- all input snapshot ids
- effective signal map
- regime and degradation state
- forecast outputs
- trigger outputs
- candidate list
- auction rankings
- selected outputs
- rejection and suppression reasons
- config version identifiers
- execution confidence used
- model overlap and diversification values

---

## 16. Freshness and SLA Expectations (Logical, Not Product-Bound)

The service expects different logical freshness classes.

## 16.1 Fast Fields
Typical expectation:
- 1–5 second freshness

Examples:
- best bid/ask
- spread
- imbalance
- near-touch depth
- volume burst

## 16.2 Medium Fields
Typical expectation:
- 10–60 second freshness

Examples:
- OI delta
- basis
- cross-exchange divergence
- execution quality rolling state

## 16.3 Slow Fields
Typical expectation:
- 1–10 minute freshness

Examples:
- options-derived context
- stablecoin flow proxies
- slower structural indicators

If actual freshness exceeds acceptable windows:
- lower confidence
- increase decay
- potentially escalate degradation if critical

---

## 17. Missing Data Behavior

## 17.1 General Rule

Missing data should:
- lower confidence
- possibly remove a feature family
- not produce undefined decisions

## 17.2 Critical Missing Fields

If missing:
- price
- bid/ask
- spread
- novelty / safety snapshot
then the service should degrade aggressively or enter no-trade.

## 17.3 Non-Critical Missing Fields

If missing:
- options data
- stablecoin flow proxies
- auxiliary local extrema
then continue with lower confidence.

---

## 18. Versioning Rules

Every payload should support:
- `schema_version`
- `producer_id`
- `config_version`
- `generated_at`

Backwards-incompatible changes require:
- version bump
- migration rules
- replay compatibility plan

---

## 19. Validation Rules Summary

The Decision Service must validate:

- numeric ranges
- enum validity
- timestamp monotonicity where relevant
- confidence and freshness bounds
- non-negative spreads
- non-negative size fractions
- non-empty reason codes for suppressions and overrides

Validation failures should be classified as:
- `hard_invalid`
- `soft_degraded`
- `recoverable_missing`

---

## 20. Metrics Derived from Contracts

The following metrics should be derivable directly from the contracts:

- trade conversion rate from candidates to intents
- suppression rate by reason
- no-trade rate by state
- average decision confidence
- trigger hit rate
- false positive rate by trigger type
- realized vs theoretical edge
- average freshness by feature family
- average confidence by feature family
- execution erosion by venue / regime

---

## 21. Minimum Companion Specs Still Worth Writing

Yes — after this document, there are still a few specs worth doing.

## Highest Priority
1. **Trigger Math / Pseudocode Spec**
2. **Auction Scoring & Constraint Spec**
3. **Execution Logic Contract**
4. **State Snapshot / Regime Logic Spec**

## Useful Later
5. Replay / Simulation Interface Spec
6. Monitoring & Alerting Spec
7. Config Management / Versioning Spec

The first two are the most important because they define where the edge actually comes from.

---

## 22. Final Build Decision

This feature schema and data contract spec is sufficient to:

- start defining interfaces
- write validators
- implement decision snapshots
- build replayable decision flows
- connect upstream feature producers to the Decision Service

The best next spec is:

**Trigger Math / Pseudocode Spec**
