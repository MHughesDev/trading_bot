//! Individual limit checks for the risk gate.

use rust_decimal::Decimal;

use domain::{
    instrument::HaltPolicy,
    money::{Price, Size},
    RiskRejection,
};

/// Global risk configuration applied to every order.
#[derive(Clone, Debug)]
pub struct GlobalRiskLimits {
    /// Maximum absolute position size per instrument (in base units).
    pub max_position: Decimal,
    /// Maximum orders in the last second (per account).
    pub max_orders_per_second: u32,
    /// Maximum orders in the last minute (per account).
    pub max_orders_per_minute: u32,
    /// Allowed price deviation from current market, in basis points.
    pub price_band_bps: u32,
    /// Maximum cumulative realized loss per day (in USD, positive = loss).
    pub max_daily_loss_usd: Decimal,
}

impl Default for GlobalRiskLimits {
    fn default() -> Self {
        Self {
            max_position: Decimal::from(100),
            max_orders_per_second: 5,
            max_orders_per_minute: 60,
            price_band_bps: 500,
            max_daily_loss_usd: Decimal::from(5000),
        }
    }
}

pub fn check_position(
    limits: &GlobalRiskLimits,
    instrument_id: &str,
    current: Decimal,
    requested: Size,
) -> Result<(), RiskRejection> {
    let new_abs = (current + requested.inner()).abs();
    if new_abs > limits.max_position {
        return Err(RiskRejection::PositionLimitExceeded {
            instrument_id: instrument_id.to_owned(),
            current: current.to_string(),
            requested: requested.inner().to_string(),
            limit: limits.max_position.to_string(),
        });
    }
    Ok(())
}

pub fn check_rate_second(
    limits: &GlobalRiskLimits,
    orders_last_second: u32,
) -> Result<(), RiskRejection> {
    if orders_last_second >= limits.max_orders_per_second {
        // Use the per-second variant so audit logs show the correct breach
        // reason (M-13: previously populated per-minute fields with per-second
        // values, making breach analysis unreliable).
        return Err(RiskRejection::RateLimitPerSecondExceeded {
            orders_last_second,
            limit_per_second: limits.max_orders_per_second,
        });
    }
    Ok(())
}

pub fn check_rate_minute(
    limits: &GlobalRiskLimits,
    orders_last_minute: u32,
) -> Result<(), RiskRejection> {
    if orders_last_minute >= limits.max_orders_per_minute {
        return Err(RiskRejection::RateLimitExceeded {
            orders_per_minute: orders_last_minute,
            limit: limits.max_orders_per_minute,
        });
    }
    Ok(())
}

/// Fat-finger price sanity check.  Passes immediately for market orders
/// (no limit price) or when no market price is available.
pub fn check_price_sanity(
    limits: &GlobalRiskLimits,
    instrument_id: &str,
    limit_price: Option<Price>,
    market_price: Option<Price>,
) -> Result<(), RiskRejection> {
    let Some(lp) = limit_price else { return Ok(()) };
    let Some(mp) = market_price else {
        return Ok(());
    };

    let lp_dec = lp.inner();
    let mp_dec = mp.inner();

    if mp_dec.is_zero() {
        return Ok(());
    }

    let band = mp_dec * Decimal::from(limits.price_band_bps) / Decimal::from(10_000_u32);
    let lower = mp_dec - band;
    let upper = mp_dec + band;

    if lp_dec < lower || lp_dec > upper {
        return Err(RiskRejection::PriceSanityFailed {
            instrument_id: instrument_id.to_owned(),
            limit_price: lp_dec.to_string(),
            market_price: mp_dec.to_string(),
            band_bps: limits.price_band_bps,
        });
    }
    Ok(())
}

/// Tick-size validity: limit price must be a whole multiple of `tick_size` (M-14).
///
/// Passes for market orders (no limit price) and when `tick_size` is zero.
pub fn check_price_tick(
    instrument_id: &str,
    limit_price: Option<domain::money::Price>,
    tick_size: Decimal,
) -> Result<(), RiskRejection> {
    let Some(lp) = limit_price else { return Ok(()) };
    if tick_size.is_zero() {
        return Ok(());
    }
    let remainder = lp.inner() % tick_size;
    if !remainder.is_zero() {
        return Err(RiskRejection::InvalidTickSize {
            instrument_id: instrument_id.to_owned(),
            price: lp.inner().to_string(),
            tick_size: tick_size.to_string(),
        });
    }
    Ok(())
}

/// Lot-size validity: order size must be a whole multiple of `lot_size`.
pub fn check_lot_size(
    instrument_id: &str,
    size: Size,
    lot_size: Decimal,
) -> Result<(), RiskRejection> {
    if lot_size.is_zero() {
        return Ok(());
    }
    let remainder = size.inner() % lot_size;
    if !remainder.is_zero() {
        return Err(RiskRejection::InvalidLotSize {
            instrument_id: instrument_id.to_owned(),
            size: size.inner().to_string(),
            lot_size: lot_size.to_string(),
        });
    }
    Ok(())
}

pub fn check_daily_loss(
    limits: &GlobalRiskLimits,
    daily_loss_usd: Decimal,
) -> Result<(), RiskRejection> {
    if daily_loss_usd >= limits.max_daily_loss_usd {
        return Err(RiskRejection::DailyLossLimitExceeded {
            daily_loss_usd: daily_loss_usd.to_string(),
            limit_usd: limits.max_daily_loss_usd.to_string(),
        });
    }
    Ok(())
}

pub fn check_instrument_active(instrument_id: &str, active: bool) -> Result<(), RiskRejection> {
    if !active {
        return Err(RiskRejection::InstrumentInactive {
            instrument_id: instrument_id.to_owned(),
        });
    }
    Ok(())
}

/// Reject an order if the instrument is outside its trading session.
/// Always passes for 24/7 instruments (when `is_in_session == true`).
pub fn check_trading_session(
    instrument_id: &str,
    is_in_session: bool,
) -> Result<(), RiskRejection> {
    if !is_in_session {
        return Err(RiskRejection::OutsideTradingHours {
            instrument_id: instrument_id.to_owned(),
        });
    }
    Ok(())
}

/// Reject an order if the instrument is currently halted and its halt policy
/// is `Haltable`.  Non-haltable instruments (e.g. permissionless crypto) pass
/// unconditionally regardless of the `is_halted` flag.
pub fn check_halt(
    instrument_id: &str,
    halt_policy: &HaltPolicy,
    is_halted: bool,
) -> Result<(), RiskRejection> {
    if is_halted && *halt_policy == HaltPolicy::Haltable {
        return Err(RiskRejection::InstrumentHalted {
            instrument_id: instrument_id.to_owned(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn limits() -> GlobalRiskLimits {
        GlobalRiskLimits::default()
    }

    // ── Session / halt checks ────────────────────────────────────────────────

    #[test]
    fn in_session_passes() {
        assert!(check_trading_session("AAPL", true).is_ok());
    }

    #[test]
    fn outside_session_rejected() {
        let err = check_trading_session("AAPL", false);
        assert!(matches!(
            err,
            Err(RiskRejection::OutsideTradingHours { .. })
        ));
    }

    #[test]
    fn haltable_and_halted_rejected() {
        let err = check_halt("AAPL", &HaltPolicy::Haltable, true);
        assert!(matches!(err, Err(RiskRejection::InstrumentHalted { .. })));
    }

    #[test]
    fn non_haltable_ignores_halt_flag() {
        assert!(check_halt("BTC-USDT", &HaltPolicy::NonHaltable, true).is_ok());
    }

    #[test]
    fn haltable_but_not_halted_passes() {
        assert!(check_halt("AAPL", &HaltPolicy::Haltable, false).is_ok());
    }

    #[test]
    fn position_within_limit_passes() {
        assert!(check_position(
            &limits(),
            "BTC-USD",
            Decimal::ZERO,
            Size::from_str("1").unwrap(),
        )
        .is_ok());
    }

    #[test]
    fn position_exceeds_limit_rejects() {
        let err = check_position(
            &limits(),
            "BTC-USD",
            Decimal::ZERO,
            Size::from_str("999").unwrap(),
        );
        assert!(matches!(
            err,
            Err(RiskRejection::PositionLimitExceeded { .. })
        ));
    }

    #[test]
    fn position_exactly_at_limit_passes() {
        let lim = GlobalRiskLimits {
            max_position: Decimal::from(10),
            ..Default::default()
        };
        assert!(check_position(
            &lim,
            "BTC-USD",
            Decimal::ZERO,
            Size::from_str("10").unwrap(),
        )
        .is_ok());
    }

    #[test]
    fn position_one_over_limit_rejects() {
        let lim = GlobalRiskLimits {
            max_position: Decimal::from(10),
            ..Default::default()
        };
        let err = check_position(
            &lim,
            "BTC-USD",
            Decimal::ZERO,
            Size::from_str("10.001").unwrap(),
        );
        assert!(matches!(
            err,
            Err(RiskRejection::PositionLimitExceeded { .. })
        ));
    }

    #[test]
    fn fat_finger_price_rejected() {
        let lim = GlobalRiskLimits {
            price_band_bps: 100, // 1% band
            ..Default::default()
        };
        let market = Some(Price::from_str("100").unwrap());
        let far_limit = Some(Price::from_str("200").unwrap()); // 100% above
        let err = check_price_sanity(&lim, "BTC-USD", far_limit, market);
        assert!(matches!(err, Err(RiskRejection::PriceSanityFailed { .. })));
    }

    #[test]
    fn price_within_band_passes() {
        let lim = GlobalRiskLimits {
            price_band_bps: 500, // 5%
            ..Default::default()
        };
        let market = Some(Price::from_str("100").unwrap());
        let close_limit = Some(Price::from_str("103").unwrap()); // 3% above
        assert!(check_price_sanity(&lim, "BTC-USD", close_limit, market).is_ok());
    }

    #[test]
    fn sub_tick_lot_size_rejected() {
        let err = check_lot_size(
            "BTC-USD",
            Size::from_str("0.005").unwrap(),
            Decimal::from_str("0.01").unwrap(),
        );
        assert!(matches!(err, Err(RiskRejection::InvalidLotSize { .. })));
    }

    #[test]
    fn exact_lot_size_passes() {
        assert!(check_lot_size(
            "BTC-USD",
            Size::from_str("0.02").unwrap(),
            Decimal::from_str("0.01").unwrap(),
        )
        .is_ok());
    }

    #[test]
    fn daily_loss_at_limit_rejects() {
        let lim = GlobalRiskLimits {
            max_daily_loss_usd: Decimal::from(1000),
            ..Default::default()
        };
        let err = check_daily_loss(&lim, Decimal::from(1000));
        assert!(matches!(
            err,
            Err(RiskRejection::DailyLossLimitExceeded { .. })
        ));
    }

    #[test]
    fn price_not_on_tick_rejected() {
        // M-14: price not a multiple of tick_size must be rejected.
        use domain::money::Price;
        let lp = Some(Price::from_str("100.001").unwrap());
        let tick = Decimal::from_str("0.01").unwrap();
        let err = check_price_tick("BTC-USD", lp, tick);
        assert!(matches!(err, Err(RiskRejection::InvalidTickSize { .. })));
    }

    #[test]
    fn price_on_tick_passes() {
        use domain::money::Price;
        let lp = Some(Price::from_str("100.02").unwrap());
        let tick = Decimal::from_str("0.01").unwrap();
        assert!(check_price_tick("BTC-USD", lp, tick).is_ok());
    }

    #[test]
    fn market_order_no_limit_price_passes_tick_check() {
        assert!(check_price_tick("BTC-USD", None, Decimal::from_str("0.01").unwrap()).is_ok());
    }

    #[test]
    fn rate_second_error_uses_per_second_fields() {
        // M-13: breach reason must identify per-second limit, not per-minute.
        let lim = GlobalRiskLimits {
            max_orders_per_second: 5,
            ..Default::default()
        };
        let err = check_rate_second(&lim, 5).unwrap_err();
        assert!(
            matches!(
                err,
                RiskRejection::RateLimitPerSecondExceeded {
                    orders_last_second: 5,
                    limit_per_second: 5
                }
            ),
            "per-second breach must use RateLimitPerSecondExceeded variant"
        );
    }
}
