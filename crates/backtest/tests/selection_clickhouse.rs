//! Set U U-6.3: the selection path against a real ClickHouse.
//!
//! `run_selection` is `load_selection` (Postgres) → `load_bars` (ClickHouse,
//! as the store stood at the instant) → `evaluate_over` (pure). The pure half
//! and the Postgres half have their own tests; this runs the ClickHouse half
//! against a server, read-only, over instruments whose bars are stored.
//!
//! ```text
//! CLICKHOUSE_URL=http://user:pass@localhost:8123/trading \
//! SELECTION_INSTRUMENTS=BTC-USD,ETH-USD SELECTION_AS_OF=2026-10-02T20:00:00Z \
//!   cargo test -j 2 -p backtest --test selection_clickhouse -- --ignored --nocapture
//! ```

use std::collections::BTreeSet;

use backtest::selection_run::{evaluate_over, load_bars};
use chrono::Utc;
use domain::strategy_def::params::ParamValues;
use domain::strategy_graph::StrategyGraph;
use strategy_engine::{compile, Membership, StdRegistries};

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs CLICKHOUSE_URL, SELECTION_INSTRUMENTS and SELECTION_AS_OF inside stored 1m bars"]
async fn a_selection_ranks_stored_instruments_by_their_bars_as_of_an_instant() {
    let ch = std::env::var("CLICKHOUSE_URL").expect("CLICKHOUSE_URL");
    let members: Vec<String> = std::env::var("SELECTION_INSTRUMENTS")
        .expect("SELECTION_INSTRUMENTS")
        .split(',')
        .map(str::to_string)
        .collect();
    let g: StrategyGraph = serde_json::from_value(serde_json::json!({
      "format": "tbot.strategy/3", "kind": "selection", "name": "by_close",
      "asset_class": "crypto_spot_cex", "timeframe": "1m",
      "universe": {"kind": "list", "instruments": members},
      "nodes": [{"id":"p", "op":"bar", "args":{"field":{"str":"close"}}}],
      "select": {"score": "p", "top_n": 1, "attributes": {"close": "p"}}
    }))
    .unwrap();
    let plan = std::sync::Arc::new(compile(&g, &StdRegistries::new(), &ParamValues::new()).unwrap());
    let membership = Membership::fixed(members.clone());

    // The store as it stood at the instant, never anything later. (A 1m
    // selection looks back minutes: an instant after the stored bars end
    // correctly excludes every member for having no data.)
    let at: chrono::DateTime<Utc> = std::env::var("SELECTION_AS_OF")
        .expect("SELECTION_AS_OF")
        .parse()
        .unwrap();
    let at_ns = at.timestamp_nanos_opt().unwrap();
    let bars = load_bars(&ch, &plan, &members, at_ns).await.unwrap();
    for (m, inputs) in &bars {
        assert!(
            inputs[0].iter().all(|b| b.knowledge_ns <= at_ns),
            "{m}: a bar known after the instant was read"
        );
        eprintln!("{m}: {} bars as of {at}", inputs[0].len());
    }
    let run = evaluate_over(
        &plan,
        &membership,
        &bars,
        at_ns,
        &mut strategy_engine::env::NoModels,
    )
    .unwrap();
    let s = &run.selection;
    // Every member is accounted for: selected, or excluded with a reason.
    let accounted: BTreeSet<&str> = s
        .selected
        .iter()
        .map(|x| x.instrument.as_str())
        .chain(s.excluded.iter().map(|x| x.instrument.as_str()))
        .collect();
    assert_eq!(accounted.len(), members.len(), "{s:#?}");
    assert_eq!(s.selected.len(), 1, "a member with bars is selected: {s:#?}");
    eprintln!("selected {} at close {:?}", s.selected[0].instrument, s.selected[0].attributes.get("close"));
}
