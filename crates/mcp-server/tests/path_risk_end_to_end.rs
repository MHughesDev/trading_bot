//! `trade_reshuffle_bootstrap` and `cost_shock_curve` over a run that actually
//! went through the simulator (S-3.12).
//!
//! Until now both tools were tested over hand-written trade arrays, and the
//! route under them returned a 422 for every stored run because the engine never
//! recorded trades. `c553b246` made `DetailedOutcome::result_document` carry a
//! `trades` array; this is the end-to-end check that what the engine writes is
//! what the tools read.
//!
//! # What is real and what is stood in
//!
//! **Real:** the market_simulator engine runs a strategy over synthetic bars
//! (`backtest::sim::run_simulation_detailed`, the same function the job manager
//! calls), the document is `DetailedOutcome::result_document()` (the same call
//! `BacktestManager` stores on `backtest_jobs.result`), and the tools are the
//! shipped handlers calling an `ApiClient` over HTTP.
//!
//! **Stood in:** the HTTP peer is a four-line axum route returning
//! `{ "trades": document["trades"] }`, which is what `evidence::trades` does with
//! the stored document (`"trades": raw`). This box's 11 completed
//! `backtest_jobs` rows predate the fix and carry no `trades` key (queried
//! 2026-10-03: 11 completed, 0 with trades), and no run can be added without
//! starting the platform. The route handler's own refusals have their own tests
//! in `routes::evidence`.
//!
//! The synthetic venue run is therefore the "real stored run" for this purpose,
//! and the document it produces is what a post-`c553b246` job row contains.

use std::sync::Arc;

use backtest::sim::{run_simulation_detailed, InstrumentPrecisions, SimulationInputs};
use backtest::store::LoadedBar;
use domain::payloads::bar::Timeframe;
use domain::strategy_def::StrategyDefinition;
use mcp_server_lib::tools::path_risk::{cost_shock_curve, trade_reshuffle_bootstrap};
use mcp_server_lib::ApiClient;
use nautilus_backtest::sdk::SimulationControl;
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use serde_json::{json, Value};

/// A deterministic walk of 1-minute bars, with the close of each the open of the
/// next. SplitMix-style so the test needs no RNG crate and never changes.
fn bars(n: i64) -> Vec<LoadedBar> {
    let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut next = || {
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        (z ^ (z >> 31)) as f64 / u64::MAX as f64
    };
    let mut price = 100.0_f64;
    let mut out = Vec::new();
    for i in 0..n {
        let open = price;
        // +-0.5% per bar: enough to hit a 0.4% stop or target most bars.
        let ret = (next() - 0.5) * 0.010;
        let close = open * (1.0 + ret);
        let high = open.max(close) * 1.0015;
        let low = open.min(close) * 0.9985;
        let d = |x: f64| Decimal::try_from((x * 100.0).round() / 100.0).expect("finite");
        out.push(LoadedBar {
            ts_ns: i * 60_000_000_000,
            open: d(open),
            high: d(high),
            low: d(low),
            close: d(close),
            volume: Decimal::from(10),
            trade_count: 1,
            ..Default::default()
        });
        price = (close * 100.0).round() / 100.0;
    }
    // Five flat bars at the end, so no exit falls on the final bar of the window.
    // An exit on the last bar trips `apply_exit_fills`'s debug assertion (first
    // reproduced by this generator at 266 bars, a `max_holding_bars` exit on bar
    // 265): reported to the owner of `backtest::sim`, not worked around silently.
    for i in n..n + 5 {
        let p = Decimal::try_from(price).expect("finite");
        out.push(LoadedBar {
            ts_ns: i * 60_000_000_000,
            open: p,
            high: p,
            low: p,
            close: p,
            volume: Decimal::from(10),
            trade_count: 1,
            ..Default::default()
        });
    }
    out
}

fn definition() -> StrategyDefinition {
    // Buy on a rising edge of "this bar closed up", sell on the next down bar. No
    // declared exits: every declared-exit configuration this generator produced
    // tripped `apply_exit_fills`'s debug assertion (`the conservative fill never
    // improves a trade`, first reproduced at 266 bars, bar 265), which is reported
    // to the owner of `backtest::sim` and is not this test's subject.
    serde_json::from_str(
        r#"{
            "strategy_id": "up_down_v1",
            "definition_version": "1.0",
            "asset_class": "crypto_spot_cex",
            "inputs": [ { "lane": "market.bars.1m", "instrument": "$bound_at_init" } ],
            "nodes": [
                { "id": "n1", "type": "condition", "expr": "bar('close') > bar('open')" },
                { "id": "n2", "type": "signal", "when": "n1", "emit": "long" },
                { "id": "n3", "type": "condition", "expr": "bar('close') < bar('open')" },
                { "id": "n4", "type": "signal", "when": "n3", "emit": "flat" }
            ],
            "actions": [
                { "on_signal": "long", "type": "place_order",
                  "order": { "side": "buy", "size_mode": "fixed", "size": "1" } },
                { "on_signal": "flat", "type": "place_order",
                  "order": { "side": "sell", "size_mode": "fixed", "size": "1" } }
            ]
        }"#,
    )
    .expect("valid definition")
}

async fn serve(document: Value) -> String {
    use axum::{routing::post, Json, Router};
    let trades = document.get("trades").cloned();
    let app = Router::new().route(
        "/api/evidence/trades",
        post(move |Json(_): Json<Value>| {
            let trades = trades.clone();
            async move {
                // What `evidence::trades` returns for a document with a trade array.
                Json(json!({ "run_id": "synthetic", "trades": trades, "trades_recorded": true }))
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });
    format!("http://{addr}")
}

#[tokio::test(flavor = "multi_thread")]
async fn a_simulated_run_reaches_both_tools_with_the_arithmetic_intact() {
    let inputs = SimulationInputs {
        definition: definition(),
        instrument_id: "BTC-USD".into(),
        venue_id: "coinbase_advanced".into(),
        asset_class: "crypto_spot_cex".into(),
        timeframe: Timeframe::Minutes1,
        quote_currency: "USD".into(),
        initial_balance: Decimal::from(10_000),
        precisions: Some(InstrumentPrecisions { price: 2, size: 4 }),
        sim_start_ns: 0,
        bars: bars(1500),
        features: Vec::new(),
    };
    let control: Arc<SimulationControl> = SimulationControl::new();
    let outcome = run_simulation_detailed(inputs, &control).expect("the engine runs");
    let document = outcome.result_document();

    let rows = document
        .get("trades")
        .and_then(Value::as_array)
        .expect("the document carries a trades array (c553b246)");
    assert!(
        rows.len() >= 30,
        "the fixture must trade enough for the tools' own floor; got {}",
        rows.len()
    );
    for field in ["pnl", "costs_paid", "entry_price", "exit_price", "qty"] {
        assert!(
            rows[0].get(field).is_some(),
            "a stored trade lacks `{field}`, which the path-risk tools read: {}",
            rows[0]
        );
    }

    // Independent arithmetic over the same rows, straight from the engine's own
    // `Trade` values and not through the tools' parser.
    let net: f64 = outcome.trades.iter().map(|t| t.pnl.to_f64().unwrap()).sum();
    let costs: f64 = outcome
        .trades
        .iter()
        .map(|t| t.costs_paid.to_f64().unwrap())
        .sum();
    assert!(costs > 0.0, "the venue charged costs on every leg");

    let url = serve(document.clone()).await;
    let api = ApiClient::new(url, "test-token");

    // cost_shock_curve: the break-even multiple is gross over costs, exactly.
    let curve = cost_shock_curve(
        &api,
        &json!({ "run_id": "00000000-0000-0000-0000-000000000001" }),
    )
    .await;
    assert!(curve.get("error").is_none(), "{curve}");
    let breakeven = curve["breakeven_multiplier"].as_f64().expect("a multiple");
    let expected = (net + costs) / costs;
    assert!(
        (breakeven - expected).abs() < 1e-9 * expected.abs().max(1.0),
        "tool {breakeven} against the engine's own trades {expected}"
    );
    assert_eq!(curve["baseline"]["n_trades"], rows.len());
    // Re-priced at 1x it is the run's own net.
    let at_one = curve["curve"][0]["net_pnl"].as_f64().unwrap();
    assert!(
        (at_one - net).abs() < 1e-6,
        "1x is the run itself: {at_one} vs {net}"
    );

    // trade_reshuffle_bootstrap: mode named, the same seed repeats, and a
    // reshuffle cannot be reported as a worst case.
    let args = json!({
        "run_id": "00000000-0000-0000-0000-000000000001",
        "mode": "reshuffle",
        "n_paths": 2000,
        "seed": 3,
    });
    let a = trade_reshuffle_bootstrap(&api, &args).await;
    let b = trade_reshuffle_bootstrap(&api, &args).await;
    assert!(a.get("error").is_none(), "{a}");
    assert_eq!(a, b, "a seeded resampling must repeat exactly");
    assert_eq!(a["mode"], "reshuffle");
    assert!(
        a.to_string().contains("not_a_worst_case") || a.to_string().contains("worst case"),
        "a reshuffle must say it is not a worst case: {a}"
    );
}
