//! The **Null Library** — a first-class, selectable, parameterized, logged null
//! object (spec §2.1).
//!
//! A permutation test's validity rests on the null being appropriate to the
//! question. The wrong null gives a confident, precise, *meaningless* p-value. So
//! the null is not a hidden default: it states its hypothesis explicitly via
//! `preserves`/`destroys`, it is **recommended but never defaulted**, and it
//! travels attached to every significance result (INV-3, [`significance`]).

pub mod generators;
pub mod significance;
pub mod store;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub use generators::{generator_for, Bar, NullData, NullGenerator};
pub use significance::SignificanceResult;
pub use store::{InMemoryNullStore, NullStore};

/// A content-addressed null identifier (`null:<hex>`).
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct NullId(String);

impl NullId {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for NullId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "NullId({})", self.0)
    }
}

impl std::fmt::Display for NullId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// The seven null kinds (spec §2.1 catalog).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NullKind {
    SignalReturnDecouple,
    BlockPermutation,
    StationaryBootstrap,
    BarPermutation,
    SyntheticGarch,
    RegimeBlock,
    RandomEntryMatched,
}

impl NullKind {
    /// The catalog's `(preserves, destroys)` hypothesis statement for this kind.
    #[must_use]
    pub fn hypothesis(self) -> (Vec<String>, Vec<String>) {
        let (p, d): (&[&str], &[&str]) = match self {
            NullKind::SignalReturnDecouple => (
                &["marginal_return_distribution", "signal_distribution"],
                &["signal_to_forward_return_pairing"],
            ),
            NullKind::BlockPermutation => (
                &["within_block_autocorrelation"],
                &["signal_timing_across_blocks"],
            ),
            NullKind::StationaryBootstrap => (
                &["autocorrelation_structure"],
                &["specific_historical_ordering"],
            ),
            NullKind::BarPermutation => (&["bar_level_ohlc_integrity"], &["inter_bar_sequence"]),
            NullKind::SyntheticGarch => (
                &["volatility_clustering", "fat_tails"],
                &["specific_realized_path"],
            ),
            NullKind::RegimeBlock => (&["within_regime_structure"], &["cross_regime_arrangement"]),
            NullKind::RandomEntryMatched => (
                &["trade_frequency", "holding_period", "exposure"],
                &["entry_timing_skill"],
            ),
        };
        (
            p.iter().map(ToString::to_string).collect(),
            d.iter().map(ToString::to_string).collect(),
        )
    }
}

/// Tunable null parameters (block lengths, resample counts, GARCH coefficients).
/// Absent fields fall back to per-kind defaults in the generators.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct NullParams {
    /// Fixed block length for `block_permutation`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block_length: Option<usize>,
    /// Mean block length for `stationary_bootstrap`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mean_block: Option<usize>,
    /// GARCH alpha (ARCH term).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub garch_alpha: Option<f64>,
    /// GARCH beta (GARCH term).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub garch_beta: Option<f64>,
}

/// A first-class null: kind + params + the explicit `preserves`/`destroys`
/// hypothesis (spec §2.1). The hypothesis fields are non-empty by construction.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Null {
    pub null_id: NullId,
    pub kind: NullKind,
    pub params: NullParams,
    pub preserves: Vec<String>,
    pub destroys: Vec<String>,
}

/// A reason a `Null` could not be constructed.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum NullError {
    #[error("a null must declare what it preserves and destroys (the hypothesis)")]
    EmptyHypothesis,
}

impl Null {
    /// Build a null for a kind, seeding `preserves`/`destroys` from the catalog.
    ///
    /// # Errors
    /// [`NullError::EmptyHypothesis`] can only arise if the catalog is corrupted
    /// (the built-in kinds always declare a hypothesis).
    pub fn new(kind: NullKind, params: NullParams) -> Result<Self, NullError> {
        let (preserves, destroys) = kind.hypothesis();
        if preserves.is_empty() || destroys.is_empty() {
            return Err(NullError::EmptyHypothesis);
        }
        let null_id = Self::compute_id(kind, &params);
        Ok(Self {
            null_id,
            kind,
            params,
            preserves,
            destroys,
        })
    }

    /// Content hash over kind + params (identical kind+params collide).
    fn compute_id(kind: NullKind, params: &NullParams) -> NullId {
        #[derive(Serialize)]
        struct H<'a> {
            kind: NullKind,
            params: &'a NullParams,
        }
        let value = serde_json::to_value(H { kind, params }).expect("null is serializable");
        let bytes = serde_json::to_vec(&value).expect("null is serializable");
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        NullId(format!("null:{}", hex::encode(hasher.finalize())))
    }

    /// Generate one null-world dataset from `data` and `seed`.
    #[must_use]
    pub fn generate(&self, data: &NullData, seed: u64) -> NullData {
        generator_for(self).generate(data, seed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_declares_a_nonempty_hypothesis() {
        for kind in [
            NullKind::SignalReturnDecouple,
            NullKind::BlockPermutation,
            NullKind::StationaryBootstrap,
            NullKind::BarPermutation,
            NullKind::SyntheticGarch,
            NullKind::RegimeBlock,
            NullKind::RandomEntryMatched,
        ] {
            let n = Null::new(kind, NullParams::default()).unwrap();
            assert!(!n.preserves.is_empty());
            assert!(!n.destroys.is_empty());
        }
    }

    #[test]
    fn identical_kind_and_params_collide_on_id() {
        let a = Null::new(
            NullKind::BlockPermutation,
            NullParams {
                block_length: Some(5),
                ..Default::default()
            },
        )
        .unwrap();
        let b = Null::new(
            NullKind::BlockPermutation,
            NullParams {
                block_length: Some(5),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(a.null_id, b.null_id);
        let c = Null::new(
            NullKind::BlockPermutation,
            NullParams {
                block_length: Some(7),
                ..Default::default()
            },
        )
        .unwrap();
        assert_ne!(a.null_id, c.null_id);
    }

    #[test]
    fn round_trips_serde() {
        let n = Null::new(
            NullKind::SyntheticGarch,
            NullParams {
                garch_alpha: Some(0.1),
                garch_beta: Some(0.85),
                ..Default::default()
            },
        )
        .unwrap();
        let back: Null = serde_json::from_str(&serde_json::to_string(&n).unwrap()).unwrap();
        assert_eq!(n, back);
    }
}
