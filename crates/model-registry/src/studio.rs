//! The seam between a training request and the Set N pieces (declaration, families,
//! usage rules), so `manager.rs` calls one function instead of growing a block per piece.
//!
//! Everything here is a function of its arguments or a read/write of the model-platform
//! tables (migrations 0207, 0208, 0209); the manager owns when they are called.
//!
//! # What a declared run does differently
//!
//! * The declaration is validated against *now* as the cutoff (no data past the present
//!   enters a training window), clipped, hashed and lowered. A declaration that uses a field
//!   neither the planner nor the trainer can honour is refused, naming each field
//!   ([`StudioError::Unsupported`]); nothing is silently dropped.
//! * The `(family, variant)` is checked against the registry the trainer also reads
//!   ([`crate::families`]): an unknown entry, a sequence family without a window, a baseline
//!   or foundation entry as the candidate, or a target the family does not list is refused
//!   before any row is written.
//! * The purge and embargo the planner sees are the declaration's derived figures; the request
//!   carries no way to set either.

use chrono::{DateTime, Duration, Utc};
use serde_json::{json, Value};
use sqlx::PgPool;

use crate::dataset_declaration::{
    DatasetDeclaration, DeclarationError, Lowered, LoweringContext, Unsupported, Validated,
};
use crate::families::{self, Family, FamilyError};
use crate::scoring_v2::ForecastUse;
use crate::types::FamilyChoice;

/// Why a training request could not be resolved.
#[derive(Debug, thiserror::Error)]
pub enum StudioError {
    #[error("the dataset declaration is refused: {0}")]
    Declaration(DeclarationError),
    #[error("the declaration uses fields no stage can honour yet: {}", .0.iter().map(|u| format!("{} ({})", u.field, u.why)).collect::<Vec<_>>().join("; "))]
    Unsupported(Vec<Unsupported>),
    #[error("{0}")]
    Family(#[from] FamilyError),
}

/// A resolved Studio request: what the manager dispatches.
#[derive(Debug, Clone)]
pub struct StudioPlan {
    pub validated: Option<Validated>,
    pub lowered: Option<Lowered>,
    /// What the trainer receives as `declaration`: the canonical declaration plus `derived`,
    /// `declaration_hash` and `trainer_fields`.
    pub declaration_json: Option<Value>,
    pub family: Option<&'static Family>,
}

impl StudioPlan {
    /// The training window the declaration fixed (already clipped to the cutoff).
    #[must_use]
    pub fn window(&self) -> Option<(DateTime<Utc>, DateTime<Utc>)> {
        self.validated
            .as_ref()
            .map(|v| (v.declaration.window.start, v.declaration.window.end))
    }

    /// `{family, variant}` as the trainer wants it.
    #[must_use]
    pub fn family_json(&self) -> Option<Value> {
        self.family
            .map(|f| json!({"family": f.family, "variant": f.variant}))
    }
}

/// The declared target's `kind` string (`forward_return`, `realized_vol`, ...).
fn target_kind(d: &DatasetDeclaration) -> Option<String> {
    serde_json::to_value(&d.target)
        .ok()?
        .get("kind")?
        .as_str()
        .map(str::to_string)
}

/// The declaration the trainer receives, from a validated one and what lowered.
#[must_use]
pub fn declaration_json(v: &Validated, trainer_fields: &[&'static str]) -> Value {
    let mut base = serde_json::to_value(&v.declaration).unwrap_or_else(|_| json!({}));
    if let Some(o) = base.as_object_mut() {
        o.insert(
            "derived".into(),
            serde_json::to_value(&v.derived).unwrap_or(Value::Null),
        );
        o.insert("declaration_hash".into(), json!(v.declaration_hash));
        o.insert("trainer_fields".into(), json!(trainer_fields));
        // CUSUM rows were already chosen by the dataset build (dataplane::sampling, N-1.7);
        // handing the trainer the same sampling would thin the thinned rows a second time.
        if matches!(
            v.declaration.sampling,
            crate::dataset_declaration::Sampling::Cusum { .. }
        ) {
            o.insert("sampling".into(), json!({"kind": "every_bar"}));
            o.insert("sampled_by".into(), json!("dataset_build"));
        }
    }
    base
}

/// Resolve the declaration and the family of a training request.
///
/// With neither present (a legacy request) the plan is empty apart from the family the
/// model's `framework` resolves to, recorded for the version row but not enforced.
///
/// # Errors
/// [`StudioError`].
pub fn resolve(
    declaration: Option<&DatasetDeclaration>,
    family: Option<&FamilyChoice>,
    framework: &str,
    output_prefix: &str,
    now: DateTime<Utc>,
) -> Result<StudioPlan, StudioError> {
    let (validated, lowered, decl_json) = match declaration {
        Some(d) => {
            let v = d.validate(Some(now)).map_err(StudioError::Declaration)?;
            let ctx = LoweringContext {
                universe_spec_id: match &v.declaration.universe {
                    crate::dataset_declaration::Universe::List(l) => {
                        format!("explicit:{}", l.join(","))
                    }
                    crate::dataset_declaration::Universe::Named { universe, .. } => {
                        format!("named:{universe}")
                    }
                },
                output_prefix: output_prefix.to_string(),
            };
            let lowered = v.lower_for_trainer(&ctx).map_err(StudioError::Unsupported)?;
            let j = declaration_json(&v, &lowered.trainer_fields);
            (Some(v), Some(lowered), Some(j))
        }
        None => (None, None, None),
    };

    let chosen = match family {
        Some(c) => {
            let target = validated.as_ref().and_then(|v| target_kind(&v.declaration));
            let has_sequence = validated
                .as_ref()
                .is_some_and(|v| v.declaration.sequence.is_some());
            Some(families::check_for_training(
                &c.family,
                &c.variant,
                target.as_deref(),
                has_sequence,
                true,
            )?)
        }
        None => families::for_framework(framework),
    };
    // A declared run that named no family is judged against the family its model's framework
    // implies, so a sequence-needing framework without a window is refused, not run at seq-len 1.
    if let (None, Some(f), Some(v)) = (family, chosen, validated.as_ref()) {
        families::check_for_training(
            &f.family,
            &f.variant,
            target_kind(&v.declaration).as_deref(),
            v.declaration.sequence.is_some(),
            true,
        )?;
    }

    Ok(StudioPlan {
        validated,
        lowered,
        declaration_json: decl_json,
        family: chosen,
    })
}

/// The declaration and family of the model's most recent successful declared run.
pub async fn last_declared_run(
    pg: &PgPool,
    model_id: &str,
) -> Option<(DatasetDeclaration, Option<FamilyChoice>)> {
    let row: Option<(Option<Value>, Option<Value>)> = sqlx::query_as(
        "SELECT declaration_json, family_json FROM training_runs          WHERE model_id = $1 AND status = 'succeeded' AND declaration_json IS NOT NULL          ORDER BY created_at DESC LIMIT 1",
    )
    .bind(model_id)
    .fetch_optional(pg)
    .await
    .ok()
    .flatten();
    let (decl, fam) = row?;
    let d: DatasetDeclaration = serde_json::from_value(strip_derived(decl?)).ok()?;
    let fam = fam.and_then(|f| serde_json::from_value::<FamilyChoice>(f).ok());
    Some((d, fam))
}

/// A model with no explicit request is retrained the way it was last trained: the same
/// declaration, shifted so its window ends now and keeps its length, and the same family.
///
/// A retrain that fell back to the model's defaults would be a silent change of dataset.
/// `None` when the model has never been trained from a declaration.
pub async fn inherit_from_last_run(
    pg: &PgPool,
    model_id: &str,
    now: DateTime<Utc>,
) -> Option<(DatasetDeclaration, Option<FamilyChoice>)> {
    let (mut d, fam) = last_declared_run(pg, model_id).await?;
    let len: Duration = d.window.end - d.window.start;
    d.window.end = now;
    d.window.start = now - len;
    Some((d, fam))
}

/// Raise an Experiment's specification count by what `child` adds beyond `parent`
/// (N-1.6). Additions are counted and removals never subtract
/// ([`crate::dataset_declaration::specification_delta`]); a delta of zero writes nothing.
///
/// # Errors
/// The experiment is unknown, or the database failed.
pub async fn record_specifications(
    pg: &PgPool,
    experiment_id: &str,
    parent: &DatasetDeclaration,
    child: &DatasetDeclaration,
) -> Result<i32, sqlx::Error> {
    let delta = crate::dataset_declaration::specification_delta(parent, child);
    let (total,): (i32,) = sqlx::query_as("SELECT experiment_add_specifications($1, $2)")
        .bind(experiment_id)
        .bind(i32::try_from(delta).unwrap_or(i32::MAX))
        .fetch_one(pg)
        .await?;
    Ok(total)
}

/// The stored JSON carries `derived`, `declaration_hash` and `trainer_fields` beside the
/// declaration; the declaration type refuses unknown fields, so they come off first.
fn strip_derived(mut v: Value) -> Value {
    if let Some(o) = v.as_object_mut() {
        for k in ["derived", "declaration_hash", "trainer_fields"] {
            o.remove(k);
        }
    }
    v
}

/// What a version's forecasts are used for, from its stored definition, family and the
/// declaration it was trained on (see [`crate::verdict::uses_for_definition`]).
pub async fn uses_for_version(pg: &PgPool, model_id: &str, version: i32) -> Vec<ForecastUse> {
    let row: Option<(Value, Option<String>, Option<Value>)> = sqlx::query_as(
        "SELECT m.definition_json, v.family, d.dataset_spec \
           FROM ai_models m \
           JOIN model_versions v ON v.model_id = m.model_id AND v.version = $2 \
           LEFT JOIN dataset_versions d ON d.dataset_version_id = v.dataset_version_id \
          WHERE m.model_id = $1",
    )
    .bind(model_id)
    .bind(version)
    .fetch_optional(pg)
    .await
    .ok()
    .flatten();
    match row {
        Some((def, fam, spec)) => crate::verdict::uses_for_definition(&def, fam.as_deref(), spec.as_ref()),
        None => vec![ForecastUse::Return],
    }
}

/// Count HPO trials on the model's trial counter (migration 0208). Every trial counts.
///
/// # Errors
/// The counting function refusing (`n < 1`) or the database failing.
/// How many HPO trials a training run's metrics report, when it ran a search (N-2.10). The key is
/// the one the trainer's adapters write (`"hpo_trials": n_trials`); a trainer test reads this
/// function so the two cannot drift (the platform once read `trial_count` and the counter never moved).
#[must_use]
pub fn hpo_trials_reported(metrics: &Value) -> Option<i64> {
    metrics.get("hpo_trials").and_then(Value::as_i64).filter(|n| *n >= 1)
}

pub async fn count_hpo_trials(pg: &PgPool, model_id: &str, n: i64) -> Result<i64, sqlx::Error> {
    let (total,): (i64,) = sqlx::query_as("SELECT model_experiment_count($1, $2, TRUE)")
        .bind(model_id)
        .bind(n)
        .fetch_one(pg)
        .await?;
    Ok(total)
}

/// The model's trial counter, or zero when it has none yet.
pub async fn trial_count(pg: &PgPool, model_id: &str) -> i64 {
    sqlx::query_scalar::<_, i64>("SELECT trial_count FROM model_experiments WHERE model_id = $1")
        .bind(model_id)
        .fetch_optional(pg)
        .await
        .ok()
        .flatten()
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decl(_extra: &str) -> DatasetDeclaration {
        serde_json::from_value(json!({
            "universe": ["BTC-USD"],
            "timeframe": "1h",
            "window": {"start": "2024-01-01T00:00:00Z", "end": "2024-03-01T00:00:00Z"},
            "features": ["set:fs_core_ohlcv_v3"],
            "target": {"kind": "forward_return", "horizon_bars": 4},
            "weights": {"kind": "uniqueness"},
            "cv": {"kind": "walk_forward", "train_days": 20, "test_days": 5, "refit_days": 5, "mode": "expanding"},
        }))
        .expect("a valid declaration")
    }

    fn now() -> DateTime<Utc> {
        "2024-06-01T00:00:00Z".parse().unwrap()
    }

    #[test]
    fn a_declared_run_lowers_with_its_derived_embargo_and_the_family_is_recorded() {
        let d = decl("");
        let plan = resolve(Some(&d), None, "lightgbm", "./artifacts", now()).expect("resolves");
        let lowered = plan.lowered.as_ref().expect("lowered");
        let derived = &plan.validated.as_ref().unwrap().derived;
        assert_eq!(lowered.request.split_spec.embargo_bars(), derived.embargo_bars);
        assert_eq!(plan.family.map(|f| f.framework.clone()), Some(Some("lightgbm".to_string())));
        let j = plan.declaration_json.as_ref().unwrap();
        assert!(j["declaration_hash"].as_str().unwrap().len() >= 32);
        assert_eq!(j["derived"]["embargo_bars"], json!(derived.embargo_bars));
    }

    /// N-1.7: CUSUM rows are chosen by the dataset build, so the trainer is told every bar is
    /// already the sample and cannot thin the rows a second time.
    #[test]
    fn a_cusum_declaration_samples_in_the_build_and_the_trainer_is_not_told_to_again() {
        let mut d = decl("");
        d.sampling = crate::dataset_declaration::Sampling::Cusum { h_sigma: 3.0 };
        let plan = resolve(Some(&d), None, "lightgbm", "./a", now()).expect("resolves");
        let lowered = plan.lowered.as_ref().unwrap();
        assert_eq!(
            lowered.request.label_spec.sampling,
            Some(dataplane::sampling::SamplingSpec::Cusum { h_sigma: 3.0 })
        );
        let j = plan.declaration_json.as_ref().unwrap();
        assert_eq!(j["sampling"], json!({"kind": "every_bar"}));
        assert_eq!(j["sampled_by"], json!("dataset_build"));
        // strategy events stay with the trainer, which holds the entry timestamps
        let mut e = decl("");
        e.target = crate::dataset_declaration::Target::MetaLabel {
            strategy_version: "sv1".into(),
            pt_sigma: 2.0,
            sl_sigma: 1.0,
            max_bars: 12,
            path: crate::dataset_declaration::BarrierPath::HighLow,
        };
        e.sampling = crate::dataset_declaration::Sampling::StrategyEvents {
            strategy_version: "sv1".into(),
        };
        if let Ok(p) = resolve(Some(&e), None, "lightgbm", "./a", now()) {
            let j = p.declaration_json.unwrap();
            assert_eq!(j["sampling"]["kind"], json!("strategy_events"));
            assert!(j.get("sampled_by").is_none());
        }
    }

    /// N-2.10: the key the platform reads from a run's metrics is the one the trainer writes.
    #[test]
    fn hpo_trials_are_read_from_the_key_the_trainer_writes() {
        assert_eq!(hpo_trials_reported(&json!({"hpo_trials": 40})), Some(40));
        assert_eq!(hpo_trials_reported(&json!({"hpo_trials": 0})), None);
        assert_eq!(hpo_trials_reported(&json!({"trial_count": 40})), None);
        assert_eq!(hpo_trials_reported(&json!({})), None);
    }

    #[test]
    fn an_unknown_family_is_refused_with_the_known_list() {
        let d = decl("");
        let bad = FamilyChoice {
            family: "gbm".into(),
            variant: "no_such".into(),
        };
        let e = resolve(Some(&d), Some(&bad), "lightgbm", "./a", now()).unwrap_err();
        let msg = e.to_string();
        assert!(msg.contains("unknown family"), "{msg}");
        assert!(msg.contains("known:"), "{msg}");
    }

    #[test]
    fn a_window_past_the_cutoff_is_clipped_and_a_future_dated_training_never_happens() {
        let mut d = decl("");
        d.window.end = "2030-01-01T00:00:00Z".parse().unwrap();
        let plan = resolve(Some(&d), None, "lightgbm", "./a", now()).unwrap();
        assert_eq!(plan.window().unwrap().1, now());
        assert!(plan.validated.unwrap().clipped_to_cutoff);
    }

    #[test]
    fn a_legacy_request_resolves_no_declaration_and_still_names_its_family() {
        let plan = resolve(None, None, "xgboost", "./a", now()).unwrap();
        assert!(plan.lowered.is_none() && plan.declaration_json.is_none());
        assert!(plan.family.is_some());
    }

    #[test]
    fn stored_derived_fields_come_off_before_a_declaration_is_read_back() {
        let d = decl("");
        let plan = resolve(Some(&d), None, "lightgbm", "./a", now()).unwrap();
        let stored = plan.declaration_json.unwrap();
        let back: DatasetDeclaration = serde_json::from_value(strip_derived(stored)).expect("round trips");
        assert_eq!(back.window, plan.validated.unwrap().declaration.window);
    }
}
