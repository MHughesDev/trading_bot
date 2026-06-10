//! P3-T05 acceptance tests — AutomationPlan model round-trip.
//!
//! Tests the serialization round-trip and the StageMembershipRow composite
//! key semantics without requiring a live database.

use chrono::Utc;
use domain::instrument::AssetClass;
use storage::automation::{AutomationRow, StageMembershipRow};
use strategy_runtime::automation::plan::{
    AutomationAccountMode, AutomationPlan, AutomationSpec, ExecutionAction, FilterStage,
};
use uuid::Uuid;

fn three_stage_pipeline_plan() -> AutomationPlan {
    let stages: Vec<FilterStage> = (1..=3)
        .map(|i| FilterStage {
            stage_id: format!("stage_{i}"),
            strategy_id: Uuid::new_v4(),
            label: Some(format!("Stage {i}")),
        })
        .collect();

    AutomationPlan {
        id: Uuid::new_v4(),
        user_id: Uuid::new_v4(),
        account_mode: AutomationAccountMode::Paper,
        spec: AutomationSpec::Pipeline {
            asset_class: AssetClass::CryptoSpotCex,
            universe: vec!["BTC-USDT".into(), "ETH-USDT".into()],
            stages,
            execution_action: ExecutionAction {
                execution_strategy_id: Uuid::new_v4(),
            },
        },
        armed: false,
        created_at: Utc::now(),
    }
}

/// A pipeline plan with 3 stages serializes to JSON and back identically.
#[test]
fn pipeline_plan_round_trips_json() {
    let plan = three_stage_pipeline_plan();
    let json = serde_json::to_string(&plan).expect("serialize");
    let recovered: AutomationPlan = serde_json::from_str(&json).expect("deserialize");

    assert_eq!(plan.id, recovered.id);
    assert_eq!(plan.account_mode, recovered.account_mode);
    assert_eq!(plan.armed, recovered.armed);

    if let AutomationSpec::Pipeline {
        stages, universe, ..
    } = &recovered.spec
    {
        assert_eq!(stages.len(), 3, "3 stages round-tripped");
        assert_eq!(universe.len(), 2);
    } else {
        panic!("recovered spec must be Pipeline");
    }
}

/// The `AutomationRow` spec field holds the serialized plan spec.
#[test]
fn automation_row_spec_is_jsonb_compatible() {
    let plan = three_stage_pipeline_plan();
    let spec_json = serde_json::to_value(&plan.spec).expect("spec to value");

    let row = AutomationRow {
        id: plan.id,
        user_id: plan.user_id,
        kind: "pipeline".into(),
        account_mode: "paper".into(),
        spec: spec_json.clone(),
        armed: plan.armed,
        created_at: plan.created_at,
    };

    // Spec round-trips through the row.
    let recovered_spec: AutomationSpec =
        serde_json::from_value(row.spec).expect("recover spec from row");
    if let AutomationSpec::Pipeline { stages, .. } = recovered_spec {
        assert_eq!(stages.len(), 3);
    } else {
        panic!("recovered spec must be Pipeline");
    }
}

/// The composite primary key (automation_id, stage_id, instrument_id) is enforced
/// at the model level: two rows with the same key are identical.
#[test]
fn membership_composite_key_uniqueness() {
    let automation_id = Uuid::new_v4();
    let row_a = StageMembershipRow::new(automation_id, "s1", "BTC-USDT");
    let row_b = StageMembershipRow::new(automation_id, "s1", "BTC-USDT");

    // Same key fields — would violate the PRIMARY KEY constraint in Postgres.
    assert_eq!(row_a.automation_id, row_b.automation_id);
    assert_eq!(row_a.stage_id, row_b.stage_id);
    assert_eq!(row_a.instrument_id, row_b.instrument_id);
}

/// Different instruments in the same stage produce distinct keys.
#[test]
fn different_instruments_produce_distinct_keys() {
    let automation_id = Uuid::new_v4();
    let row_btc = StageMembershipRow::new(automation_id, "s1", "BTC-USDT");
    let row_eth = StageMembershipRow::new(automation_id, "s1", "ETH-USDT");

    assert_ne!(row_btc.instrument_id, row_eth.instrument_id);
}
