//! Contracts on model versions (Set U U-5.1; ADR-0043 decision 1).
//!
//! Every model version a strategy may call carries an `IoContract`. Versions
//! that predate contracts get the one their serving path actually honours,
//! derived from their kind, their definition and — when the trainer reported it
//! — the bundle's training feature order ([`derived_contract`]). A `transform`
//! is custom code with nothing to derive from; it stays without a contract until
//! one is declared, and the strategy compiler refuses to reference it with that
//! fix.
//!
//! [`backfill`] runs at platform start, after migrations, and is idempotent: it
//! touches only versions with no contract, and a derived contract never
//! replaces a declared one.

use domain::model_def::contract::{derived_contract, IoContract};
use domain::model_def::ModelDefinition;

/// What a backfill did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BackfillReport {
    /// Versions that now have a derived contract.
    pub derived: usize,
    /// Versions left without one, and why — each must be declared by hand.
    pub left: Vec<(String, i32, String)>,
}

/// The contract a version gets from its definition and training metrics, or
/// why it gets none.
///
/// # Errors
/// The reason no contract can be derived: a definition that does not parse, or
/// a kind (`transform`) that must declare its own.
pub fn derive_for(
    definition_json: &serde_json::Value,
    metrics_json: Option<&serde_json::Value>,
) -> Result<IoContract, String> {
    let def: ModelDefinition = serde_json::from_value(definition_json.clone())
        .map_err(|e| format!("definition does not parse: {e}"))?;
    let feature_order: Option<Vec<String>> = metrics_json
        .and_then(|m| m.get("feature_order"))
        .and_then(|f| serde_json::from_value(f.clone()).ok());
    derived_contract(def.model_kind, &def, feature_order.as_deref()).ok_or_else(|| {
        format!(
            "a {:?} declares its own contract; there is nothing to derive one from",
            def.model_kind
        )
    })
}

/// Give every version without a contract the one derived from it.
///
/// # Errors
/// A database error.
pub async fn backfill(pg: &sqlx::PgPool) -> Result<BackfillReport, storage::postgres::PgError> {
    let mut report = BackfillReport::default();
    for (model_id, version, definition, metrics) in
        storage::postgres::models::versions_without_contract(pg).await?
    {
        match derive_for(&definition, metrics.as_ref()) {
            Ok(contract) => {
                let value = serde_json::to_value(&contract).unwrap_or_default();
                if storage::postgres::models::set_version_contract(
                    pg, &model_id, version, &value, false,
                )
                .await?
                {
                    report.derived += 1;
                }
            }
            Err(reason) => report.left.push((model_id, version, reason)),
        }
    }
    Ok(report)
}

/// Declare a version's contract by hand — the only way a `transform` gets one,
/// and a way to correct a derived one.
///
/// # Errors
/// A contract that does not validate, or a database error.
pub async fn declare(
    pg: &sqlx::PgPool,
    model_id: &str,
    version: i32,
    contract: &serde_json::Value,
) -> anyhow::Result<bool> {
    let parsed: IoContract = serde_json::from_value(contract.clone())
        .map_err(|e| anyhow::anyhow!("contract does not validate: {e}"))?;
    let canonical = serde_json::to_value(&parsed)?;
    Ok(
        storage::postgres::models::set_version_contract(pg, model_id, version, &canonical, true)
            .await?,
    )
}

/// Why a transform was refused.
#[derive(Debug, thiserror::Error)]
pub enum TransformError {
    #[error("contract does not validate: {0}")]
    Contract(String),
    #[error("the dry run did not answer in the contract's shape: {0}")]
    DryRun(String),
    #[error("database: {0}")]
    Db(#[from] storage::postgres::PgError),
    #[error("database: {0}")]
    Sqlx(#[from] sqlx::Error),
}

/// A sample row for a dry run: every row feature and numeric scalar at 1.0.
fn sample_row(contract: &IoContract) -> std::collections::BTreeMap<String, f64> {
    use domain::model_def::contract::{FeatureSource, InputShape};
    let mut row = std::collections::BTreeMap::new();
    for ci in &contract.inputs {
        match &ci.shape {
            InputShape::Row {
                features: FeatureSource::Named { features },
            } => {
                row.extend(features.iter().map(|f| (f.clone(), 1.0)));
            }
            InputShape::Scalar { .. } => {
                row.insert(ci.name.clone(), 1.0);
            }
            _ => {}
        }
    }
    row
}

/// Call a transform once, on a sample row, and check its answer types against
/// the declared contract — the registration-time typecheck of code that cannot
/// be typechecked statically.
///
/// # Errors
/// A contract that does not validate, or a dry run that does not answer every
/// declared output in its type.
pub async fn dry_run_transform(
    client: &dyn crate::provider::V2Client,
    name: &str,
    contract: &serde_json::Value,
    spec: &domain::model_def::transform::TransformSpec,
) -> Result<IoContract, TransformError> {
    let parsed: IoContract = serde_json::from_value(contract.clone())
        .map_err(|e| TransformError::Contract(e.to_string()))?;
    let reply = client
        .predict_v2(crate::sidecar::PredictV2Request {
            model_id: name.into(),
            version: 1,
            model_kind: "transform".into(),
            artifact_uri: String::new(),
            artifact_hash: String::new(),
            rows: vec![crate::sidecar::V2Row {
                key: "dry-run".into(),
                features: sample_row(&parsed),
            }],
            transform: Some(serde_json::to_value(spec).unwrap_or_default()),
        })
        .await
        .map_err(|e| TransformError::DryRun(e.to_string()))?;
    let answer = reply
        .answers
        .first()
        .ok_or_else(|| TransformError::DryRun("no answer".into()))?;
    match crate::provider::outcome_of(answer, &parsed, name) {
        strategy_engine::ModelOutcome::Answered { outputs } => {
            if let Some((k, _)) = outputs
                .iter()
                .find(|(_, v)| matches!(v, domain::strategy_graph::Value::Missing(_)))
            {
                return Err(TransformError::DryRun(format!("it gave no `{k}`")));
            }
            Ok(parsed)
        }
        strategy_engine::ModelOutcome::Abstained { detail }
        | strategy_engine::ModelOutcome::Unavailable { detail } => {
            Err(TransformError::DryRun(detail))
        }
        strategy_engine::ModelOutcome::TimedOut { .. } => {
            Err(TransformError::DryRun("timed out".into()))
        }
    }
}

/// Register a transform: dry-run it against its declared contract, then record
/// the model and version 1 with that contract. No training, no artifact, no
/// `trained_through` — it answers any decision, in a backtest as on paper.
///
/// # Errors
/// As [`dry_run_transform`], or a database error.
pub async fn register_transform(
    pg: &sqlx::PgPool,
    client: &dyn crate::provider::V2Client,
    owner: uuid::Uuid,
    name: &str,
    asset_class: &str,
    contract: &serde_json::Value,
    spec: &domain::model_def::transform::TransformSpec,
) -> Result<(), TransformError> {
    let parsed = dry_run_transform(client, name, contract, spec).await?;
    let definition = serde_json::json!({
        "schema_version": "1.1",
        "model_kind": "transform",
        "framework": "custom",
        "asset_class": asset_class,
        "transform": spec,
    });
    let mut tx = pg.begin().await?;
    sqlx::query(
        "INSERT INTO ai_models (model_id, slug, display_name, model_kind, asset_class, definition_json, created_by) \
         VALUES ($1, $1, $1, 'transform', $2, $3, $4)",
    )
    .bind(name)
    .bind(asset_class)
    .bind(&definition)
    .bind(owner)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO model_versions (model_id, version, status, config_json, created_by, io_contract, contract_source) \
         VALUES ($1, 1, 'ready', $2, $3, $4, 'declared')",
    )
    .bind(name)
    .bind(serde_json::json!({"transform": true}))
    .bind(owner)
    .bind(serde_json::to_value(&parsed).unwrap_or_default())
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn forecaster() -> serde_json::Value {
        serde_json::json!({
            "schema_version": "1.1",
            "model_kind": "forecaster",
            "framework": "lightgbm",
            "asset_class": "crypto_spot_cex",
            "target": {"field": "return", "horizon": "1h"},
            "feature_set_ref": "core_v1"
        })
    }

    #[test]
    fn a_forecaster_gets_the_contract_its_serving_path_honours() {
        let c = derive_for(&forecaster(), None).expect("derived");
        assert!(
            c.output("direction").is_some() || c.output("median_return").is_some(),
            "{c:?}"
        );
        assert_eq!(
            c.pit.embargo.as_deref(),
            Some("1h"),
            "the label horizon is the embargo"
        );
    }

    #[test]
    fn the_training_feature_order_wins_when_the_trainer_reported_it() {
        let metrics = serde_json::json!({"feature_order": ["rsi_14", "atr_14"]});
        let c = derive_for(&forecaster(), Some(&metrics)).unwrap();
        let shown = serde_json::to_string(&c).unwrap();
        assert!(
            shown.contains("rsi_14") && shown.contains("atr_14"),
            "{shown}"
        );
    }

    #[test]
    fn a_transform_must_declare_its_own() {
        let mut def = forecaster();
        def["model_kind"] = serde_json::json!("transform");
        let err = derive_for(&def, None).unwrap_err();
        assert!(err.contains("declares its own contract"), "{err}");
    }
}

/// Live-Postgres: the backfill gives every derivable version a contract.
///
/// ```text
/// DATABASE_URL=postgres://… cargo test -j 2 -p model-registry --lib contracts -- --ignored
/// ```
#[cfg(test)]
mod pg_tests {
    #[tokio::test]
    #[ignore = "needs DATABASE_URL"]
    async fn after_the_backfill_every_derivable_version_has_a_contract() {
        let pg = sqlx::PgPool::connect(&std::env::var("DATABASE_URL").unwrap())
            .await
            .unwrap();
        let id = format!("m-{}", uuid::Uuid::new_v4());
        let def = serde_json::json!({
            "schema_version": "1.1", "model_kind": "forecaster", "framework": "lightgbm",
            "asset_class": "crypto_spot_cex", "target": {"field": "return", "horizon": "1h"},
            "feature_set_ref": "core_v1"
        });
        let mut transform = def.clone();
        transform["model_kind"] = serde_json::json!("transform");
        for (mid, d) in [(id.clone(), def), (format!("{id}-t"), transform)] {
            sqlx::query("INSERT INTO ai_models (model_id, slug, display_name, model_kind, asset_class, definition_json, created_by) VALUES ($1,$1,$1,'forecaster','crypto_spot_cex',$2,gen_random_uuid())")
                .bind(&mid).bind(&d).execute(&pg).await.unwrap();
            for v in [1, 2] {
                sqlx::query("INSERT INTO model_versions (model_id, version, config_json, created_by) VALUES ($1,$2,'{}',gen_random_uuid())")
                    .bind(&mid).bind(v).execute(&pg).await.unwrap();
            }
        }
        let report = super::backfill(&pg).await.unwrap();
        assert!(report.derived >= 2);
        assert!(report
            .left
            .iter()
            .any(|(m, _, why)| m == &format!("{id}-t") && why.contains("declares its own")));
        let snapshot = backtest::models::RegistrySnapshot::load(&pg).await.unwrap();
        use strategy_engine::Registries;
        assert!(
            snapshot.model(&format!("{id}@2")).is_some(),
            "the compiler resolves the pinned version"
        );
        assert!(snapshot.model_without_contract(&format!("{id}-t@1")));
        // Idempotent, and a declared contract is never overwritten by a derived one.
        let again = super::backfill(&pg).await.unwrap();
        assert_eq!(again.derived, 0);
    }

    /// Registration against a running sidecar and Postgres: dry-run, record,
    /// then a strategy compiles against it.
    #[tokio::test]
    #[ignore = "needs DATABASE_URL and a running inference sidecar (MODEL_INFERENCE_URL)"]
    async fn a_transform_is_registered_typechecked_and_callable() {
        let pg = sqlx::PgPool::connect(&std::env::var("DATABASE_URL").unwrap())
            .await
            .unwrap();
        let client = crate::sidecar::SidecarClient::from_env();
        let name = format!("spread-{}", uuid::Uuid::new_v4());
        let contract = serde_json::json!({
            "inputs": [{"name": "a", "shape": "scalar", "value_type": {"type": "num"}}],
            "outputs": [{"name": "z", "value_type": {"type": "num"}}],
            "timeout_ms": 500
        });
        let good = domain::model_def::transform::TransformSpec {
            language: "python".into(),
            entrypoint: "f".into(),
            source: "def f(x):
    return {'z': x['a'] * 2}
"
            .into(),
        };
        super::register_transform(
            &pg,
            &client,
            uuid::Uuid::new_v4(),
            &name,
            "crypto_spot_cex",
            &contract,
            &good,
        )
        .await
        .expect("registered");
        // Code whose answer does not match its contract is refused at the door.
        let wrong = domain::model_def::transform::TransformSpec {
            source: "def f(x):
    return {'y': 1.0}
"
            .into(),
            ..good.clone()
        };
        let err = super::register_transform(
            &pg,
            &client,
            uuid::Uuid::new_v4(),
            &format!("{name}-bad"),
            "crypto_spot_cex",
            &contract,
            &wrong,
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("`z`"), "{err}");

        let snapshot = backtest::models::RegistrySnapshot::load(&pg).await.unwrap();
        let entry = snapshot.pinned(&name, 1).expect("in the registry");
        assert_eq!(entry.kind, "transform");
        assert!(entry.transform.is_some() && entry.contract.is_some());
        let graph: domain::strategy_graph::StrategyGraph =
            serde_json::from_value(serde_json::json!({
              "format": "tbot.strategy/3", "kind": "trading", "name": "uses_transform",
              "asset_class": "crypto_spot_cex", "timeframe": "1h",
              "nodes": [
                {"id":"m","op":"model","args":{"ref":{"str": format!("{name}@1")},"a":"close"}},
                {"id":"go","op":"gt","args":{"a":"m.z","b":0}}
              ],
              "decide": {"rules": [{"when":"go","then":{"kind":"hold"}}]}
            }))
            .unwrap();
        strategy_engine::compile(
            &graph,
            &snapshot,
            &domain::strategy_def::params::ParamValues::new(),
        )
        .expect("a strategy compiles against the registered transform");
    }
}
