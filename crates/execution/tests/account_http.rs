//! Set E 3.1 / 3.2: the Tradier and Tradovate `AccountSource` adapters over real
//! HTTP, against a local stand-in server.
//!
//! The stand-in (`support/mock_http.rs`) records every request head, so the
//! tests assert on what the adapter *sent* — the bearer token, the number of
//! token exchanges, the retry after a 401 — and not only on what it parsed.

#[path = "support/mock_http.rs"]
mod mock_http;

use execution::account::{TradierAccountSource, TradovateAccountSource};
use execution::account_source::{AccountSource, AccountSourceError, VenueCredentials};
use mock_http::serve;
use rust_decimal::Decimal;
use uuid::Uuid;

fn creds(s: &str) -> VenueCredentials {
    VenueCredentials {
        venue: "test".into(),
        plaintext: s.as_bytes().to_vec(),
    }
}

// ── Tradier ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn tradier_returns_real_balances_positions_and_history() {
    let mock = serve(vec![
        (
            "GET /accounts/ACC1/balances",
            vec![(200, r#"{"balances":{"total_cash":1000,"total_equity":2500,"cash":{"cash_available":900}}}"#)],
        ),
        (
            "GET /accounts/ACC1/positions",
            vec![(200, r#"{"positions":{"position":{"symbol":"AAPL","quantity":10,"cost_basis":1500}}}"#)],
        ),
        (
            "GET /accounts/ACC1/history",
            vec![(
                200,
                r#"{"history":{"event":[{"amount":-1500,"date":"2026-09-01T00:00:00Z","type":"trade","trade":{"symbol":"AAPL"}}]}}"#,
            )],
        ),
    ])
    .await;
    let src = TradierAccountSource::new().with_base_url(&mock.base);
    let c = creds("tok123:ACC1");

    let bal = src.fetch_balances(&c).await.expect("balances");
    assert_eq!(bal[0].available.inner(), Decimal::from(900));
    assert_eq!(bal[0].locked.inner(), Decimal::from(100));

    let pos = src.fetch_positions(&c).await.expect("positions");
    assert_eq!(pos[0].instrument_id, "AAPL");
    assert_eq!(pos[0].avg_entry_price.inner(), Decimal::from(150));

    let tx = src
        .fetch_transactions(&c, Uuid::new_v4(), None)
        .await
        .expect("history");
    assert_eq!(tx.len(), 1);
    assert_eq!(tx[0].instrument_id.as_deref(), Some("AAPL"));

    assert!(
        mock.head_containing("GET /accounts/ACC1/balances")
            .to_lowercase()
            .contains("authorization: bearer tok123"),
        "the token must go out as a bearer header"
    );
}

#[tokio::test]
async fn tradier_surfaces_an_http_error_body_and_never_returns_not_implemented() {
    let mock = serve(vec![(
        "GET /accounts/ACC1/balances",
        vec![(401, r#"{"fault":"Invalid Access Token"}"#)],
    )])
    .await;
    let src = TradierAccountSource::new().with_base_url(&mock.base);
    match src.fetch_balances(&creds("bad:ACC1")).await {
        Err(AccountSourceError::HttpStatus(body)) => assert!(body.contains("Invalid Access Token")),
        other => panic!("expected the venue's error body, got {other:?}"),
    }
}

// ── Tradovate ────────────────────────────────────────────────────────────────

const TOKEN_OK: &str = r#"{"accessToken":"AT-1","expirationTime":"2099-01-01T00:00:00Z"}"#;

#[tokio::test]
async fn tradovate_exchanges_once_caches_the_token_and_resolves_contract_ids() {
    let mock = serve(vec![
        ("POST /auth/accesstoken", vec![(200, TOKEN_OK)]),
        ("GET /account/list", vec![(200, r#"[{"id":7,"name":"DEMO7"}]"#)]),
        (
            "POST /cashBalance/getcashbalancesnapshot",
            vec![(200, r#"{"totalCashValue":10000,"initialMargin":2500,"openPnL":100}"#)],
        ),
        (
            "GET /position/list",
            vec![(200, r#"[{"contractId":11,"netPos":2,"netPrice":5000.25},{"contractId":12,"netPos":0,"netPrice":0}]"#)],
        ),
        ("GET /contract/item", vec![(200, r#"{"id":11,"name":"ESM6"}"#)]),
    ])
    .await;
    let src = TradovateAccountSource::new().with_base_url(&mock.base);
    let c = creds("alice:hunter2");

    let bal = src.fetch_balances(&c).await.expect("balances");
    assert_eq!(bal[0].available.inner(), Decimal::from(7500));
    let pos = src.fetch_positions(&c).await.expect("positions");
    assert_eq!(pos.len(), 1, "the closed row is dropped");
    assert_eq!(
        pos[0].instrument_id, "ESM6",
        "contractId resolved to a symbol"
    );

    assert_eq!(
        mock.hits("POST /auth/accesstoken"),
        1,
        "one exchange serves every call until the token nears expiry"
    );
    assert!(mock
        .head_containing("GET /account/list")
        .to_lowercase()
        .contains("authorization: bearer at-1"));
    // The contract is resolved once however many times it is asked for.
    src.fetch_positions(&c).await.expect("positions again");
    assert_eq!(mock.hits("GET /contract/item"), 1);
}

#[tokio::test]
async fn tradovate_renews_after_a_401_and_retries_once() {
    let mock = serve(vec![
        (
            "POST /auth/accesstoken",
            vec![
                (
                    200,
                    r#"{"accessToken":"AT-OLD","expirationTime":"2099-01-01T00:00:00Z"}"#,
                ),
                (
                    200,
                    r#"{"accessToken":"AT-NEW","expirationTime":"2099-01-01T00:00:00Z"}"#,
                ),
            ],
        ),
        (
            "GET /fill/list",
            vec![
                (401, r#"{"errorText":"Expired Access Token"}"#),
                (200, "[]"),
            ],
        ),
    ])
    .await;
    let src = TradovateAccountSource::new().with_base_url(&mock.base);
    let tx = src
        .fetch_transactions(&creds("alice:pw"), Uuid::new_v4(), None)
        .await
        .expect("the retry with a fresh token succeeds");
    assert!(tx.is_empty());
    assert_eq!(
        mock.hits("POST /auth/accesstoken"),
        2,
        "the second exchange is the renewal"
    );
    assert_eq!(mock.hits("GET /fill/list"), 2);
}

#[tokio::test]
async fn tradovate_gives_up_after_one_retry_and_reports_a_throttled_login() {
    let mock = serve(vec![
        ("POST /auth/accesstoken", vec![(200, TOKEN_OK)]),
        ("GET /position/list", vec![(401, r#"{"errorText":"nope"}"#)]),
    ])
    .await;
    let src = TradovateAccountSource::new().with_base_url(&mock.base);
    assert!(src.fetch_positions(&creds("alice:pw")).await.is_err());
    assert_eq!(mock.hits("GET /position/list"), 2, "one retry, not a loop");

    let throttled = serve(vec![(
        "POST /auth/accesstoken",
        vec![(200, r#"{"p-ticket":"x","p-time":30,"p-captcha":false}"#)],
    )])
    .await;
    let src = TradovateAccountSource::new().with_base_url(&throttled.base);
    match src.fetch_balances(&creds("alice:pw")).await {
        Err(AccountSourceError::HttpStatus(m)) => assert!(m.contains("throttling"), "{m}"),
        other => panic!("expected a throttle error, got {other:?}"),
    }
}
