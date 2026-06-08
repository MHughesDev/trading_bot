//! Tighten-only invariant for `risk_overrides`.
//!
//! Compares each override against `GlobalRiskLimits::default()` — the same
//! values the risk gate applies at order-submission time. Any override that
//! is *more permissive* than the global default is rejected.

use domain::strategy_def::risk_overrides::RiskOverrides;
use risk::GlobalRiskLimits;

use crate::ValidationError;

/// Validate that `overrides` only tightens (does not loosen) the global limits.
pub fn validate_risk_overrides(overrides: &RiskOverrides) -> Vec<ValidationError> {
    let limits = GlobalRiskLimits::default();
    let mut errors = Vec::new();

    if let Some(max_pos) = overrides.max_position {
        if max_pos > limits.max_position {
            errors.push(ValidationError {
                path: "risk_overrides.max_position".into(),
                message: format!(
                    "override {max_pos} exceeds global limit {g}; \
                     risk_overrides may only tighten limits",
                    g = limits.max_position
                ),
            });
        }
    }

    if let Some(rate) = overrides.max_order_rate_per_minute {
        if rate > limits.max_orders_per_minute {
            errors.push(ValidationError {
                path: "risk_overrides.max_order_rate_per_minute".into(),
                message: format!(
                    "override {rate} exceeds global limit {g}; \
                     risk_overrides may only tighten limits",
                    g = limits.max_orders_per_minute
                ),
            });
        }
    }

    if let Some(rate) = overrides.max_order_rate_per_second {
        if rate > limits.max_orders_per_second {
            errors.push(ValidationError {
                path: "risk_overrides.max_order_rate_per_second".into(),
                message: format!(
                    "override {rate} exceeds global limit {g}; \
                     risk_overrides may only tighten limits",
                    g = limits.max_orders_per_second
                ),
            });
        }
    }

    errors
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::Decimal;

    #[test]
    fn no_overrides_is_valid() {
        let o = RiskOverrides::default();
        assert!(validate_risk_overrides(&o).is_empty());
    }

    #[test]
    fn tightening_max_position_is_valid() {
        let o = RiskOverrides {
            max_position: Some(Decimal::from(50)),
            ..Default::default()
        };
        assert!(validate_risk_overrides(&o).is_empty());
    }

    #[test]
    fn loosening_max_position_is_rejected() {
        let o = RiskOverrides {
            max_position: Some(Decimal::from(200)),
            ..Default::default()
        };
        let errs = validate_risk_overrides(&o);
        assert_eq!(errs.len(), 1);
        assert_eq!(errs[0].path, "risk_overrides.max_position");
    }

    #[test]
    fn loosening_rate_per_minute_is_rejected() {
        let o = RiskOverrides {
            max_order_rate_per_minute: Some(120),
            ..Default::default()
        };
        let errs = validate_risk_overrides(&o);
        assert_eq!(errs.len(), 1);
        assert_eq!(errs[0].path, "risk_overrides.max_order_rate_per_minute");
    }

    #[test]
    fn loosening_rate_per_second_is_rejected() {
        let o = RiskOverrides {
            max_order_rate_per_second: Some(10),
            ..Default::default()
        };
        let errs = validate_risk_overrides(&o);
        assert_eq!(errs.len(), 1);
        assert_eq!(errs[0].path, "risk_overrides.max_order_rate_per_second");
    }

    #[test]
    fn multiple_violations_collected() {
        let o = RiskOverrides {
            max_position: Some(Decimal::from(999)),
            max_order_rate_per_minute: Some(999),
            max_order_rate_per_second: Some(999),
        };
        assert_eq!(validate_risk_overrides(&o).len(), 3);
    }
}
