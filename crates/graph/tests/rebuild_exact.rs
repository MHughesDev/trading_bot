//! P7-T02 acceptance tests — graph population and exact rebuild.
//!
//! Live tests are ignored and require a running TigerGraph instance.

use graph::populate::{InstrumentSpec, RegistrySnapshot, VenueSpec};

/// RegistrySnapshot constructs from explicit data.
#[test]
fn registry_snapshot_builds() {
    let snap = RegistrySnapshot {
        asset_classes: vec!["crypto_spot_cex", "equity"],
        instruments: vec![InstrumentSpec {
            instrument_id: "BTC-USDT".into(),
            asset_class: "crypto_spot_cex".into(),
            venue_id: "kraken".into(),
            active: true,
        }],
        venues: vec![VenueSpec {
            venue_id: "kraken".into(),
            name: "Kraken".into(),
            data_types: vec!["market.ohlcv".into(), "market.trade".into()],
            asset_classes: vec!["crypto_spot_cex".into()],
        }],
        data_types: vec!["market.ohlcv"],
        strategies: vec![],
    };
    assert_eq!(snap.instruments.len(), 1);
    assert_eq!(snap.venues.len(), 1);
}

/// Domain defaults snapshot covers all known asset classes and data types.
#[test]
fn domain_defaults_snapshot_is_non_empty() {
    let snap = RegistrySnapshot::from_domain_defaults();
    assert!(
        !snap.asset_classes.is_empty(),
        "asset_classes must be non-empty"
    );
    assert!(!snap.data_types.is_empty(), "data_types must be non-empty");
}

/// Domain defaults include crypto_spot_cex and market.ohlcv.
#[test]
fn domain_defaults_include_known_types() {
    let snap = RegistrySnapshot::from_domain_defaults();
    assert!(
        snap.asset_classes.iter().any(|a| *a == "crypto_spot_cex"),
        "crypto_spot_cex should be in defaults"
    );
    assert!(
        snap.data_types.iter().any(|d| *d == "market.ohlcv"),
        "market.ohlcv should be in defaults"
    );
    assert!(
        snap.data_types.iter().any(|d| *d == "web.page_snapshot"),
        "web.page_snapshot should be in defaults"
    );
}

/// Live: populate, wipe, rebuild — vertex and edge type counts are identical.
#[tokio::test]
#[ignore = "requires a running TigerGraph instance (docker-compose up tigergraph)"]
async fn wipe_rebuild_produces_identical_snapshot() {
    let cfg = graph::TigerGraphConfig::from_env();
    let client = graph::connect(cfg).await.expect("connect failed");

    client.init_schema().await.expect("schema init failed");

    let snap = RegistrySnapshot::from_domain_defaults();
    client.populate(&snap).await.expect("first populate failed");

    let vtypes_before = client.list_vertex_types().await.unwrap();
    let etypes_before = client.list_edge_types().await.unwrap();

    client.rebuild(&snap).await.expect("rebuild failed");

    let vtypes_after = client.list_vertex_types().await.unwrap();
    let etypes_after = client.list_edge_types().await.unwrap();

    assert_eq!(vtypes_before, vtypes_after, "vertex types must match");
    assert_eq!(etypes_before, etypes_after, "edge types must match");
}
