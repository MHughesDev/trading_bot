//! Kalshi is **exposure only** (DATA-005 §11 S7; Set P, P-3.4).
//!
//! The acceptance criterion is "no trading endpoint is reachable from this
//! adapter, proven by a test over its client surface". A test that merely calls
//! the public functions and observes that none of them place an order proves
//! nothing about the function somebody adds next week, so this test reads the
//! adapter's own source and fails on the appearance of any trading path or any
//! writing HTTP verb.
//!
//! Kalshi's trading surface lives under `/portfolio/*` in the same
//! `trade-api/v2` namespace as the market data this adapter reads, which is
//! exactly why the fence has to be mechanical: the base URL is one path segment
//! away from the ability to place an order.

/// The live quote poller.
const KALSHI_SRC: &str = include_str!("../src/prediction/kalshi.rs");
/// The history reader added by P-3.4.
const HISTORY_SRC: &str = include_str!("../src/prediction/history.rs");

/// Paths and verbs that would turn exposure into execution.
const FORBIDDEN: &[&str] = &[
    "/portfolio/orders",
    "/portfolio/positions",
    "/portfolio/fills",
    "/portfolio/balance",
    "create_order",
    "place_order",
    "cancel_order",
    "batch_create_orders",
    "batch_cancel_orders",
    "decrease_order",
];

/// Written HTTP verbs. The market-data surface is entirely GET.
const FORBIDDEN_VERBS: &[&str] = &[".post(", ".put(", ".patch(", ".delete("];

fn assert_exposure_only(name: &str, src: &str) {
    for needle in FORBIDDEN {
        assert!(
            !src.contains(needle),
            "{name} references Kalshi trading surface {needle:?}; the adapter is exposure only \
             (DATA-005 §11 S7). If a trading path is genuinely wanted, it is a new ADR and a \
             different crate, not an edit here."
        );
    }
    for verb in FORBIDDEN_VERBS {
        assert!(
            !src.contains(verb),
            "{name} issues a writing HTTP request ({verb}); the Kalshi adapter reads market data \
             and nothing else."
        );
    }
}

#[test]
fn the_kalshi_quote_adapter_reaches_no_trading_endpoint() {
    assert_exposure_only("prediction::kalshi", KALSHI_SRC);
}

#[test]
fn the_kalshi_history_adapter_reaches_no_trading_endpoint() {
    assert_exposure_only("prediction::history", HISTORY_SRC);
}

/// The guard is only meaningful if it would actually fire.
#[test]
fn the_guard_catches_a_planted_trading_call() {
    let planted = r#"let url = format!("{KALSHI_REST_BASE}/portfolio/orders");"#;
    let outcome = std::panic::catch_unwind(|| assert_exposure_only("planted", planted));
    assert!(
        outcome.is_err(),
        "the exposure-only guard must reject a planted trading call"
    );
}

/// The declared lag exists on the source row, which is the other half of P-3.4's
/// acceptance: a Kalshi series read through the Data API carries a stated lag
/// rather than an implied zero.
#[test]
fn the_kalshi_sources_declare_their_lag() {
    for name in ["kalshi_rest", "kalshi_history"] {
        let lag = dataplane::source::declared_lag_ms(name)
            .unwrap_or_else(|e| panic!("{name} must be in the source registry: {e}"));
        assert!(lag > 0, "{name} must declare a non-zero lag for history reads");
    }
}
