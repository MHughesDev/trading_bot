//! The unit a return series is denominated in.
//!
//! A return series is a sequence of dimensionless ratios, which is exactly why
//! the unit has to travel beside it: nothing about `0.0031` says whether the
//! equity curve it came from was counted in US dollars, in Tether, or in the
//! base asset. Summing a USDT-denominated series into a USD one is a silent
//! error — the numbers add, the arithmetic is valid, and the answer is wrong by
//! whatever the peg moved, which is precisely when it matters.
//!
//! Set T's reconciliation compares a realised series against the backtest series
//! the ledger stored. "Element-wise comparable" is not a property two series can
//! have by accident; it has to be asserted, and the numeraire is the first thing
//! that has to match. [`Numeraire::assert_comparable`] is the assertion.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// What a return or an equity curve is counted in.
///
/// Deliberately a closed enum rather than a free string. A free string would let
/// `"USD"` and `"usd"` and `"US Dollar"` all coexist, and a comparison keyed on
/// equality would then pass for two series that were never in the same unit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Numeraire {
    /// US dollars.
    Usd,
    /// Tether. A dollar-referenced stablecoin, **not** a dollar.
    Usdt,
    /// USD Coin. Likewise not a dollar.
    Usdc,
    /// Euros.
    Eur,
    /// Bitcoin, for a strategy whose book is kept in BTC.
    Btc,
    /// Ether, likewise.
    Eth,
}

impl Numeraire {
    /// The canonical lowercase token stored in the database and on the wire.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Usd => "usd",
            Self::Usdt => "usdt",
            Self::Usdc => "usdc",
            Self::Eur => "eur",
            Self::Btc => "btc",
            Self::Eth => "eth",
        }
    }

    /// Every numeraire, in the order the database CHECK lists them.
    #[must_use]
    pub fn all() -> &'static [Numeraire] {
        &[
            Self::Usd,
            Self::Usdt,
            Self::Usdc,
            Self::Eur,
            Self::Btc,
            Self::Eth,
        ]
    }

    /// Refuse to treat two differently-denominated series as one series.
    ///
    /// There is deliberately no conversion here. Converting a *return* series
    /// between numeraires is not a scalar multiply — it needs the FX path over
    /// the same bars, and inventing one inside a comparison is how a divergence
    /// figure becomes a fiction. Converting happens upstream, where the equity
    /// curve still exists and the rate is a PIT lookup; by the time two return
    /// series meet, the only honest operation is to check they agree.
    ///
    /// # Errors
    /// [`NumeraireMismatch`] when the two differ.
    pub fn assert_comparable(self, other: Numeraire) -> Result<(), NumeraireMismatch> {
        if self == other {
            Ok(())
        } else {
            Err(NumeraireMismatch {
                left: self,
                right: other,
            })
        }
    }
}

impl fmt::Display for Numeraire {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Two series that are not in the same unit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
#[error("numeraire mismatch: {left} vs {right} — these series are not element-wise comparable")]
pub struct NumeraireMismatch {
    pub left: Numeraire,
    pub right: Numeraire,
}

/// An unrecognised numeraire token.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
#[error("unknown numeraire {0:?}")]
pub struct UnknownNumeraire(pub String);

impl FromStr for Numeraire {
    type Err = UnknownNumeraire;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "usd" => Ok(Self::Usd),
            "usdt" => Ok(Self::Usdt),
            "usdc" => Ok(Self::Usdc),
            "eur" => Ok(Self::Eur),
            "btc" => Ok(Self::Btc),
            "eth" => Ok(Self::Eth),
            _ => Err(UnknownNumeraire(s.to_owned())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_numeraire_is_comparable() {
        assert!(Numeraire::Usd.assert_comparable(Numeraire::Usd).is_ok());
    }

    /// The adversarial case this type exists for: USDT is not USD, and a
    /// comparison that quietly accepted it would produce a divergence number
    /// that is off by whatever the peg moved.
    #[test]
    fn usdt_is_not_usd() {
        let err = Numeraire::Usdt
            .assert_comparable(Numeraire::Usd)
            .expect_err("USDT must not compare equal to USD");
        assert_eq!(err.left, Numeraire::Usdt);
        assert_eq!(err.right, Numeraire::Usd);
    }

    #[test]
    fn no_conversion_api_exists() {
        // A compile-level statement of intent: `Numeraire` offers no `convert`,
        // no `rate`, and no `into_usd`. Converting a *return* series between
        // units needs a PIT FX path over the same bars; a scalar multiply here
        // would be a fiction that makes the divergence figure look defensible.
        // Asserted by the static check in
        // `crates/risk/tests/no_divergence_bypass.rs`.
        assert_eq!(Numeraire::all().len(), 6);
    }

    #[test]
    fn round_trips_through_its_token() {
        for n in Numeraire::all() {
            assert_eq!(Numeraire::from_str(n.as_str()).unwrap(), *n);
        }
    }

    #[test]
    fn unknown_token_is_rejected_rather_than_defaulted() {
        assert!(Numeraire::from_str("US Dollar").is_err());
        assert!(Numeraire::from_str("").is_err());
    }
}
