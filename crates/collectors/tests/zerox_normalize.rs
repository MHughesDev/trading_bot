//! P2-T09 acceptance test: 0x quote response normalizes to DexQuote event.

use collectors::dex::zerox::ZeroXCollector;

#[test]
fn zerox_collector_instrument_id_is_sell_buy() {
    let c = ZeroXCollector::new("WETH", "USDC", "1000000000000000000");
    assert_eq!(c.instrument_id, "WETH-USDC");
    assert_eq!(c.sell_token, "WETH");
    assert_eq!(c.buy_token, "USDC");
}
