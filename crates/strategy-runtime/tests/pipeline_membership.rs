//! P3-T06 acceptance tests — stateful pipeline stage membership.

use chrono::Utc;
use uuid::Uuid;

use strategy_runtime::automation::pipeline::PipelineRuntime;
use strategy_runtime::automation::plan::{
    AutomationAccountMode, AutomationPlan, AutomationSpec, ExecutionAction, FilterStage,
};

fn make_plan(universe: Vec<String>, stages: Vec<FilterStage>) -> AutomationPlan {
    AutomationPlan {
        id: Uuid::new_v4(),
        user_id: Uuid::new_v4(),
        account_mode: AutomationAccountMode::Paper,
        spec: AutomationSpec::Pipeline {
            asset_class: domain::instrument::AssetClass::CryptoSpotCex,
            universe,
            stages,
            execution_action: ExecutionAction {
                execution_strategy_id: Uuid::new_v4(),
            },
        },
        armed: false,
        created_at: Utc::now(),
    }
}

fn stage(id: &str) -> FilterStage {
    FilterStage {
        stage_id: id.into(),
        strategy_id: Uuid::new_v4(),
        label: None,
    }
}

/// An instrument that newly passes stage 1 produces an enter delta.
#[test]
fn instrument_entering_stage_produces_enter_delta() {
    let plan = make_plan(vec!["AAPL".into(), "MSFT".into()], vec![stage("s1")]);
    let mut rt = PipelineRuntime::new();

    // First eval: AAPL passes, MSFT does not.
    let result = rt.evaluate(&plan, |_stage, inst| inst == "AAPL");
    assert_eq!(result.deltas.len(), 1);
    let delta = &result.deltas[0];
    assert_eq!(delta.stage_id, "s1");
    assert!(delta.entered.contains(&"AAPL".to_string()));
    assert!(delta.exited.is_empty());
}

/// An instrument that stops passing a stage produces an exit delta.
#[test]
fn instrument_exiting_stage_produces_exit_delta() {
    let plan = make_plan(vec!["BTC-USDT".into()], vec![stage("s1")]);
    let mut rt = PipelineRuntime::new();

    // First eval: BTC passes.
    rt.evaluate(&plan, |_, inst| inst == "BTC-USDT");

    // Second eval: BTC no longer passes.
    let result = rt.evaluate(&plan, |_, _inst| false);
    assert_eq!(result.deltas.len(), 1);
    let delta = &result.deltas[0];
    assert_eq!(delta.stage_id, "s1");
    assert!(delta.entered.is_empty());
    assert!(delta.exited.contains(&"BTC-USDT".to_string()));
}

/// An instrument that clears the final stage is handed to execution exactly once per crossing.
#[test]
fn instrument_clearing_final_stage_appears_in_final_cleared() {
    let plan = make_plan(
        vec!["ETH-USDT".into(), "SOL-USDT".into()],
        vec![stage("s1"), stage("s2")],
    );
    let mut rt = PipelineRuntime::new();

    // ETH passes both stages; SOL passes only s1.
    let result = rt.evaluate(&plan, |stage, inst| {
        if stage.stage_id == "s1" {
            true // all pass
        } else {
            inst == "ETH-USDT"
        }
    });

    assert!(
        result.final_stage_cleared.contains(&"ETH-USDT".to_string()),
        "ETH-USDT clears both stages"
    );
    assert!(
        !result.final_stage_cleared.contains(&"SOL-USDT".to_string()),
        "SOL-USDT does not clear stage 2"
    );
}

/// No delta is emitted when membership is unchanged between evaluations.
#[test]
fn no_delta_when_membership_unchanged() {
    let plan = make_plan(vec!["X".into()], vec![stage("s1")]);
    let mut rt = PipelineRuntime::new();

    rt.evaluate(&plan, |_, _| true);
    let result = rt.evaluate(&plan, |_, _| true);

    // X was already a member; no change → no delta.
    assert!(result.deltas.is_empty());
}
