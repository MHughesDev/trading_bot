//! Lane classification: public (shareable) vs private (per-user scoped).

/// Returns `true` if `lane_str` is a private lane that must be scoped per authenticated user.
///
/// Private lanes: orders.*, positions.*, strategy.*
pub fn is_private_lane(lane_str: &str) -> bool {
    matches!(
        lane_str,
        "orders.commands" | "orders.events" | "positions.events" | "strategy.signals"
    )
}

/// Returns `true` if `lane_str` is public and may be shared across users.
pub fn is_public_lane(lane_str: &str) -> bool {
    !is_private_lane(lane_str)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn market_lanes_are_public() {
        assert!(is_public_lane("market.bars.1m"));
        assert!(is_public_lane("market.trades"));
        assert!(is_public_lane("market.orderbook.l2"));
        assert!(is_public_lane("features.technical"));
        assert!(is_public_lane("ui.orderbook.snapshot"));
    }

    #[test]
    fn order_and_position_lanes_are_private() {
        assert!(is_private_lane("orders.commands"));
        assert!(is_private_lane("orders.events"));
        assert!(is_private_lane("positions.events"));
        assert!(is_private_lane("strategy.signals"));
    }
}
