//! Trust tier gate — refuses orders derived from data below the strategy's
//! declared `min_trust_tier`.

use domain::{RiskRejection, TrustTier};

/// Returns `Ok(())` if `event_tier >= min_required`.
pub fn check_trust(event_tier: TrustTier, min_required: TrustTier) -> Result<(), RiskRejection> {
    if event_tier < min_required {
        return Err(RiskRejection::TrustTierInsufficient {
            required: format!("{min_required:?}"),
            actual: format!("{event_tier:?}"),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equal_tier_passes() {
        assert!(check_trust(
            TrustTier::CentralizedExchange,
            TrustTier::CentralizedExchange
        )
        .is_ok());
    }

    #[test]
    fn higher_tier_passes() {
        assert!(check_trust(TrustTier::Regulated, TrustTier::CentralizedExchange).is_ok());
    }

    #[test]
    fn lower_tier_rejected() {
        let err = check_trust(TrustTier::SocialDerived, TrustTier::CentralizedExchange);
        assert!(matches!(
            err,
            Err(RiskRejection::TrustTierInsufficient { .. })
        ));
    }
}
