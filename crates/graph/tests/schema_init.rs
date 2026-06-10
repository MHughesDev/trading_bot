//! P7-T01 acceptance tests — TigerGraph schema initialization.
//!
//! Live tests are ignored and require a running TigerGraph instance.

/// Compile-time: the schema module is present and constants are accessible.
#[test]
fn schema_module_compiles() {
    assert!(!graph::schema::ALL_VERTEX_TYPES.is_empty());
    assert!(!graph::schema::ALL_EDGE_TYPES.is_empty());
}

/// All required vertex types are declared in ALL_VERTEX_TYPES.
#[test]
fn required_vertex_types_are_declared() {
    use graph::schema::vertex_types;
    let required = [
        vertex_types::ASSET_CLASS,
        vertex_types::INSTRUMENT,
        vertex_types::VENUE,
        vertex_types::DATA_TYPE,
        vertex_types::STRATEGY_DEFINITION,
        vertex_types::WIDGET,
    ];
    for vt in required {
        assert!(
            graph::schema::ALL_VERTEX_TYPES.contains(&vt),
            "{vt} missing from ALL_VERTEX_TYPES"
        );
    }
}

/// All required edge types are declared in ALL_EDGE_TYPES.
#[test]
fn required_edge_types_are_declared() {
    use graph::schema::edge_types;
    let required = [
        edge_types::INSTRUMENT_IS_A,
        edge_types::VENUE_PROVIDES,
        edge_types::STRATEGY_REQUIRES_DATA,
        edge_types::INSTRUMENT_AT_VENUE,
        edge_types::VENUE_SUPPORTS_ASSET_CLASS,
        edge_types::STRATEGY_FOR_ASSET_CLASS,
    ];
    for et in required {
        assert!(
            graph::schema::ALL_EDGE_TYPES.contains(&et),
            "{et} missing from ALL_EDGE_TYPES"
        );
    }
}

/// The embedded GSQL DDL is non-empty and mentions all vertex types.
#[test]
fn schema_gsql_mentions_vertex_types() {
    let gsql = graph::schema::SCHEMA_GSQL;
    assert!(!gsql.is_empty(), "SCHEMA_GSQL must not be empty");
    for vt in graph::schema::ALL_VERTEX_TYPES {
        assert!(
            gsql.contains(vt),
            "GSQL schema does not mention vertex type {vt}"
        );
    }
}

/// Live: create schema; re-run is idempotent; all types present after init.
#[tokio::test]
#[ignore = "requires a running TigerGraph instance (docker-compose up tigergraph)"]
async fn schema_init_is_idempotent() {
    let cfg = graph::TigerGraphConfig::from_env();
    let client = graph::connect(cfg).await.expect("connect failed");

    client
        .init_schema()
        .await
        .expect("first init_schema failed");
    client
        .init_schema()
        .await
        .expect("second init_schema (idempotency) failed");

    let vtypes = client
        .list_vertex_types()
        .await
        .expect("list_vertex_types failed");
    let etypes = client
        .list_edge_types()
        .await
        .expect("list_edge_types failed");

    for vt in graph::schema::ALL_VERTEX_TYPES {
        assert!(vtypes.iter().any(|v| v == vt), "vertex type {vt} missing");
    }
    for et in graph::schema::ALL_EDGE_TYPES {
        assert!(etypes.iter().any(|e| e == et), "edge type {et} missing");
    }
}
