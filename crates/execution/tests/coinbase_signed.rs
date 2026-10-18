//! Set E 3.5: the Coinbase broker signs real ES256 JWTs per request and refuses
//! to send an order (BLOCKED:HUMAN). Read-only queries run against a local
//! stand-in, and the token each request carried is verified cryptographically.

#[path = "support/mock_http.rs"]
mod mock_http;

use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine as _;
use domain::{
    money::{Price, Size},
    order::{OrderIntent, OrderType, Side},
};
use execution::broker::{Broker, BrokerError, BrokerOrderState};
use execution::coinbase::{CoinbaseBroker, CoinbaseCredentials};
use ring::rand::SystemRandom;
use ring::signature::{
    EcdsaKeyPair, UnparsedPublicKey, ECDSA_P256_SHA256_FIXED, ECDSA_P256_SHA256_FIXED_SIGNING,
};
use rust_decimal::Decimal;

fn creds() -> CoinbaseCredentials {
    let pkcs8 =
        EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &SystemRandom::new())
            .unwrap();
    let pem = format!(
        "-----BEGIN PRIVATE KEY-----\n{}\n-----END PRIVATE KEY-----\n",
        STANDARD.encode(pkcs8.as_ref())
    );
    CoinbaseCredentials::from_pem("organizations/o/apiKeys/k", &pem).unwrap()
}

fn claims_of(jwt: &str, public: &[u8]) -> serde_json::Value {
    let parts: Vec<&str> = jwt.split('.').collect();
    let sig = URL_SAFE_NO_PAD.decode(parts[2]).unwrap();
    UnparsedPublicKey::new(&ECDSA_P256_SHA256_FIXED, public)
        .verify(format!("{}.{}", parts[0], parts[1]).as_bytes(), &sig)
        .expect("the request's token must verify");
    serde_json::from_slice(&URL_SAFE_NO_PAD.decode(parts[1]).unwrap()).unwrap()
}

fn approved(side: Side) -> risk::ApprovedOrder {
    let gate = risk::RiskGate::new(
        risk::GlobalRiskLimits::default(),
        std::sync::Arc::new(risk::KillSwitch::new(false)),
    );
    let ctx = risk::GateContext::for_manual_order(
        Decimal::ZERO,
        Some(Price::from_decimal(Decimal::from(60000))),
        Decimal::new(1, 2),
        Decimal::new(1, 3),
        Decimal::ZERO,
        true,
        0,
        0,
    );
    gate.check(
        OrderIntent::new(
            "BTC-USD",
            side,
            OrderType::Market,
            Size::from_decimal(Decimal::new(1, 2)),
            None,
            None,
        ),
        &ctx,
    )
    .expect("approved")
}

#[tokio::test]
async fn submit_builds_and_signs_the_request_and_never_sends_it() {
    let mock = mock_http::serve(vec![("POST /api/v3/brokerage/orders", vec![(200, "{}")])]).await;
    let c = creds();
    let public = c.public_key();
    let broker = CoinbaseBroker::new(c).with_base_url(&mock.base);
    let order = approved(Side::Buy);

    // The signed request exists and is correct...
    let prepared = broker.prepare_submit(&order).expect("prepared");
    assert_eq!(prepared.method, "POST");
    let host = mock.base.trim_start_matches("http://");
    let claims = claims_of(prepared.bearer().expect("bearer"), &public);
    assert_eq!(
        claims["uri"],
        format!("POST {host}/api/v3/brokerage/orders")
    );
    let body: serde_json::Value = serde_json::from_str(prepared.body.as_deref().unwrap()).unwrap();
    assert_eq!(
        body["client_order_id"],
        order.intent.idempotency_key.to_string()
    );
    assert_eq!(
        body["order_configuration"]["market_market_ioc"]["base_size"],
        "0.01"
    );

    // ...and submit refuses, having sent nothing.
    match broker.submit(&order).await {
        Err(BrokerError::Rejected(m)) => assert!(m.contains("BLOCKED:HUMAN"), "{m}"),
        other => panic!("submit must refuse, got {other:?}"),
    }
    assert!(matches!(
        broker.cancel("abc").await,
        Err(BrokerError::Rejected(_))
    ));
    assert_eq!(
        mock.hits("POST /api/v3/brokerage/orders")
            + mock.hits("POST /api/v3/brokerage/orders/batch_cancel"),
        0,
        "no order or cancel request may reach the venue"
    );
}

#[tokio::test]
async fn query_order_sends_a_verifiable_token_for_exactly_that_endpoint() {
    let mock = mock_http::serve(vec![(
        "GET /api/v3/brokerage/orders/historical/abc",
        vec![(
            200,
            r#"{"order":{"order_id":"abc","product_id":"BTC-USD","side":"SELL","order_type":"MARKET","status":"FILLED","filled_size":"0.01","average_filled_price":"60010","total_fees":"2.4","order_configuration":{"market_market_ioc":{"base_size":"0.01"}}}}"#,
        )],
    )])
    .await;
    let c = creds();
    let public = c.public_key();
    let broker = CoinbaseBroker::new(c).with_base_url(&mock.base);

    let s = broker.query_order("abc").await.expect("status");
    assert_eq!(s.state, BrokerOrderState::Filled);
    assert_eq!(s.side, Side::Sell);
    assert_eq!(s.filled_qty, Decimal::new(1, 2));

    let head = mock.head_containing("GET /api/v3/brokerage/orders/historical/abc");
    let token = head
        .split(" | ")
        .find_map(|h| {
            h.strip_prefix("authorization: Bearer ")
                .or_else(|| h.strip_prefix("Authorization: Bearer "))
        })
        .expect("a bearer header was sent")
        .to_owned();
    let host = mock.base.trim_start_matches("http://");
    assert_eq!(
        claims_of(&token, &public)["uri"],
        format!("GET {host}/api/v3/brokerage/orders/historical/abc"),
        "the token is bound to the endpoint it was minted for"
    );
}

#[tokio::test]
async fn an_unreadable_open_order_fails_the_listing_instead_of_vanishing() {
    let mock = mock_http::serve(vec![(
        "GET /api/v3/brokerage/orders/historical/batch",
        vec![(200, r#"{"orders":[{"order_id":"x","product_id":"BTC-USD","side":"","order_type":"LIMIT","status":"OPEN","order_configuration":{"limit_limit_gtc":{"base_size":"1"}}}]}"#)],
    )])
    .await;
    let broker = CoinbaseBroker::new(creds()).with_base_url(&mock.base);
    assert!(broker.query_open_orders().await.is_err());
}
