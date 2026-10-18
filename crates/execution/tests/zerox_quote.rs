//! Set E 3.3: the paper DEX path quotes from 0x's real price endpoint, with real
//! token decimals, over HTTP — and the live adapter refuses to pretend to trade.

#[path = "support/mock_http.rs"]
mod mock_http;

use domain::{
    money::{Price, Size},
    order::{OrderIntent, OrderType, Side},
};
use execution::venues::zerox::ZeroXQuoter;
use rust_decimal::Decimal;

fn intent(side: Side, size: i64) -> OrderIntent {
    OrderIntent::new(
        "WETH-USDC",
        side,
        OrderType::Market,
        Size::from_decimal(Decimal::from(size)),
        None,
        None,
    )
}

#[tokio::test]
async fn a_sell_quote_is_requested_in_wei_and_priced_in_usdc() {
    let mock = mock_http::serve(vec![(
        "GET /swap/permit2/price",
        vec![(
            200,
            r#"{"sellAmount":"1000000000000000000","buyAmount":"2500000000","totalNetworkFee":"6000000000000000","liquidityAvailable":true}"#,
        )],
    )])
    .await;
    let quoter = ZeroXQuoter::new("key-1").with_base_url(&mock.base);
    let quote = quoter
        .fetch_quote(
            &intent(Side::Sell, 1),
            Price::from_decimal(Decimal::from(2500)),
            Decimal::from(2500),
        )
        .await
        .expect("quote");
    assert_eq!(quote.effective_price.inner(), Decimal::from(2500));
    assert_eq!(quote.fee_usd, Decimal::from(15));

    let head = mock.head_containing("GET /swap/permit2/price");
    assert!(
        head.contains("sellAmount=1000000000000000000"),
        "1 WETH in wei: {head}"
    );
    assert!(head.contains("chainId=1"), "{head}");
    assert!(
        head.to_lowercase().contains("0x-api-key: key-1")
            && head.to_lowercase().contains("0x-version: v2"),
        "{head}"
    );
    assert!(
        head.contains("sellToken=0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2"),
        "{head}"
    );
}

#[tokio::test]
async fn a_buy_sizes_its_usdc_input_with_six_decimals() {
    let mock = mock_http::serve(vec![(
        "GET /swap/permit2/price",
        vec![(
            200,
            r#"{"sellAmount":"2550000000","buyAmount":"1000000000000000000","totalNetworkFee":"0","liquidityAvailable":true}"#,
        )],
    )])
    .await;
    let quoter = ZeroXQuoter::new("k").with_base_url(&mock.base);
    let quote = quoter
        .fetch_quote(
            &intent(Side::Buy, 1),
            Price::from_decimal(Decimal::from(2550)),
            Decimal::from(2500),
        )
        .await
        .expect("quote");
    assert_eq!(quote.effective_price.inner(), Decimal::from(2550));
    let head = mock.head_containing("GET /swap/permit2/price");
    assert!(
        head.contains("sellAmount=2550000000"),
        "2,550 USDC is 2,550,000,000 units at 6 decimals, not 10^21: {head}"
    );
}

#[tokio::test]
async fn an_unknown_token_or_no_liquidity_is_an_error() {
    let quoter = ZeroXQuoter::new("k");
    let mut odd = intent(Side::Sell, 1);
    odd.instrument_id = "MYSTERY-USDC".to_owned();
    assert!(quoter
        .price_request("MYSTERY-USDC", Side::Sell, Decimal::ONE, Decimal::ONE)
        .is_err());
    assert!(quoter
        .price_request("WETHUSDC", Side::Sell, Decimal::ONE, Decimal::ONE)
        .is_err());

    let mock = mock_http::serve(vec![(
        "GET /swap/permit2/price",
        vec![(200, r#"{"liquidityAvailable":false}"#)],
    )])
    .await;
    let quoter = ZeroXQuoter::new("k").with_base_url(&mock.base);
    assert!(quoter
        .fetch_quote(
            &intent(Side::Sell, 1),
            Price::from_decimal(Decimal::ONE),
            Decimal::ONE
        )
        .await
        .is_err());
}

#[tokio::test]
async fn the_live_zerox_broker_refuses_instead_of_fabricating_an_order_id() {
    use execution::broker::Broker;
    use execution::venues::ZeroXBroker;
    let gate = risk::RiskGate::new(
        risk::GlobalRiskLimits::default(),
        std::sync::Arc::new(risk::KillSwitch::new(false)),
    );
    let ctx = risk::GateContext::for_manual_order(
        Decimal::ZERO,
        Some(Price::from_decimal(Decimal::from(2500))),
        Decimal::new(1, 2),
        Decimal::new(1, 3),
        Decimal::ZERO,
        true,
        0,
        0,
    );
    let approved = gate.check(intent(Side::Buy, 1), &ctx).expect("approved");
    let err = ZeroXBroker::new("k").submit(&approved).await.unwrap_err();
    assert!(err.to_string().contains("nothing was broadcast"), "{err}");
}
