//! The one way a v3 strategy starts on paper (Set U U-8.3): record, then
//! declare — from `POST /api/deployments` and from an armed automation alike.
//!
//! ```text
//! DATABASE_URL=… cargo test -j 2 -p api --test pg_record_and_declare -- --ignored
//! ```
//! Needs migrations 0142–0144 and 0148.

use api::routes::deployments::{record_and_declare, stop_started_by, DeclareError};
use rust_decimal::Decimal;
use storage::postgres::deployments::{self, NewDeployment};
use uuid::Uuid;

fn graph() -> domain::strategy_graph::StrategyGraph {
    serde_json::from_value(serde_json::json!({
      "format": "tbot.strategy/3", "kind": "trading", "name": "rd_test",
      "asset_class": "crypto_spot_cex", "timeframe": "1h",
      "nodes": [{"id":"go","op":"ge","args":{"a":"close","b":0}}],
      "decide": {"rules": [{"when":"go","then":{"kind":"hold"}}]}
    }))
    .unwrap()
}

fn record(user: &str, instrument: &str, started_by: &str) -> NewDeployment {
    let g = graph();
    NewDeployment {
        deployment_id: Uuid::new_v4(),
        user_id: user.into(),
        strategy_name: g.name.clone(),
        revision_hash: g.revision().unwrap().as_str().to_string(),
        definition_json: serde_json::to_value(&g).unwrap(),
        instrument_id: instrument.into(),
        starting_capital: Decimal::from(1_000),
        model_pins: serde_json::json!([]),
        started_by: started_by.into(),
        forward_deployment_id: None,
        target: None,
        parent_deployment_id: None,
    }
}

#[tokio::test]
#[ignore = "needs DATABASE_URL with migrations 0142-0144 and 0148"]
async fn a_deployment_is_recorded_declared_and_stopped_by_who_started_it() {
    let pg = sqlx::PgPool::connect(&std::env::var("DATABASE_URL").expect("DATABASE_URL"))
        .await
        .unwrap();
    let user = format!("u-{}", Uuid::new_v4());
    let automation = format!("automation:{}", Uuid::new_v4());
    let first = record(&user, "RD-BTC-USD", &automation);
    record_and_declare(&pg, &first, graph()).await.unwrap();
    let registry = strategy_runtime::live::global();
    assert!(
        registry.is_v3_declared(first.deployment_id),
        "declared live"
    );
    assert!(deployments::running(&pg)
        .await
        .unwrap()
        .iter()
        .any(|r| r.deployment_id == first.deployment_id && r.started_by == automation));

    // The same binding again is refused by the record, and nothing is declared.
    let again = record(&user, "RD-BTC-USD", &automation);
    assert!(matches!(
        record_and_declare(&pg, &again, graph()).await,
        Err(DeclareError::NotRecorded(_))
    ));
    assert!(!registry.is_v3_declared(again.deployment_id));

    // A deployment someone else started is untouched by stopping this one's.
    let other = record(&user, "RD-ETH-USD", "someone-else");
    record_and_declare(&pg, &other, graph()).await.unwrap();
    assert_eq!(
        stop_started_by(&pg, &automation, "automation disarmed").await,
        1
    );
    assert!(!registry.is_v3_declared(first.deployment_id), "withdrawn");
    assert!(registry.is_v3_declared(other.deployment_id));
    let row = deployments::get(&pg, first.deployment_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.stop_reason.as_deref(), Some("automation disarmed"));
    assert_eq!(
        stop_started_by(&pg, &automation, "again").await,
        0,
        "stopped once"
    );
    stop_started_by(&pg, "someone-else", "test cleanup").await;
}
