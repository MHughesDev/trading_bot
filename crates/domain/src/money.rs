//! `Price` and `Size` newtypes over `Decimal` — the compiler refuses floats.
//!
//! # Money safety invariant
//!
//! Neither `Price` nor `Size` implements `From<f64>` or `TryFrom<f64>`.
//! Construction from a float literal is therefore a **compile error**, enforced
//! without runtime overhead.  Use `from_str` / `from_decimal` at the ingestion
//! boundary where a string or `Decimal` is already present.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// A non-negative decimal price (bid, ask, trade price, OHLC, etc.).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Price(pub Decimal);

/// A non-negative decimal quantity (trade size, bar volume, order size, etc.).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Size(pub Decimal);

impl Price {
    pub fn zero() -> Self {
        Self(Decimal::ZERO)
    }

    pub fn from_decimal(d: Decimal) -> Self {
        Self(d)
    }

    /// Round to `dp` decimal places using `MidpointAwayFromZero`.
    pub fn quantize(self, dp: u32) -> Self {
        Self(self.0.round_dp(dp))
    }

    pub fn inner(self) -> Decimal {
        self.0
    }
}

impl Size {
    pub fn zero() -> Self {
        Self(Decimal::ZERO)
    }

    pub fn from_decimal(d: Decimal) -> Self {
        Self(d)
    }

    pub fn quantize(self, dp: u32) -> Self {
        Self(self.0.round_dp(dp))
    }

    pub fn inner(self) -> Decimal {
        self.0
    }
}

impl FromStr for Price {
    type Err = rust_decimal::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Decimal::from_str(s).map(Self)
    }
}

impl FromStr for Size {
    type Err = rust_decimal::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Decimal::from_str(s).map(Self)
    }
}

impl fmt::Display for Price {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl fmt::Display for Size {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::ops::Add for Price {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        Self(self.0 + rhs.0)
    }
}

impl std::ops::Sub for Price {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self {
        Self(self.0 - rhs.0)
    }
}

impl std::ops::Mul for Size {
    type Output = Self;
    fn mul(self, rhs: Self) -> Self {
        Self(self.0 * rhs.0)
    }
}

impl std::ops::Mul<Price> for Size {
    type Output = Price;
    fn mul(self, rhs: Price) -> Price {
        Price(self.0 * rhs.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn price_from_str_round_trips() {
        let p: Price = "42.50".parse().unwrap();
        assert_eq!(p.to_string(), "42.50");
    }

    #[test]
    fn size_from_str_round_trips() {
        let s: Size = "0.001".parse().unwrap();
        assert_eq!(s.to_string(), "0.001");
    }

    #[test]
    fn quantize_truncates_correctly() {
        let p: Price = "42.123456".parse().unwrap();
        let q = p.quantize(2);
        assert_eq!(q.to_string(), "42.12");
    }

    #[test]
    fn serde_round_trip() {
        let p: Price = "100.00".parse().unwrap();
        let json = serde_json::to_string(&p).unwrap();
        let back: Price = serde_json::from_str(&json).unwrap();
        assert_eq!(p, back);
    }
}
