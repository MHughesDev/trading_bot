//! What a selection strategy returns (FEAT-007 §4).
//!
//! A ranked list of the instruments that met the criteria, each with its score
//! and the named attributes the strategy computed — and, as importantly, every
//! instrument that did **not** make it and why. A screener that reports only
//! winners cannot be audited: an instrument missing from the list might have
//! scored low, been filtered out, or had no data, and those are three different
//! facts about the market.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::value::{MissingReason, Value};

/// One evaluation of a selection strategy.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Selection {
    /// The selection strategy's revision.
    pub revision: String,
    /// The instant the selection describes (not when it was computed).
    pub as_of: DateTime<Utc>,
    /// How many instruments the universe held at `as_of`.
    pub universe_size: u32,
    /// Best first, by the strategy's declared order.
    pub selected: Vec<SelectedAsset>,
    /// Every universe member not selected, with the reason.
    pub excluded: Vec<Excluded>,
}

impl Selection {
    /// Every member is accounted for exactly once: selected or excluded.
    #[must_use]
    pub fn accounts_for_universe(&self) -> bool {
        let n = self.selected.len() + self.excluded.len();
        u32::try_from(n).is_ok_and(|n| n == self.universe_size)
    }

    /// Instruments in rank order.
    #[must_use]
    pub fn instruments(&self) -> Vec<&str> {
        self.selected
            .iter()
            .map(|s| s.instrument.as_str())
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SelectedAsset {
    pub instrument: String,
    /// 1 is best.
    pub rank: u32,
    pub score: f64,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub attributes: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Excluded {
    pub instrument: String,
    pub why: ExclusionReason,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ExclusionReason {
    /// The `where` filter was false.
    Filtered,
    /// The score or the filter could not be computed. Never ranked as zero.
    Missing { reason: MissingReason },
    /// Qualified, but ranked below `top_n`.
    BelowTopN { rank: u32, score: f64 },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_selection_accounts_for_every_member_and_round_trips() {
        let s = Selection {
            revision: "a".repeat(64),
            as_of: Utc::now(),
            universe_size: 4,
            selected: vec![SelectedAsset {
                instrument: "SOL-USD".into(),
                rank: 1,
                score: 2.1,
                attributes: BTreeMap::new(),
            }],
            excluded: vec![
                Excluded {
                    instrument: "ETH-USD".into(),
                    why: ExclusionReason::BelowTopN {
                        rank: 2,
                        score: 1.4,
                    },
                },
                Excluded {
                    instrument: "DOGE-USD".into(),
                    why: ExclusionReason::Filtered,
                },
                Excluded {
                    instrument: "NEW-USD".into(),
                    why: ExclusionReason::Missing {
                        reason: MissingReason::WarmingUp {
                            needed: 20,
                            have: 3,
                        },
                    },
                },
            ],
        };
        assert!(s.accounts_for_universe());
        assert_eq!(s.instruments(), vec!["SOL-USD"]);
        let back: Selection = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back, s);
    }
}
