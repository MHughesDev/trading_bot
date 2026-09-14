//! Futures contracts, roll schedules with `decision_time`, and continuous series as
//! derived views (SPEC §1.5, INV-08, R-09).

use chrono::{DateTime, Duration, NaiveDate, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::identity::InstrumentKey;
use crate::quality::QualityFlags;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FutureContract {
    pub instrument_id: InstrumentKey,
    pub root: String,
    pub expiry: NaiveDate,
    pub first_notice: Option<NaiveDate>,
    pub last_trade: NaiveDate,
    pub contract_size: Decimal,
    pub tick_value: Decimal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RollRule {
    Calendar { days_before_expiry: i64 },
    OiCrossover,
    VolumeCrossover,
}

impl RollRule {
    /// Minimum lag between the observation a rule reads and the moment it may
    /// fire. OI and volume are published with a lag; firing on same-day OI is
    /// look-ahead (R-09).
    #[must_use]
    pub fn publication_lag(self) -> Duration {
        match self {
            Self::Calendar { .. } => Duration::zero(),
            Self::OiCrossover | Self::VolumeCrossover => Duration::days(1),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RollEvent {
    pub root: String,
    pub rule: RollRule,
    pub from_instrument_id: InstrumentKey,
    pub to_instrument_id: InstrumentKey,
    /// When the observation driving the rule describes (e.g. the OI's trading day).
    pub observation_time: DateTime<Utc>,
    /// When the rule could FIRE.
    pub decision_time: DateTime<Utc>,
    /// When the roll applies to the series.
    pub roll_event_time: DateTime<Utc>,
    pub knowledge_time: DateTime<Utc>,
    pub ratio_factor: Option<Decimal>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RollError {
    #[error("roll applies at {applies} before the rule could fire at {decision}")]
    AppliesBeforeDecision { applies: DateTime<Utc>, decision: DateTime<Utc> },
    #[error("rule {rule:?} fires at {decision} but its observation at {observed} is not published until {published} (look-ahead)")]
    UsesUnpublishedObservation {
        rule: RollRule,
        observed: DateTime<Utc>,
        decision: DateTime<Utc>,
        published: DateTime<Utc>,
    },
    #[error("decision_time {decision} is not known until knowledge_time {knowledge}")]
    DecisionBeforeKnowledge { decision: DateTime<Utc>, knowledge: DateTime<Utc> },
}

/// Validate a roll event (AT-07).
///
/// # Errors
/// Rejects any roll that applies before it could fire, fires on an observation
/// before its publication lag has elapsed, or is decided before it was knowable.
pub fn validate_roll(e: &RollEvent) -> Result<(), RollError> {
    if e.roll_event_time < e.decision_time {
        return Err(RollError::AppliesBeforeDecision {
            applies: e.roll_event_time,
            decision: e.decision_time,
        });
    }
    let published = e.observation_time + e.rule.publication_lag();
    if e.decision_time < published {
        return Err(RollError::UsesUnpublishedObservation {
            rule: e.rule,
            observed: e.observation_time,
            decision: e.decision_time,
            published,
        });
    }
    // A roll cannot apply to the series before the platform knew it had fired.
    if e.knowledge_time > e.roll_event_time {
        return Err(RollError::DecisionBeforeKnowledge {
            decision: e.decision_time,
            knowledge: e.knowledge_time,
        });
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContinuousMethod {
    /// Default. History immutable: a 2026 backtest reproduces bit-for-bit in 2031.
    ForwardAdjustedRatio,
    /// Rewrites all history at every roll and can go negative. Flags the dataset
    /// and every trial that uses it `non_reproducible`.
    BackAdjustedDifference,
}

impl ContinuousMethod {
    #[must_use]
    pub fn non_reproducible(self) -> bool {
        matches!(self, Self::BackAdjustedDifference)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ContractPrice {
    pub instrument_id: InstrumentKey,
    pub t: DateTime<Utc>,
    pub close: Decimal,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ContinuousPoint {
    pub t: DateTime<Utc>,
    pub value: Decimal,
    pub source_instrument: InstrumentKey,
    pub quality_flags: QualityFlags,
}

/// Build a continuous series VIEW from raw per-contract prices and roll events
/// known by `as_of`. Never stored as a fact.
#[must_use]
pub fn continuous_series(
    prices: &[ContractPrice],
    rolls: &[RollEvent],
    as_of: DateTime<Utc>,
    method: ContinuousMethod,
) -> Vec<ContinuousPoint> {
    let mut rolls: Vec<&RollEvent> = rolls
        .iter()
        .filter(|r| r.knowledge_time <= as_of && validate_roll(r).is_ok())
        .collect();
    rolls.sort_by_key(|r| r.roll_event_time);
    let Some(first) = rolls.first().map(|r| r.from_instrument_id).or_else(|| prices.first().map(|p| p.instrument_id)) else {
        return Vec::new();
    };

    let mut prices: Vec<&ContractPrice> = prices.iter().filter(|p| p.t <= as_of).collect();
    prices.sort_by_key(|p| p.t);

    // Which contract is active at each time.
    let active_at = |t: DateTime<Utc>| -> InstrumentKey {
        let mut cur = first;
        for r in &rolls {
            if r.roll_event_time <= t {
                cur = r.to_instrument_id;
            }
        }
        cur
    };
    let close_of = |inst: InstrumentKey, t: DateTime<Utc>| -> Option<Decimal> {
        prices
            .iter()
            .filter(|p| p.instrument_id == inst && p.t <= t)
            .max_by_key(|p| p.t)
            .map(|p| p.close)
    };

    let mut raw: Vec<ContinuousPoint> = Vec::new();
    for p in &prices {
        if p.instrument_id == active_at(p.t) {
            raw.push(ContinuousPoint {
                t: p.t,
                value: p.close,
                source_instrument: p.instrument_id,
                quality_flags: QualityFlags::NONE,
            });
        }
    }

    match method {
        ContinuousMethod::ForwardAdjustedRatio => {
            let mut factor = Decimal::ONE;
            let mut applied = 0usize;
            for pt in &mut raw {
                while applied < rolls.len() && rolls[applied].roll_event_time <= pt.t {
                    let r = rolls[applied];
                    let at = r.roll_event_time;
                    if let (Some(old), Some(new)) = (close_of(r.from_instrument_id, at), close_of(r.to_instrument_id, at)) {
                        if !new.is_zero() {
                            factor *= r.ratio_factor.unwrap_or(old / new);
                        }
                    }
                    applied += 1;
                }
                pt.value *= factor;
                if applied > 0 {
                    pt.quality_flags |= QualityFlags::SYNTHETIC_ROLL;
                }
            }
        }
        ContinuousMethod::BackAdjustedDifference => {
            for r in rolls.iter().rev() {
                let at = r.roll_event_time;
                if let (Some(old), Some(new)) = (close_of(r.from_instrument_id, at), close_of(r.to_instrument_id, at)) {
                    let gap = new - old;
                    for pt in raw.iter_mut().filter(|pt| pt.t < at) {
                        pt.value += gap;
                    }
                }
            }
            for pt in &mut raw {
                pt.quality_flags |= QualityFlags::SYNTHETIC_ROLL | QualityFlags::NON_REPRODUCIBLE;
            }
        }
    }
    raw
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use rust_decimal_macros::dec;

    fn d(day: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 3, day, 21, 0, 0).unwrap()
    }

    fn oi_roll(observed: DateTime<Utc>, decision: DateTime<Utc>) -> RollEvent {
        RollEvent {
            root: "ES".into(),
            rule: RollRule::OiCrossover,
            from_instrument_id: InstrumentKey(1),
            to_instrument_id: InstrumentKey(2),
            observation_time: observed,
            decision_time: decision,
            roll_event_time: decision,
            knowledge_time: decision,
            ratio_factor: None,
        }
    }

    /// AT-07: a same-day open-interest roll is look-ahead and must be rejected.
    #[test]
    fn same_day_oi_roll_is_rejected() {
        assert!(matches!(
            validate_roll(&oi_roll(d(10), d(10))),
            Err(RollError::UsesUnpublishedObservation { .. })
        ));
        assert_eq!(validate_roll(&oi_roll(d(10), d(11))), Ok(()));
    }

    #[test]
    fn roll_cannot_apply_before_it_fires() {
        let mut r = oi_roll(d(10), d(12));
        r.roll_event_time = d(11);
        assert!(matches!(validate_roll(&r), Err(RollError::AppliesBeforeDecision { .. })));
    }

    fn prices() -> Vec<ContractPrice> {
        let mut v = Vec::new();
        for day in 1..=20 {
            v.push(ContractPrice { instrument_id: InstrumentKey(1), t: d(day), close: dec!(100) + Decimal::from(day) });
            v.push(ContractPrice { instrument_id: InstrumentKey(2), t: d(day), close: dec!(110) + Decimal::from(day) });
        }
        v
    }

    #[test]
    fn forward_adjusted_history_is_immutable_across_later_rolls() {
        let p = prices();
        let roll = oi_roll(d(9), d(10));
        let before_roll = continuous_series(&p, &[], d(15), ContinuousMethod::ForwardAdjustedRatio);
        let with_roll = continuous_series(&p, &[roll], d(15), ContinuousMethod::ForwardAdjustedRatio);
        for (a, b) in before_roll.iter().zip(&with_roll).filter(|(a, _)| a.t < d(10)) {
            assert_eq!(a.value, b.value, "pre-roll history must not change at {}", a.t);
        }
        // Continuity at the roll: the first post-roll point equals the old contract's close.
        let at_roll = with_roll.iter().find(|pt| pt.t == d(10)).unwrap();
        assert_eq!(at_roll.value, dec!(110));
        assert_eq!(at_roll.source_instrument, InstrumentKey(2));
    }

    #[test]
    fn back_adjusted_rewrites_history_and_is_flagged() {
        let p = prices();
        let roll = oi_roll(d(9), d(10));
        let series = continuous_series(&p, &[roll], d(15), ContinuousMethod::BackAdjustedDifference);
        let first = &series[0];
        assert_eq!(first.value, dec!(111), "history shifted by the roll gap");
        assert!(series.iter().all(|pt| pt.quality_flags.contains(QualityFlags::NON_REPRODUCIBLE)));
        assert!(ContinuousMethod::BackAdjustedDifference.non_reproducible());
    }

    #[test]
    fn roll_not_yet_known_is_not_applied() {
        let p = prices();
        let mut roll = oi_roll(d(9), d(10));
        roll.knowledge_time = d(12);
        roll.roll_event_time = d(12);
        roll.decision_time = d(12);
        let s = continuous_series(&p, &[roll], d(11), ContinuousMethod::ForwardAdjustedRatio);
        assert!(s.iter().all(|pt| pt.source_instrument == InstrumentKey(1)));
    }
}
