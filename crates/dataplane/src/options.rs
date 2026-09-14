//! Options: store implied volatility with its model inputs, never greeks
//! (SPEC §1.6, INV-07, R-08). Greeks are computed here, in the read layer.

use chrono::{DateTime, NaiveDate, Utc};
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::identity::InstrumentKey;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Right {
    #[serde(rename = "C")]
    Call,
    #[serde(rename = "P")]
    Put,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OptionContract {
    pub instrument_id: InstrumentKey,
    pub underlying_id: InstrumentKey,
    pub expiry: NaiveDate,
    pub strike: Decimal,
    pub right: Right,
    pub exercise_style: char,
    pub multiplier: i32,
    pub occ_symbol: String,
    /// Post-corporate-action nonstandard deliverable.
    pub adjusted_flag: bool,
}

/// One long-format, sparse option minute bar. There is no greek field on purpose.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OptionBar {
    pub instrument_id: InstrumentKey,
    pub event_time: DateTime<Utc>,
    pub knowledge_time: DateTime<Utc>,
    pub ingest_time: DateTime<Utc>,
    pub close: Option<Decimal>,
    pub volume: Option<i64>,
    /// OI is published T+1; `knowledge_time` proves it.
    pub open_interest: Option<i64>,
    pub bid_close: Option<Decimal>,
    pub ask_close: Option<Decimal>,
    pub underlying_close: Option<Decimal>,
    pub iv_close: Option<f64>,
    pub iv_model: Option<String>,
    pub iv_rate: Option<f64>,
    pub iv_div: Option<f64>,
    pub moneyness: Option<f64>,
    pub dte: Option<i32>,
}

/// Column names that must never exist on an option table (AT-08).
pub const FORBIDDEN_GREEK_COLUMNS: &[&str] = &["delta", "gamma", "vega", "theta", "rho", "vanna", "volga", "charm"];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Greeks {
    pub price: f64,
    pub delta: f64,
    pub gamma: f64,
    pub vega: f64,
    pub theta: f64,
    pub rho: f64,
}

fn norm_cdf(x: f64) -> f64 {
    0.5 * erfc(-x / std::f64::consts::SQRT_2)
}

fn norm_pdf(x: f64) -> f64 {
    (-0.5 * x * x).exp() / (2.0 * std::f64::consts::PI).sqrt()
}

/// Complementary error function (W. J. Cody rational approximation, |err| < 1.2e-7).
fn erfc(x: f64) -> f64 {
    let z = x.abs();
    let t = 1.0 / (1.0 + 0.5 * z);
    let r = t * (-z * z - 1.265_512_23
        + t * (1.000_023_68
            + t * (0.374_091_96
                + t * (0.096_784_18
                    + t * (-0.186_288_06
                        + t * (0.278_868_07
                            + t * (-1.135_203_98 + t * (1.488_515_87 + t * (-0.822_152_23 + t * 0.170_872_77)))))))))
        .exp();
    if x >= 0.0 { r } else { 2.0 - r }
}

/// Black–Scholes–Merton greeks from stored inputs. Deterministic given
/// `(iv, S, K, T, r, q)` — which is exactly why storing greeks is redundant.
#[must_use]
pub fn bsm_greeks(right: Right, s: f64, k: f64, t_years: f64, r: f64, q: f64, iv: f64) -> Option<Greeks> {
    if !(s > 0.0 && k > 0.0 && t_years > 0.0 && iv > 0.0) {
        return None;
    }
    let sqrt_t = t_years.sqrt();
    let d1 = ((s / k).ln() + (r - q + 0.5 * iv * iv) * t_years) / (iv * sqrt_t);
    let d2 = d1 - iv * sqrt_t;
    let dq = (-q * t_years).exp();
    let dr = (-r * t_years).exp();
    let (price, delta, theta, rho) = match right {
        Right::Call => (
            s * dq * norm_cdf(d1) - k * dr * norm_cdf(d2),
            dq * norm_cdf(d1),
            -s * dq * norm_pdf(d1) * iv / (2.0 * sqrt_t) - r * k * dr * norm_cdf(d2) + q * s * dq * norm_cdf(d1),
            k * t_years * dr * norm_cdf(d2),
        ),
        Right::Put => (
            k * dr * norm_cdf(-d2) - s * dq * norm_cdf(-d1),
            dq * (norm_cdf(d1) - 1.0),
            -s * dq * norm_pdf(d1) * iv / (2.0 * sqrt_t) + r * k * dr * norm_cdf(-d2) - q * s * dq * norm_cdf(-d1),
            -k * t_years * dr * norm_cdf(-d2),
        ),
    };
    Some(Greeks {
        price,
        delta,
        gamma: dq * norm_pdf(d1) / (s * iv * sqrt_t),
        vega: s * dq * norm_pdf(d1) * sqrt_t,
        theta,
        rho,
    })
}

/// Read-layer greeks for a stored bar. `None` when the bar lacks IV inputs.
#[must_use]
pub fn greeks_for(contract: &OptionContract, bar: &OptionBar) -> Option<Greeks> {
    let s = bar.underlying_close?.to_f64()?;
    let k = contract.strike.to_f64()?;
    let expiry = contract.expiry.and_hms_opt(20, 0, 0)?.and_utc();
    let t = (expiry - bar.event_time).num_seconds() as f64 / (365.25 * 86_400.0);
    bsm_greeks(contract.right, s, k, t, bar.iv_rate?, bar.iv_div?, bar.iv_close?)
}

/// Default liquid-universe gate (OQ-03 answered: spec default).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct UniverseGate {
    pub max_abs_log_moneyness_dev: f64,
    pub min_dte: i32,
    pub max_dte: i32,
    pub require_two_sided_quote: bool,
}

impl Default for UniverseGate {
    fn default() -> Self {
        Self {
            max_abs_log_moneyness_dev: 0.30,
            min_dte: 1,
            max_dte: 400,
            require_two_sided_quote: true,
        }
    }
}

impl UniverseGate {
    /// In-gate bars go to the hot tier; the rest are retained cold, never deleted.
    #[must_use]
    pub fn admits(&self, bar: &OptionBar) -> bool {
        let Some(m) = bar.moneyness else { return false };
        let Some(dte) = bar.dte else { return false };
        let quoted = bar.bid_close.is_some_and(|b| b > Decimal::ZERO) && bar.ask_close.is_some_and(|a| a > Decimal::ZERO);
        (m - 1.0).abs() <= self.max_abs_log_moneyness_dev
            && (self.min_dte..=self.max_dte).contains(&dte)
            && (!self.require_two_sided_quote || quoted)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UniverseMembership {
    pub universe_id: i32,
    pub instrument_id: InstrumentKey,
    pub valid_from: DateTime<Utc>,
    pub valid_to: Option<DateTime<Utc>>,
    pub knowledge_time: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[allow(clippy::many_single_char_names)] // Black–Scholes notation: S, K, T, r, q, C, P.
    fn put_call_parity_holds() {
        let (s, k, t, r, q, iv) = (100.0, 95.0, 0.5, 0.03, 0.01, 0.25);
        let c = bsm_greeks(Right::Call, s, k, t, r, q, iv).unwrap();
        let p = bsm_greeks(Right::Put, s, k, t, r, q, iv).unwrap();
        let parity = s * (-q * t).exp() - k * (-r * t).exp();
        assert!((c.price - p.price - parity).abs() < 1e-5);
        assert!((c.delta - p.delta - (-q * t).exp()).abs() < 1e-6);
        assert!((c.gamma - p.gamma).abs() < 1e-9);
    }

    #[test]
    fn greeks_are_deterministic_from_stored_inputs() {
        let a = bsm_greeks(Right::Call, 50.0, 55.0, 0.2, 0.04, 0.0, 0.4).unwrap();
        let b = bsm_greeks(Right::Call, 50.0, 55.0, 0.2, 0.04, 0.0, 0.4).unwrap();
        assert_eq!(a, b);
        assert!(bsm_greeks(Right::Call, 50.0, 55.0, 0.0, 0.04, 0.0, 0.4).is_none());
    }

    #[test]
    fn option_bar_has_no_greek_fields() {
        let json = serde_json::to_value(OptionBar {
            instrument_id: InstrumentKey(1),
            event_time: Utc::now(),
            knowledge_time: Utc::now(),
            ingest_time: Utc::now(),
            close: None,
            volume: None,
            open_interest: None,
            bid_close: None,
            ask_close: None,
            underlying_close: None,
            iv_close: None,
            iv_model: None,
            iv_rate: None,
            iv_div: None,
            moneyness: None,
            dte: None,
        })
        .unwrap();
        for g in FORBIDDEN_GREEK_COLUMNS {
            assert!(json.get(*g).is_none(), "{g} must not be a stored field");
        }
    }

    #[test]
    fn universe_gate_defaults() {
        let mut bar: OptionBar = serde_json::from_value(serde_json::json!({
            "instrument_id": 1, "event_time": "2026-01-01T15:00:00Z", "knowledge_time": "2026-01-01T15:01:00Z",
            "ingest_time": "2026-01-01T15:01:00Z", "close": null, "volume": null, "open_interest": null,
            "bid_close": "1.0", "ask_close": "1.1", "underlying_close": null, "iv_close": null, "iv_model": null,
            "iv_rate": null, "iv_div": null, "moneyness": 1.1, "dte": 30
        }))
        .unwrap();
        let g = UniverseGate::default();
        assert!(g.admits(&bar));
        bar.moneyness = Some(1.5);
        assert!(!g.admits(&bar));
        bar.moneyness = Some(1.0);
        bar.ask_close = None;
        assert!(!g.admits(&bar));
    }
}
