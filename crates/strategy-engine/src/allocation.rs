//! Rebalancing a selection-bound deployment (Set U U-6.4, U-6.5; FEAT-007 §4;
//! ADR-0040 decision 4).
//!
//! A deployment can trade whatever a selection picks. Each time the selection
//! runs, [`rebalance`] compares what it picked with what the deployment holds and
//! says what changes: which instruments enter (start a trading instance), which
//! leave (and what happens to them — `close`, `let_strategy_exit`, `hold`), and
//! how the deployment's capital is divided.
//!
//! One pure function, called by the portfolio backtest and by the live
//! allocator alike, so a rebalance cannot mean one thing in a backtest and
//! another on paper.
//!
//! # Capital
//!
//! Weights are fractions of the deployment's capital and always sum to exactly
//! one over the instruments that hold an allocation: equal by default, or in
//! proportion to a selection attribute (a member whose attribute is missing, or
//! not positive, gets nothing and is not entered). An instrument that is
//! letting its strategy exit keeps the weight it had until its instance stops;
//! the rest is shared among the selection's members. A member offered nothing
//! — all the capital is still with exiting instances — is not entered; it waits
//! for a rebalance with capital to give. The last member by rank takes the
//! remainder of the division, so the sum is exact in decimal.

use std::collections::BTreeMap;

use domain::strategy_graph::deployment::{OnRemoval, Weighting};
use domain::strategy_graph::selection::Selection;
use domain::strategy_graph::Value;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// One instrument holding part of the deployment's capital.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sleeve {
    /// Fraction of the deployment's capital.
    pub weight: Decimal,
    /// Left the selection under `let_strategy_exit`: its instance runs until
    /// its own rules exit, then stops.
    pub exiting: bool,
}

/// What a deployment holds: instrument → sleeve.
pub type Allocation = BTreeMap<String, Sleeve>;

/// One change a rebalance makes, as the journal records it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "change", rename_all = "snake_case")]
pub enum AllocationChange {
    /// Start a trading instance.
    Entered { instrument: String, weight: Decimal },
    /// Stop its instance; `action` says what happens to the position.
    Left {
        instrument: String,
        action: OnRemoval,
    },
    /// Its instance keeps running until its own rules exit.
    Exiting { instrument: String, weight: Decimal },
    /// A member that stayed, with a new weight.
    Reweighted {
        instrument: String,
        from: Decimal,
        to: Decimal,
    },
}

impl AllocationChange {
    #[must_use]
    pub fn instrument(&self) -> &str {
        match self {
            Self::Entered { instrument, .. }
            | Self::Left { instrument, .. }
            | Self::Exiting { instrument, .. }
            | Self::Reweighted { instrument, .. } => instrument,
        }
    }
}

/// The rebalance's result: the new allocation and every change, in instrument
/// order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rebalance {
    pub next: Allocation,
    pub changes: Vec<AllocationChange>,
}

/// The numeric attributes `selection` gave `instrument`, as a member's
/// portfolio view carries them (U-6.6). One conversion, used by the portfolio
/// backtest and the live allocator alike, so both read the same number.
#[must_use]
pub fn member_attributes(selection: &Selection, instrument: &str) -> BTreeMap<String, Decimal> {
    selection
        .selected
        .iter()
        .find(|s| s.instrument == instrument)
        .map(|s| {
            s.attributes
                .iter()
                .filter_map(|(k, v)| match v {
                    // Through the shortest string that reads back as the same
                    // f64, so the attribute round-trips exactly.
                    Value::Num(x) => x
                        .get()
                        .to_string()
                        .parse::<Decimal>()
                        .ok()
                        .map(|d| (k.clone(), d)),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Rebalance `current` to the top `top_n` of `selection`.
#[must_use]
pub fn rebalance(
    current: &Allocation,
    selection: &Selection,
    top_n: u32,
    on_removal: OnRemoval,
    weighting: &Weighting,
) -> Rebalance {
    // The members that would take capital, best first, with their raw weight.
    let raw: Vec<(String, Decimal)> = selection
        .selected
        .iter()
        .take(top_n as usize)
        .filter_map(|s| {
            let w = match weighting {
                Weighting::Equal => Decimal::ONE,
                Weighting::Attribute { name } => match s.attributes.get(name) {
                    Some(Value::Num(x)) => Decimal::from_f64_retain(x.get())?,
                    _ => return None,
                },
            };
            (w > Decimal::ZERO).then(|| (s.instrument.clone(), w))
        })
        .collect();
    let chosen: BTreeMap<&str, Decimal> = raw.iter().map(|(m, w)| (m.as_str(), *w)).collect();

    let mut next = Allocation::new();
    let mut changes = Vec::new();

    // Leavers first: they decide how much capital is still spoken for.
    for (m, sleeve) in current {
        if chosen.contains_key(m.as_str()) {
            continue;
        }
        if sleeve.exiting {
            next.insert(m.clone(), sleeve.clone());
            continue;
        }
        match on_removal {
            OnRemoval::LetStrategyExit => {
                next.insert(
                    m.clone(),
                    Sleeve {
                        weight: sleeve.weight,
                        exiting: true,
                    },
                );
                changes.push(AllocationChange::Exiting {
                    instrument: m.clone(),
                    weight: sleeve.weight,
                });
            }
            action => changes.push(AllocationChange::Left {
                instrument: m.clone(),
                action,
            }),
        }
    }
    let spoken: Decimal = next.values().map(|s| s.weight).sum();
    let free = (Decimal::ONE - spoken).max(Decimal::ZERO);

    // Share what is free among the chosen, in rank order; the last takes the
    // remainder so the weights sum exactly.
    let total: Decimal = raw.iter().map(|(_, w)| *w).sum();
    let mut given = Decimal::ZERO;
    for (i, (m, w)) in raw.iter().enumerate() {
        let weight = if i + 1 == raw.len() {
            free - given
        } else {
            (free * *w / total).round_dp(12)
        };
        given += weight;
        if weight.is_zero() && !current.contains_key(m) {
            // Nothing free to give it — every unit is held by instances still
            // exiting. It waits for a later rebalance rather than starting an
            // instance with no capital.
            continue;
        }
        match current.get(m) {
            Some(old) if old.weight != weight || old.exiting => {
                changes.push(AllocationChange::Reweighted {
                    instrument: m.clone(),
                    from: old.weight,
                    to: weight,
                });
            }
            Some(_) => {}
            None => changes.push(AllocationChange::Entered {
                instrument: m.clone(),
                weight,
            }),
        }
        next.insert(
            m.clone(),
            Sleeve {
                weight,
                exiting: false,
            },
        );
    }
    changes.sort_by(|a, b| a.instrument().cmp(b.instrument()));
    Rebalance { next, changes }
}

/// An exiting instrument's instance has stopped (flat, no working orders): its
/// weight is released. Returns the change to journal, if it was exiting.
pub fn finish_exit(current: &mut Allocation, instrument: &str) -> Option<AllocationChange> {
    match current.get(instrument) {
        Some(s) if s.exiting => {
            current.remove(instrument);
            Some(AllocationChange::Left {
                instrument: instrument.to_string(),
                action: OnRemoval::LetStrategyExit,
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use domain::strategy_graph::selection::SelectedAsset;

    fn picked(xs: &[(&str, f64)]) -> Selection {
        Selection {
            revision: "r".into(),
            as_of: Utc::now(),
            universe_size: u32::try_from(xs.len()).unwrap(),
            selected: xs
                .iter()
                .enumerate()
                .map(|(i, (m, w))| SelectedAsset {
                    instrument: (*m).to_string(),
                    rank: u32::try_from(i + 1).unwrap(),
                    score: 0.0,
                    attributes: BTreeMap::from([("w".to_string(), Value::num(*w, "w"))]),
                })
                .collect(),
            excluded: vec![],
        }
    }

    fn sum(a: &Allocation) -> Decimal {
        a.values().map(|s| s.weight).sum()
    }

    #[test]
    fn three_equal_members_sum_to_exactly_one() {
        let r = rebalance(
            &Allocation::new(),
            &picked(&[("A", 1.0), ("B", 1.0), ("C", 1.0)]),
            3,
            OnRemoval::Close,
            &Weighting::Equal,
        );
        assert_eq!(sum(&r.next), Decimal::ONE);
        assert_eq!(r.changes.len(), 3);
        assert!(r
            .changes
            .iter()
            .all(|c| matches!(c, AllocationChange::Entered { .. })));
    }

    #[test]
    fn a_leaver_is_closed_an_entrant_started_and_the_stayer_reweighted() {
        let first = rebalance(
            &Allocation::new(),
            &picked(&[("A", 1.0), ("B", 1.0)]),
            2,
            OnRemoval::Close,
            &Weighting::Equal,
        );
        let second = rebalance(
            &first.next,
            &picked(&[("B", 1.0), ("C", 1.0), ("D", 1.0)]),
            3,
            OnRemoval::Close,
            &Weighting::Equal,
        );
        assert_eq!(
            second
                .changes
                .iter()
                .map(|c| (c.instrument(), std::mem::discriminant(c)))
                .collect::<Vec<_>>()
                .len(),
            4
        );
        assert!(matches!(
            &second.changes[0],
            AllocationChange::Left { instrument, action: OnRemoval::Close } if instrument == "A"
        ));
        assert!(
            matches!(&second.changes[1], AllocationChange::Reweighted { instrument, .. } if instrument == "B")
        );
        assert!(
            matches!(&second.changes[2], AllocationChange::Entered { instrument, .. } if instrument == "C")
        );
        assert_eq!(sum(&second.next), Decimal::ONE);
        assert!(!second.next.contains_key("A"));
    }

    #[test]
    fn top_n_caps_the_members_that_take_capital() {
        let r = rebalance(
            &Allocation::new(),
            &picked(&[("A", 1.0), ("B", 1.0), ("C", 1.0)]),
            2,
            OnRemoval::Close,
            &Weighting::Equal,
        );
        assert_eq!(r.next.keys().collect::<Vec<_>>(), ["A", "B"]);
    }

    #[test]
    fn attribute_weights_are_proportional_and_a_missing_one_gets_nothing() {
        let mut s = picked(&[("A", 3.0), ("B", 1.0), ("C", 2.0)]);
        s.selected[2].attributes.insert(
            "w".into(),
            Value::Missing(domain::strategy_graph::MissingReason::NoData { what: "w".into() }),
        );
        let r = rebalance(
            &Allocation::new(),
            &s,
            3,
            OnRemoval::Close,
            &Weighting::Attribute { name: "w".into() },
        );
        assert_eq!(r.next["A"].weight, Decimal::new(75, 2));
        assert_eq!(r.next["B"].weight, Decimal::new(25, 2));
        assert!(!r.next.contains_key("C"), "no weight, no instance");
        assert_eq!(sum(&r.next), Decimal::ONE);
    }

    #[test]
    fn a_strategy_exit_keeps_its_weight_until_the_instance_stops() {
        let first = rebalance(
            &Allocation::new(),
            &picked(&[("A", 1.0), ("B", 1.0)]),
            2,
            OnRemoval::LetStrategyExit,
            &Weighting::Equal,
        );
        let mut second = rebalance(
            &first.next,
            &picked(&[("B", 1.0), ("C", 1.0)]),
            2,
            OnRemoval::LetStrategyExit,
            &Weighting::Equal,
        )
        .next;
        assert!(second["A"].exiting);
        assert_eq!(
            second["A"].weight,
            Decimal::new(5, 1),
            "frozen at what it had"
        );
        assert_eq!(second["B"].weight + second["C"].weight, Decimal::new(5, 1));
        assert_eq!(sum(&second), Decimal::ONE);
        assert!(finish_exit(&mut second, "A").is_some());
        assert!(finish_exit(&mut second, "B").is_none(), "B is not exiting");
        let third = rebalance(
            &second,
            &picked(&[("B", 1.0), ("C", 1.0)]),
            2,
            OnRemoval::LetStrategyExit,
            &Weighting::Equal,
        );
        assert_eq!(
            sum(&third.next),
            Decimal::ONE,
            "the released weight is shared again"
        );
    }

    #[test]
    fn a_member_with_no_free_capital_waits_rather_than_starting_empty() {
        let first = rebalance(
            &Allocation::new(),
            &picked(&[("A", 1.0)]),
            1,
            OnRemoval::LetStrategyExit,
            &Weighting::Equal,
        );
        let second = rebalance(
            &first.next,
            &picked(&[("B", 1.0)]),
            1,
            OnRemoval::LetStrategyExit,
            &Weighting::Equal,
        );
        assert!(!second.next.contains_key("B"), "A still holds every unit");
        assert_eq!(
            second.changes.len(),
            1,
            "only A's exit: {:?}",
            second.changes
        );
        let mut after = second.next;
        finish_exit(&mut after, "A");
        let third = rebalance(
            &after,
            &picked(&[("B", 1.0)]),
            1,
            OnRemoval::LetStrategyExit,
            &Weighting::Equal,
        );
        assert_eq!(
            third.next["B"].weight,
            Decimal::ONE,
            "entered once the capital is free"
        );
    }

    #[test]
    fn hold_releases_the_capital_and_says_so() {
        let first = rebalance(
            &Allocation::new(),
            &picked(&[("A", 1.0)]),
            1,
            OnRemoval::Hold,
            &Weighting::Equal,
        );
        let second = rebalance(
            &first.next,
            &picked(&[("B", 1.0)]),
            1,
            OnRemoval::Hold,
            &Weighting::Equal,
        );
        assert!(matches!(
            &second.changes[0],
            AllocationChange::Left {
                action: OnRemoval::Hold,
                ..
            }
        ));
        assert_eq!(second.next["B"].weight, Decimal::ONE);
    }
}
