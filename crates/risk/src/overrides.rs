//! Tighten-only override application.
//!
//! Per-strategy `RiskOverrides` may only tighten global limits — never loosen
//! them.  If any override field would raise a limit above the global value,
//! the gate returns `ValidationError::RiskOverrideTooPermissive`.

use domain::{strategy_def::risk_overrides::RiskOverrides, ValidationError};

use crate::limits::GlobalRiskLimits;

/// Apply `overrides` on top of `global`, enforcing the tighten-only invariant.
///
/// Returns the effective limits (the minimum of each global/override pair).
/// Returns an error if any override would loosen a global limit.
pub fn apply_tighten_only(
    global: &GlobalRiskLimits,
    overrides: &RiskOverrides,
) -> Result<GlobalRiskLimits, ValidationError> {
    let mut effective = global.clone();

    if let Some(override_max_pos) = overrides.max_position {
        if override_max_pos > global.max_position {
            return Err(ValidationError::RiskOverrideTooPermissive {
                field: "max_position".to_owned(),
            });
        }
        effective.max_position = override_max_pos;
    }

    if let Some(override_rate_per_sec) = overrides.max_order_rate_per_second {
        if override_rate_per_sec > global.max_orders_per_second {
            return Err(ValidationError::RiskOverrideTooPermissive {
                field: "max_order_rate_per_second".to_owned(),
            });
        }
        effective.max_orders_per_second = override_rate_per_sec;
    }

    if let Some(override_rate_per_min) = overrides.max_order_rate_per_minute {
        if override_rate_per_min > global.max_orders_per_minute {
            return Err(ValidationError::RiskOverrideTooPermissive {
                field: "max_order_rate_per_minute".to_owned(),
            });
        }
        effective.max_orders_per_minute = override_rate_per_min;
    }

    Ok(effective)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::Decimal;

    fn default_limits() -> GlobalRiskLimits {
        GlobalRiskLimits {
            max_position: Decimal::from(10),
            max_orders_per_second: 5,
            max_orders_per_minute: 60,
            price_band_bps: 500,
            max_daily_loss_usd: Decimal::from(1000),
        }
    }

    #[test]
    fn tightening_override_applied() {
        let global = default_limits();
        let overrides = RiskOverrides {
            max_position: Some(Decimal::from(5)),
            ..Default::default()
        };
        let effective = apply_tighten_only(&global, &overrides).unwrap();
        assert_eq!(effective.max_position, Decimal::from(5));
    }

    #[test]
    fn loosening_override_rejected() {
        let global = default_limits();
        let overrides = RiskOverrides {
            max_position: Some(Decimal::from(50)), // more than global 10
            ..Default::default()
        };
        let err = apply_tighten_only(&global, &overrides);
        assert!(matches!(
            err,
            Err(ValidationError::RiskOverrideTooPermissive { field }) if field == "max_position"
        ));
    }

    #[test]
    fn no_overrides_returns_global_unchanged() {
        let global = default_limits();
        let effective = apply_tighten_only(&global, &RiskOverrides::default()).unwrap();
        assert_eq!(effective.max_position, global.max_position);
        assert_eq!(
            effective.max_orders_per_second,
            global.max_orders_per_second
        );
    }
}
