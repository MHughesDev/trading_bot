//! `definition_version` as an ordered value rather than a string (ADR-D-20).
//!
//! # Why this module exists
//!
//! The version gate used to be `matches!(v, "1.0" | "1.2")`. A whitelist is a
//! list somebody has to remember to extend, and twice nobody did: 1.1 shipped
//! (the `ModelForecast` node) and 1.3 shipped (the typed `exits` block), and
//! neither was added. The compiler in `strategy-lang` emits 1.3 documents, so
//! **every** strategy the authoring path produced was refused by the validator
//! the persist route runs first — on a code whose own `fix` in [`crate::teach`]
//! told the author to write 1.3.
//!
//! The replacement is a bounded range: `1.0 <= v <= `[`MAX_SUPPORTED`]. An
//! additive version is admitted the moment the parser gains it, and a version
//! from the future is still refused, which is the half of the whitelist that was
//! doing real work.
//!
//! # Why it is not a string comparison
//!
//! `"1.10" < "1.2"` lexically and `1.10 > 1.2` numerically, so a string compare
//! silently inverts the order at the tenth minor version. [`exits`] carried that
//! bug — `def.definition_version.as_str() >= "1.3"` — and it would have started
//! reading 1.10 documents as pre-exits, which means a 1.10 strategy with a stop
//! loss would have had the stop refused as "too low a version" while its entries
//! stayed legal. Ordering is on `(major, minor)` as numbers, here, once.
//!
//! [`exits`]: crate::exits

use std::cmp::Ordering;

use domain::strategy_def::{DEFINITION_VERSION, DEFINITION_VERSION_V1_4};

/// The lowest `definition_version` this validator accepts.
///
/// Tied to the domain constant rather than spelled again: the two cannot drift.
pub const MIN_SUPPORTED: &str = DEFINITION_VERSION;

/// The highest `definition_version` this validator accepts.
///
/// **This is the one line that moves when the schema gains a version.** It is
/// the domain's newest constant, so raising it is a single edit in `domain` plus
/// this alias — not a new arm in a `matches!` that nobody is looking at.
pub const MAX_SUPPORTED: &str = DEFINITION_VERSION_V1_4;

/// A parsed `MAJOR.MINOR` definition version.
///
/// Ordered by `(major, minor)` numerically. Deliberately not a general semver
/// type: the schema has never had a patch component, and accepting one here
/// would mean deciding what `1.2.1` means to a runtime that reads `1.2`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DefinitionVersion {
    pub major: u32,
    pub minor: u32,
}

impl DefinitionVersion {
    /// Parse `"1.3"`. Returns `None` for anything that is not exactly two
    /// non-negative integers separated by one `.`.
    ///
    /// Strict on purpose. `"1.3.0"`, `"v1.3"`, `"1.3 "` and `"01.3"` all parse
    /// "obviously" under a lenient reader and each would be a second spelling of
    /// a version whose canonical form is used as a hash input elsewhere.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        let (major, minor) = s.split_once('.')?;
        if major.is_empty() || minor.is_empty() {
            return None;
        }
        // Reject a leading zero (other than "0" itself) so `"01.3"` is not a
        // second spelling of `"1.3"`.
        for part in [major, minor] {
            if part.len() > 1 && part.starts_with('0') {
                return None;
            }
            if !part.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
        }
        Some(Self {
            major: major.parse().ok()?,
            minor: minor.parse().ok()?,
        })
    }

    /// The `(1, 0)` floor of the supported range.
    #[must_use]
    pub fn min_supported() -> Self {
        Self::parse(MIN_SUPPORTED).expect("MIN_SUPPORTED is a valid version")
    }

    /// The ceiling of the supported range.
    #[must_use]
    pub fn max_supported() -> Self {
        Self::parse(MAX_SUPPORTED).expect("MAX_SUPPORTED is a valid version")
    }

    /// Whether this version is inside `[MIN_SUPPORTED, MAX_SUPPORTED]`.
    #[must_use]
    pub fn is_supported(self) -> bool {
        self >= Self::min_supported() && self <= Self::max_supported()
    }
}

impl Ord for DefinitionVersion {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.major, self.minor).cmp(&(other.major, other.minor))
    }
}

impl PartialOrd for DefinitionVersion {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl std::fmt::Display for DefinitionVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}", self.major, self.minor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::strategy_def::{
        DEFINITION_VERSION_V1_1, DEFINITION_VERSION_V1_2, DEFINITION_VERSION_V1_3,
    };

    #[test]
    fn every_version_the_domain_declares_is_inside_the_range() {
        // The failure this replaces: a constant existed in `domain`, the parser
        // read it, and the validator refused it. Iterating the domain's own list
        // means shipping a version without admitting it is not possible without
        // also editing this array — which is the reviewable event a `matches!`
        // arm was not.
        for v in [
            DEFINITION_VERSION,
            DEFINITION_VERSION_V1_1,
            DEFINITION_VERSION_V1_2,
            DEFINITION_VERSION_V1_3,
            DEFINITION_VERSION_V1_4,
        ] {
            let parsed = DefinitionVersion::parse(v).unwrap_or_else(|| panic!("{v} must parse"));
            assert!(
                parsed.is_supported(),
                "{v} is declared by domain and refused here"
            );
        }
    }

    #[test]
    fn a_version_from_the_future_is_still_refused() {
        // The half of the whitelist that was doing real work.
        for v in ["1.5", "2.0", "9.9"] {
            let parsed = DefinitionVersion::parse(v).expect("parses");
            assert!(!parsed.is_supported(), "{v} must be refused");
        }
    }

    #[test]
    fn a_version_below_the_floor_is_refused() {
        assert!(!DefinitionVersion::parse("0.9")
            .expect("parses")
            .is_supported());
    }

    #[test]
    fn ten_sorts_above_two() {
        // The string comparison this module exists to remove: "1.10" < "1.2".
        assert!(
            "1.10" < "1.2",
            "the lexical order is the bug, not a strawman"
        );
        assert!(
            DefinitionVersion::parse("1.10").expect("parses")
                > DefinitionVersion::parse("1.2").expect("parses")
        );
        assert!(
            DefinitionVersion::parse("1.10").expect("parses")
                > DefinitionVersion::parse("1.3").expect("parses")
        );
    }

    #[test]
    fn a_malformed_version_is_none_rather_than_a_guess() {
        for bad in [
            "", "1", "1.", ".1", "1.3.0", "v1.3", "1.3 ", " 1.3", "1.x", "01.3", "-1.2",
        ] {
            assert!(
                DefinitionVersion::parse(bad).is_none(),
                "{bad:?} must not parse: an absent version is never a default one"
            );
        }
    }

    #[test]
    fn zero_is_a_legal_component_but_a_leading_zero_is_not() {
        assert_eq!(
            DefinitionVersion::parse("1.0"),
            Some(DefinitionVersion { major: 1, minor: 0 })
        );
        assert!(DefinitionVersion::parse("1.00").is_none());
    }
}
