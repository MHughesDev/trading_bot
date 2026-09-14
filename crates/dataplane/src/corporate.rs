//! Corporate actions and index membership, applied at READ time (SPEC §1.4, INV-03).
//!
//! Stored prices are unadjusted. An adjusted close is a function of the entire
//! future action stream, so adjustment composes the factors known by `as_of` when
//! the dataset is read — history stays immutable and dataset hashes stay stable.

use chrono::{DateTime, NaiveDate, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::identity::InstrumentKey;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionType {
    Split,
    Dividend,
    Spinoff,
    Merger,
    SymbolChange,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CorporateAction {
    pub instrument_id: InstrumentKey,
    pub action_type: ActionType,
    /// When the market learned of it.
    pub announcement_time: DateTime<Utc>,
    pub ex_date: NaiveDate,
    pub effective_time: DateTime<Utc>,
    /// Multiplicative price factor applied to prices strictly before `effective_time`.
    pub price_factor: Option<Decimal>,
    pub volume_factor: Option<Decimal>,
    pub cash_amount: Option<Decimal>,
    pub currency: Option<String>,
    pub knowledge_time: DateTime<Utc>,
    pub revision_seq: u32,
}

/// Read-time adjustment rules. Part of the dataset content hash.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdjustmentPolicy {
    Unadjusted,
    SplitsOnly,
    SplitsAndDividends,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AdjustFactors {
    pub price: Decimal,
    pub volume: Decimal,
}

/// The cumulative factor for a price observed at `event_time`, using only the
/// latest revision of each action known by `as_of`.
#[must_use]
pub fn factors_at(
    actions: &[CorporateAction],
    instrument: InstrumentKey,
    event_time: DateTime<Utc>,
    as_of: DateTime<Utc>,
    policy: AdjustmentPolicy,
) -> AdjustFactors {
    let mut price = Decimal::ONE;
    let mut volume = Decimal::ONE;
    if policy == AdjustmentPolicy::Unadjusted {
        return AdjustFactors { price, volume };
    }
    for a in current_revisions(actions, instrument, as_of) {
        if a.effective_time <= event_time {
            continue;
        }
        let applies = match a.action_type {
            ActionType::Split => true,
            ActionType::Dividend => policy == AdjustmentPolicy::SplitsAndDividends,
            _ => false,
        };
        if !applies {
            continue;
        }
        if let Some(f) = a.price_factor {
            price *= f;
        }
        if let Some(f) = a.volume_factor {
            volume *= f;
        }
    }
    AdjustFactors { price, volume }
}

fn current_revisions(
    actions: &[CorporateAction],
    instrument: InstrumentKey,
    as_of: DateTime<Utc>,
) -> Vec<&CorporateAction> {
    let mut latest: std::collections::HashMap<(ActionType, NaiveDate), &CorporateAction> =
        std::collections::HashMap::new();
    for a in actions
        .iter()
        .filter(|a| a.instrument_id == instrument && a.knowledge_time <= as_of)
    {
        latest
            .entry((a.action_type, a.ex_date))
            .and_modify(|cur| {
                if (a.knowledge_time, a.revision_seq) > (cur.knowledge_time, cur.revision_seq) {
                    *cur = a;
                }
            })
            .or_insert(a);
    }
    latest.into_values().collect()
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct IndexMembership {
    pub index_id: i32,
    pub instrument_id: InstrumentKey,
    /// Reconstitution is announced BEFORE it happens; that gap is where the alpha is.
    pub announcement_time: DateTime<Utc>,
    pub effective_from: DateTime<Utc>,
    pub effective_to: Option<DateTime<Utc>>,
    pub weight: Option<Decimal>,
    pub knowledge_time: DateTime<Utc>,
}

/// Was `instrument` in `index` at `at`, as known by `as_of`?
#[must_use]
pub fn member_at(
    rows: &[IndexMembership],
    index_id: i32,
    instrument: InstrumentKey,
    at: DateTime<Utc>,
    as_of: DateTime<Utc>,
) -> bool {
    rows.iter().any(|m| {
        m.index_id == index_id
            && m.instrument_id == instrument
            && m.knowledge_time <= as_of
            && m.effective_from <= at
            && m.effective_to.is_none_or(|e| at < e)
    })
}

/// Known-announced-but-not-yet-effective additions at `as_of`: tradeable information.
#[must_use]
pub fn pending_additions(rows: &[IndexMembership], index_id: i32, as_of: DateTime<Utc>) -> Vec<InstrumentKey> {
    rows.iter()
        .filter(|m| m.index_id == index_id && m.knowledge_time <= as_of && m.announcement_time <= as_of && as_of < m.effective_from)
        .map(|m| m.instrument_id)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone};
    use rust_decimal_macros::dec;

    fn t(y: i32, m: u32, d: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, 14, 30, 0).unwrap()
    }

    fn split(eff: DateTime<Utc>, known: DateTime<Utc>) -> CorporateAction {
        CorporateAction {
            instrument_id: InstrumentKey(7),
            action_type: ActionType::Split,
            announcement_time: known,
            ex_date: eff.date_naive(),
            effective_time: eff,
            price_factor: Some(dec!(0.25)),
            volume_factor: Some(dec!(4)),
            cash_amount: None,
            currency: None,
            knowledge_time: known,
            revision_seq: 0,
        }
    }

    /// AT-06 (read-time half): a retroactively ingested split changes adjusted values
    /// only for readers whose as_of is past its knowledge_time; the stored bar never
    /// changes.
    #[test]
    fn retroactive_split_is_invisible_to_earlier_as_of() {
        let stored_close = dec!(400);
        let bar_time = t(2024, 1, 10);
        let actions = vec![split(t(2024, 6, 1), t(2024, 5, 1))];
        let early = factors_at(&actions, InstrumentKey(7), bar_time, t(2024, 4, 1), AdjustmentPolicy::SplitsOnly);
        let late = factors_at(&actions, InstrumentKey(7), bar_time, t(2024, 7, 1), AdjustmentPolicy::SplitsOnly);
        assert_eq!(stored_close * early.price, dec!(400));
        assert_eq!(stored_close * late.price, dec!(100));
        assert_eq!(late.volume, dec!(4));
        // Bars after the effective time are not adjusted.
        let post = factors_at(&actions, InstrumentKey(7), t(2024, 6, 2), t(2024, 7, 1), AdjustmentPolicy::SplitsOnly);
        assert_eq!(post.price, Decimal::ONE);
    }

    #[test]
    fn unadjusted_policy_ignores_everything() {
        let actions = vec![split(t(2024, 6, 1), t(2024, 5, 1))];
        let f = factors_at(&actions, InstrumentKey(7), t(2024, 1, 1), t(2025, 1, 1), AdjustmentPolicy::Unadjusted);
        assert_eq!(f.price, Decimal::ONE);
    }

    #[test]
    fn a_revised_factor_supersedes_only_after_known() {
        let mut revised = split(t(2024, 6, 1), t(2024, 8, 1));
        revised.price_factor = Some(dec!(0.5));
        revised.revision_seq = 1;
        let actions = vec![split(t(2024, 6, 1), t(2024, 5, 1)), revised];
        let before = factors_at(&actions, InstrumentKey(7), t(2024, 1, 1), t(2024, 7, 1), AdjustmentPolicy::SplitsOnly);
        let after = factors_at(&actions, InstrumentKey(7), t(2024, 1, 1), t(2024, 9, 1), AdjustmentPolicy::SplitsOnly);
        assert_eq!(before.price, dec!(0.25));
        assert_eq!(after.price, dec!(0.5));
    }

    #[test]
    fn announced_reconstitution_is_visible_before_effective() {
        let m = IndexMembership {
            index_id: 500,
            instrument_id: InstrumentKey(9),
            announcement_time: t(2024, 3, 1),
            effective_from: t(2024, 3, 18),
            effective_to: None,
            weight: None,
            knowledge_time: t(2024, 3, 1) + Duration::minutes(5),
        };
        let rows = vec![m];
        assert_eq!(pending_additions(&rows, 500, t(2024, 3, 5)), vec![InstrumentKey(9)]);
        assert!(!member_at(&rows, 500, InstrumentKey(9), t(2024, 3, 5), t(2024, 3, 5)));
        assert!(member_at(&rows, 500, InstrumentKey(9), t(2024, 3, 20), t(2024, 3, 20)));
    }
}
