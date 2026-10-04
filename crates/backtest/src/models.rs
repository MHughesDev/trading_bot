//! The model registry as the strategy compiler and the model providers read it
//! (Set U U-5.1; ADR-0043 decisions 1 and 4).
//!
//! A [`RegistrySnapshot`] is every model version with its contract, artifact and
//! `trained_through`, plus every alias, read once. Compiling against it resolves
//! `name@version` exactly and `name@alias` to the version the alias names *at
//! compile time* — after that the plan holds a version, so moving the alias
//! later does not change what a running strategy calls.
//!
//! It lives here, not in `model-registry`, because `model-registry` depends on
//! this crate; everything that compiles a strategy can reach it.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use domain::model_def::contract::IoContract;
use strategy_engine::env::ModelProvider;
use strategy_engine::{ModelOutcome, ModelRequest, Registries, ResolvedModel};

/// One model version.
#[derive(Clone, Debug, PartialEq)]
pub struct VersionEntry {
    pub model: String,
    pub version: u32,
    pub kind: String,
    /// `None` when the version declares no contract (and none could be derived).
    pub contract: Option<IoContract>,
    /// The last instant its training data reached; `None` when unknown.
    pub trained_through: Option<DateTime<Utc>>,
    pub artifact_uri: Option<String>,
    pub artifact_sha256: Option<String>,
    /// An LLM adapter's settings (provider, model, endpoint, cost), when the
    /// version is an external LLM adapter.
    pub adapter: Option<serde_json::Value>,
    /// A transform's code (`{language, entrypoint, source}`), when the version
    /// is a transform: no training, so the point-in-time rule has nothing to
    /// hold it to and it answers any decision.
    pub transform: Option<serde_json::Value>,
    /// The walk-forward folds' own models (Set U U-5.8), earliest training end
    /// first. Empty for a version trained once. A backtest decision is answered
    /// by the latest of these — or the version's own artifact — trained early
    /// enough; live serving always uses the version's own artifact.
    pub folds: Vec<FoldArtifact>,
}

/// One walk-forward fold's model: trained on the data before that fold's test
/// window, so it may answer the decisions inside it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FoldArtifact {
    pub index: u32,
    pub trained_through: DateTime<Utc>,
    pub artifact_uri: String,
    pub artifact_sha256: String,
}

impl VersionEntry {
    /// The model that may answer a decision at `at_ns`: the candidate (the
    /// folds and the version's own artifact) with the latest training end
    /// that is still before `at_ns − embargo`. `None` when none is; the
    /// version's own artifact with an unknown training end is never chosen.
    #[must_use]
    pub fn answerer_at(&self, at_ns: i64, embargo_ns: i64) -> Option<Answerer> {
        let fits = |through: i64| through.saturating_add(embargo_ns) < at_ns;
        let own = self
            .trained_through
            .and_then(|t| t.timestamp_nanos_opt())
            .filter(|t| fits(*t))
            .map(|t| (t, None));
        self.folds
            .iter()
            .filter_map(|f| {
                let t = f.trained_through.timestamp_nanos_opt()?;
                fits(t).then_some((t, Some(f)))
            })
            .chain(own)
            // The latest training end; on a tie the version's own artifact.
            .max_by_key(|(t, f)| (*t, f.is_none()))
            .map(|(t, f)| Answerer {
                trained_through_ns: t,
                fold: f.cloned(),
            })
    }

    /// This entry as the given fold's model: its artifact and training end.
    #[must_use]
    pub fn as_fold(&self, fold: &FoldArtifact) -> Self {
        Self {
            trained_through: Some(fold.trained_through),
            artifact_uri: Some(fold.artifact_uri.clone()),
            artifact_sha256: Some(fold.artifact_sha256.clone()),
            folds: Vec::new(),
            ..self.clone()
        }
    }
}

/// Which of a version's models answers one decision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Answerer {
    pub trained_through_ns: i64,
    /// `None`: the version's own artifact.
    pub fold: Option<FoldArtifact>,
}

/// Every version and alias, read once.
#[derive(Clone, Debug, Default)]
pub struct RegistrySnapshot {
    /// `model@version` → entry.
    versions: BTreeMap<String, VersionEntry>,
    /// `model@alias` → version.
    aliases: BTreeMap<String, u32>,
}

impl RegistrySnapshot {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a version (tests and loaders).
    #[must_use]
    pub fn with_version(mut self, e: VersionEntry) -> Self {
        self.versions
            .insert(format!("{}@{}", e.model, e.version), e);
        self
    }

    /// Point `model@alias` at a version.
    #[must_use]
    pub fn with_alias(mut self, model: &str, alias: &str, version: u32) -> Self {
        self.aliases.insert(format!("{model}@{alias}"), version);
        self
    }

    /// Read the registry from Postgres. A stored contract that no longer parses
    /// is treated as absent — refused by the compiler with its fix — never as a
    /// different contract.
    ///
    /// # Errors
    /// A database error.
    pub async fn load(pg: &sqlx::PgPool) -> Result<Self, storage::postgres::PgError> {
        let mut snap = Self::new();
        for row in storage::postgres::models::version_contracts(pg).await? {
            let Ok(version) = u32::try_from(row.version) else {
                continue;
            };
            let contract = row
                .io_contract
                .and_then(|v| serde_json::from_value::<IoContract>(v).ok());
            snap = snap.with_version(VersionEntry {
                model: row.model_id,
                version,
                kind: row.model_kind,
                contract,
                trained_through: row.trained_through,
                artifact_uri: row.artifact_uri,
                artifact_sha256: row.artifact_sha256,
                adapter: row.adapter.filter(|a| !a.is_null()),
                transform: row.transform.filter(|t| !t.is_null()),
                folds: Vec::new(),
            });
        }
        for row in storage::postgres::models::fold_artifacts(pg).await? {
            let (Ok(version), Ok(index)) =
                (u32::try_from(row.version), u32::try_from(row.fold_index))
            else {
                continue;
            };
            if let Some(e) = snap
                .versions
                .get_mut(&format!("{}@{version}", row.model_id))
            {
                e.folds.push(FoldArtifact {
                    index,
                    trained_through: row.trained_through,
                    artifact_uri: row.storage_uri,
                    artifact_sha256: row.sha256,
                });
            }
        }
        for e in snap.versions.values_mut() {
            e.folds.sort_by_key(|f| (f.trained_through, f.index));
        }
        for (model, alias, version) in storage::postgres::models::model_aliases(pg).await? {
            if let Ok(v) = u32::try_from(version) {
                snap = snap.with_alias(&model, &alias, v);
            }
        }
        Ok(snap)
    }

    /// [`Self::load`], or an empty registry when the database cannot be read —
    /// said in the log, and harmless: a strategy that calls a model then fails
    /// to compile naming the model, and one that calls none is unaffected.
    pub async fn load_or_empty(pg: &sqlx::PgPool) -> Self {
        match Self::load(pg).await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(error = %e, "model registry unreadable; compiling with no models");
                Self::new()
            }
        }
    }

    /// The version a reference names, resolving an alias.
    #[must_use]
    pub fn entry(&self, reference: &str) -> Option<&VersionEntry> {
        let (model, tag) = reference.rsplit_once('@')?;
        let version = match tag.parse::<u32>() {
            Ok(v) => v,
            Err(_) => *self.aliases.get(reference)?,
        };
        self.versions.get(&format!("{model}@{version}"))
    }

    /// Every version the snapshot holds, by reference.
    pub fn entries(&self) -> impl Iterator<Item = &VersionEntry> {
        self.versions.values()
    }

    /// A pinned version's entry.
    #[must_use]
    pub fn pinned(&self, model: &str, version: u32) -> Option<&VersionEntry> {
        self.versions.get(&format!("{model}@{version}"))
    }
}

// ── point-in-time model answers (Set U U-5.4) ────────────────────────────────

/// Where an answer comes from when the cache has none: the inference sidecar
/// in production, a fake in tests. Called only for requests the point-in-time
/// rule allows.
pub trait PredictionSource: Send {
    fn predict(&mut self, entry: &VersionEntry, req: &ModelRequest) -> ModelOutcome;
}

/// A source with nothing behind it: every request is unavailable.
#[derive(Debug, Default)]
pub struct NoSource;

impl PredictionSource for NoSource {
    fn predict(&mut self, entry: &VersionEntry, _req: &ModelRequest) -> ModelOutcome {
        ModelOutcome::Unavailable {
            detail: format!(
                "no model serving configured for {}@{}",
                entry.model, entry.version
            ),
        }
    }
}

/// The key a prediction is cached under: the pinned version, the decision
/// instant, and a digest of exactly what was asked.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CacheKey {
    pub model: String,
    pub version: u32,
    pub at_ns: i64,
    pub inputs_digest: String,
}

impl CacheKey {
    #[must_use]
    pub fn of(req: &ModelRequest) -> Self {
        use sha2::Digest;
        let inputs = serde_json::to_vec(&req.inputs).unwrap_or_default();
        Self {
            model: req.model.clone(),
            version: req.version,
            at_ns: req.at_ns,
            inputs_digest: hex::encode(sha2::Sha256::digest(&inputs)),
        }
    }
}

/// A store of answers already computed — the prediction series a repeated run
/// reads instead of calling the sidecar again.
pub trait PredictionCache: Send + Sync {
    fn get(&self, key: &CacheKey) -> Option<ModelOutcome>;
    /// Keep an answer, with the training end of the version that gave it — so
    /// a stored series says which model answered, not only what it answered.
    fn put(&self, key: CacheKey, outcome: ModelOutcome, trained_through_ns: Option<i64>);
}

/// An in-process cache.
#[derive(Debug, Default)]
pub struct MemoryCache(std::sync::Mutex<BTreeMap<CacheKey, ModelOutcome>>);

impl MemoryCache {
    /// Every key held, in order.
    #[must_use]
    pub fn keys(&self) -> Vec<CacheKey> {
        self.0
            .lock()
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_default()
    }
}

impl PredictionCache for MemoryCache {
    fn get(&self, key: &CacheKey) -> Option<ModelOutcome> {
        self.0.lock().ok()?.get(key).cloned()
    }
    fn put(&self, key: CacheKey, outcome: ModelOutcome, _trained_through_ns: Option<i64>) {
        if let Ok(mut m) = self.0.lock() {
            m.insert(key, outcome);
        }
    }
}

/// The prediction series in ClickHouse (`model_prediction_series`, U-5.5) as
/// a cache: read before the sidecar is called, written after.
pub struct ClickHouseCache {
    client: clickhouse::Client,
    handle: tokio::runtime::Handle,
}

#[derive(clickhouse::Row, serde::Serialize, serde::Deserialize)]
struct SeriesRow {
    model_id: String,
    version: u32,
    at_ns: i64,
    inputs_digest: String,
    status: String,
    outcome: String,
    trained_through_ns: Option<i64>,
}

impl ClickHouseCache {
    #[must_use]
    pub fn new(url: &str, handle: tokio::runtime::Handle) -> Self {
        Self {
            client: storage::clickhouse::connect(url),
            handle,
        }
    }

    fn block<F: std::future::Future>(&self, f: F) -> F::Output {
        tokio::task::block_in_place(|| self.handle.block_on(f))
    }
}

impl PredictionCache for ClickHouseCache {
    fn get(&self, key: &CacheKey) -> Option<ModelOutcome> {
        #[derive(clickhouse::Row, serde::Deserialize)]
        struct Hit {
            outcome: String,
        }
        let hit: Option<Hit> = self
            .block(
                self.client
                    .query(
                        "SELECT outcome FROM model_prediction_series FINAL \
                         WHERE model_id = ? AND version = ? AND at_ns = ? AND inputs_digest = ? LIMIT 1",
                    )
                    .bind(&key.model)
                    .bind(key.version)
                    .bind(key.at_ns)
                    .bind(&key.inputs_digest)
                    .fetch_optional(),
            )
            .ok()
            .flatten();
        hit.and_then(|h| serde_json::from_str(&h.outcome).ok())
    }

    fn put(&self, key: CacheKey, outcome: ModelOutcome, trained_through_ns: Option<i64>) {
        let row = SeriesRow {
            status: match &outcome {
                ModelOutcome::Answered { .. } => "answered",
                ModelOutcome::Abstained { .. } => "abstained",
                ModelOutcome::TimedOut { .. } => "timed_out",
                ModelOutcome::Unavailable { .. } => "unavailable",
            }
            .into(),
            outcome: serde_json::to_string(&outcome).unwrap_or_default(),
            model_id: key.model,
            version: key.version,
            at_ns: key.at_ns,
            inputs_digest: key.inputs_digest,
            trained_through_ns,
        };
        let written: Result<(), clickhouse::error::Error> = self.block(async {
            let mut insert = self.client.insert("model_prediction_series")?;
            insert.write(&row).await?;
            insert.end().await
        });
        if let Err(e) = written {
            // A cache that cannot be written is a slower run, not a wrong one.
            tracing::warn!(error = %e, "prediction series write failed");
        }
    }
}

/// The graph a `predict_series` job runs: the model node and nothing that
/// trades, so every bar asks the model exactly what a strategy calling it would
/// ask (the same assembled inputs, so the same cache key).
///
/// # Errors
/// A reference whose contract has an input a graph cannot wire without a
/// strategy (a scalar, text), named.
pub fn series_graph(
    reference: &str,
    contract: &domain::model_def::contract::IoContract,
    asset_class: &str,
    timeframe: &str,
) -> Result<domain::strategy_graph::StrategyGraph, String> {
    use domain::model_def::contract::InputShape;
    for ci in &contract.inputs {
        if !matches!(ci.shape, InputShape::Row { .. } | InputShape::Window { .. }) && !ci.optional {
            return Err(format!(
                "input `{}` needs a value a strategy wires in; a series cannot be computed for it alone",
                ci.name
            ));
        }
    }
    serde_json::from_value(serde_json::json!({
      "format": domain::strategy_graph::FORMAT, "kind": "trading", "name": format!("series:{reference}"),
      "asset_class": asset_class, "timeframe": timeframe,
      "nodes": [
        {"id": "m", "op": "model", "args": {"ref": {"str": reference}}},
        {"id": "always", "op": "ge", "args": {"a": "close", "b": 0}}
      ],
      "decide": {"rules": [{"when": "always", "then": {"kind": "hold"}}]}
    }))
    .map_err(|e| e.to_string())
}

/// What a prediction-series run wrote.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SeriesReport {
    /// Decisions the version answered (each now in the series).
    pub answered: u64,
    /// Decisions it could not answer point-in-time — before its training end
    /// plus embargo — and that are therefore not in the series.
    pub before_training_end: u64,
    /// Decisions it abstained on or could not reach.
    pub other: u64,
    pub first_ns: Option<i64>,
    pub last_ns: Option<i64>,
}

/// One answer a run consumed, as Gate 0 judges it.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ModelConsumption {
    pub at_ns: i64,
    pub model: String,
    pub version: u32,
    pub trained_through_ns: Option<i64>,
    pub embargo_ns: i64,
}

impl ModelConsumption {
    /// The answer came from a version trained strictly before the decision
    /// minus its embargo.
    #[must_use]
    pub fn point_in_time(&self) -> bool {
        self.trained_through_ns
            .is_some_and(|t| self.at_ns > t.saturating_add(self.embargo_ns))
    }
}

/// A horizon token (`30s 1m 15m 1h 4h 1d`) in nanoseconds.
///
/// # Errors
/// A token that is not a positive count of a known unit.
pub fn horizon_ns(token: &str) -> Result<i64, String> {
    let t = token.trim();
    let split = t.find(|c: char| !c.is_ascii_digit()).unwrap_or(t.len());
    let (n, unit) = t.split_at(split);
    let n: i64 = n
        .parse()
        .map_err(|_| format!("'{token}' is not a horizon"))?;
    let secs = match unit {
        "s" => 1,
        "m" => 60,
        "h" => 3_600,
        "d" => 86_400,
        "w" => 604_800,
        _ => return Err(format!("'{token}' has an unknown unit")),
    };
    Ok(n * secs * 1_000_000_000)
}

/// What a run's model provider reports after the run.
pub trait RunModels: ModelProvider + Send {
    /// Every answer consumed — Gate 0's input.
    fn consumption(&self) -> &[ModelConsumption];
    /// The run declared `optimistic_models`.
    fn optimistic(&self) -> bool;
    /// Answers per pinned version — the manifest's fold usage.
    fn fold_uses(&self) -> Vec<crate::run::FoldUse>;
    /// Calls made to the source (not answered from the cache).
    fn source_calls(&self) -> u64;
}

/// The backtest's model provider: the point-in-time rule, a cache, a source.
///
/// For a request at `t` a version may answer only when it was trained through
/// an instant before `t − embargo` (the embargo is the contract's
/// `pit.embargo`, normally the label horizon). Otherwise — including when its
/// training end is unknown — it is `unavailable`, the strategy holds, and the
/// run's manifest counts it. `optimistic_models` lifts the rule for a declared
/// comparison run, which is `unsafe` and fails Gate 0.
pub struct PitModels<S: PredictionSource> {
    snapshot: std::sync::Arc<RegistrySnapshot>,
    source: S,
    cache: std::sync::Arc<dyn PredictionCache>,
    optimistic: bool,
    consumed: Vec<ModelConsumption>,
    calls: u64,
    /// Answers per (model, version, fold) — `None` is the version's own
    /// artifact.
    answered: BTreeMap<(String, u32, Option<u32>), u64>,
}

impl<S: PredictionSource> PitModels<S> {
    #[must_use]
    pub fn new(
        snapshot: std::sync::Arc<RegistrySnapshot>,
        source: S,
        cache: std::sync::Arc<dyn PredictionCache>,
        optimistic: bool,
    ) -> Self {
        Self {
            snapshot,
            source,
            cache,
            optimistic,
            consumed: Vec::new(),
            calls: 0,
            answered: BTreeMap::new(),
        }
    }
}

impl<S: PredictionSource> ModelProvider for PitModels<S> {
    fn answer(&mut self, req: &ModelRequest) -> ModelOutcome {
        let Some(entry) = self.snapshot.pinned(&req.model, req.version).cloned() else {
            return ModelOutcome::Unavailable {
                detail: format!("{}@{} is not in the registry", req.model, req.version),
            };
        };
        let embargo_ns = match entry
            .contract
            .as_ref()
            .and_then(|c| c.pit.embargo.as_deref())
            .map(horizon_ns)
            .transpose()
        {
            Ok(e) => e.unwrap_or(0),
            Err(why) => {
                return ModelOutcome::Unavailable {
                    detail: format!("{}@{}: embargo {why}", req.model, req.version),
                }
            }
        };
        // Which model answers: a transform trained on nothing, so it is in the
        // past of every decision; otherwise the latest of the version's own
        // artifact and its folds' (U-5.8) trained before `at − embargo`.
        let (trained_through_ns, answerer, fold) = if entry.kind == "transform" {
            (Some(i64::MIN), entry.clone(), None)
        } else if let Some(a) = entry.answerer_at(req.at_ns, embargo_ns) {
            match &a.fold {
                Some(f) => (Some(a.trained_through_ns), entry.as_fold(f), Some(f.index)),
                None => (Some(a.trained_through_ns), entry.clone(), None),
            }
        } else {
            (
                entry.trained_through.and_then(|t| t.timestamp_nanos_opt()),
                entry.clone(),
                None,
            )
        };
        let consumption = ModelConsumption {
            at_ns: req.at_ns,
            model: req.model.clone(),
            version: req.version,
            trained_through_ns,
            embargo_ns,
        };
        if !self.optimistic && !consumption.point_in_time() {
            let trained = entry
                .trained_through
                .map_or_else(|| "an unknown instant".to_string(), |t| t.to_rfc3339());
            let folds = match entry.folds.first() {
                Some(f) => format!(
                    "; its earliest of {} folds through {}",
                    entry.folds.len(),
                    f.trained_through.to_rfc3339()
                ),
                None => String::new(),
            };
            return ModelOutcome::Unavailable {
                detail: format!(
                    "{}@{} was trained through {trained}{folds} (embargo {}s): no fold trained                      early enough to answer at {}",
                    req.model,
                    req.version,
                    embargo_ns / 1_000_000_000,
                    chrono::DateTime::from_timestamp_nanos(req.at_ns).to_rfc3339()
                ),
            };
        }
        let key = CacheKey::of(req);
        let outcome = if let Some(hit) = self.cache.get(&key) {
            hit
        } else {
            self.calls += 1;
            let fresh = self.source.predict(&answerer, req);
            // Only deterministic answers are cached: a timeout or an outage
            // is a fact about that call, not about the model.
            if matches!(
                fresh,
                ModelOutcome::Answered { .. } | ModelOutcome::Abstained { .. }
            ) {
                self.cache
                    .put(key, fresh.clone(), consumption.trained_through_ns);
            }
            fresh
        };
        if matches!(outcome, ModelOutcome::Answered { .. }) {
            *self
                .answered
                .entry((req.model.clone(), req.version, fold))
                .or_insert(0) += 1;
            self.consumed.push(consumption);
        }
        outcome
    }
}

impl<S: PredictionSource> RunModels for PitModels<S> {
    fn consumption(&self) -> &[ModelConsumption] {
        &self.consumed
    }
    fn optimistic(&self) -> bool {
        self.optimistic
    }
    fn fold_uses(&self) -> Vec<crate::run::FoldUse> {
        self.answered
            .iter()
            .map(|((model, version, fold), n)| {
                let entry = self.snapshot.pinned(model, *version);
                let through = |t: Option<DateTime<Utc>>| {
                    t.map_or_else(
                        || "trained_through unknown".into(),
                        |t| format!("trained through {}", t.to_rfc3339()),
                    )
                };
                let label = match fold {
                    None => through(entry.and_then(|e| e.trained_through)),
                    Some(i) => format!(
                        "fold {i}, {}",
                        through(entry.and_then(|e| {
                            e.folds
                                .iter()
                                .find(|f| f.index == *i)
                                .map(|f| f.trained_through)
                        }))
                    ),
                };
                crate::run::FoldUse {
                    model: model.clone(),
                    version: *version,
                    fold: label,
                    answered: *n,
                }
            })
            .collect()
    }
    fn source_calls(&self) -> u64 {
        self.calls
    }
}

impl Registries for RegistrySnapshot {
    fn feature_lookback(&self, name: &str) -> Option<u32> {
        features::runtime::lookback_bars(name)
    }

    fn feature_set(&self, name: &str) -> Option<Vec<String>> {
        features::feature_sets::resolve(name).map(|s| s.features.clone())
    }

    fn model(&self, reference: &str) -> Option<ResolvedModel> {
        let e = self.entry(reference)?;
        Some(ResolvedModel {
            model: e.model.clone(),
            version: e.version,
            contract: e.contract.clone()?,
        })
    }

    fn model_without_contract(&self, reference: &str) -> bool {
        self.entry(reference).is_some_and(|e| e.contract.is_none())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::model_def::contract::{ContractOutput, CostClass, PitSpec};
    use domain::strategy_def::params::ParamValues;
    use domain::strategy_graph::ValueType;

    fn contract() -> IoContract {
        IoContract {
            inputs: vec![],
            outputs: vec![ContractOutput {
                name: "f".into(),
                value_type: ValueType::Num,
            }],
            horizon: None,
            pit: PitSpec::default(),
            cost_class: CostClass::Cheap,
            timeout_ms: 100,
        }
    }

    fn entry(model: &str, version: u32, contract: Option<IoContract>) -> VersionEntry {
        VersionEntry {
            model: model.into(),
            version,
            kind: "forecaster".into(),
            contract,
            trained_through: None,
            artifact_uri: None,
            artifact_sha256: None,
            adapter: None,
            transform: None,
            folds: Vec::new(),
        }
    }

    fn graph(reference: &str) -> domain::strategy_graph::StrategyGraph {
        serde_json::from_value(serde_json::json!({
          "format": "tbot.strategy/3", "kind": "trading", "name": "m",
          "asset_class": "crypto_spot_cex", "timeframe": "1h",
          "nodes": [
            {"id":"m","op":"model","args":{"ref":{"str":reference}}},
            {"id":"go","op":"gt","args":{"a":"m.f","b":0}}
          ],
          "decide": {"rules": [{"when":"go","then":{"kind":"hold"}}]}
        }))
        .unwrap()
    }

    #[test]
    fn the_compiler_resolves_a_pinned_version_and_an_alias_to_its_version_now() {
        let reg = RegistrySnapshot::new()
            .with_version(entry("vol", 3, Some(contract())))
            .with_version(entry("vol", 4, Some(contract())))
            .with_alias("vol", "prod", 3);
        let plan = strategy_engine::compile(&graph("vol@4"), &reg, &ParamValues::new()).unwrap();
        assert!(plan.nodes.iter().any(
            |n| matches!(&n.kind, strategy_engine::plan::NodeKind::Model(c) if c.version == 4)
        ));
        let plan = strategy_engine::compile(&graph("vol@prod"), &reg, &ParamValues::new()).unwrap();
        assert!(
            plan.nodes.iter().any(
                |n| matches!(&n.kind, strategy_engine::plan::NodeKind::Model(c) if c.version == 3)
            ),
            "the alias is resolved to a version at compile time"
        );
    }

    #[test]
    fn the_answerer_is_the_latest_model_trained_before_the_embargo() {
        use chrono::TimeZone;
        let t = |h: u32| Utc.with_ymd_and_hms(2025, 1, 1, h, 0, 0).unwrap();
        let n = |h: u32| t(h).timestamp_nanos_opt().unwrap();
        let fold = |index: u32, h: u32| FoldArtifact {
            index,
            trained_through: t(h),
            artifact_uri: format!("f{index}"),
            artifact_sha256: String::new(),
        };
        let mut e = entry("m", 1, Some(contract()));
        e.trained_through = Some(t(10));
        e.folds = vec![fold(0, 2), fold(1, 6)];
        let hour = n(1) - n(0);
        let pick =
            |e: &VersionEntry, at: i64| e.answerer_at(at, hour).map(|a| a.fold.map(|f| f.index));
        assert_eq!(
            pick(&e, n(3)),
            None,
            "trained through 2h + 1h embargo is not before 3h"
        );
        assert_eq!(pick(&e, n(3) + 1), Some(Some(0)));
        assert_eq!(pick(&e, n(8)), Some(Some(1)));
        assert_eq!(
            pick(&e, n(12)),
            Some(None),
            "the version's own artifact once it may"
        );
        // An unknown training end never answers; the folds still do.
        e.trained_through = None;
        assert_eq!(pick(&e, n(20)), Some(Some(1)));
        // Same training end as a fold: the version's own artifact.
        e.trained_through = Some(t(6));
        assert_eq!(pick(&e, n(20)), Some(None));
        // A fold answers as that fold's model.
        let f = e.as_fold(&e.folds[0]);
        assert_eq!(f.artifact_uri.as_deref(), Some("f0"));
        assert_eq!(f.trained_through, Some(t(2)));
        assert!(f.folds.is_empty());
    }

    #[test]
    fn a_model_without_a_contract_is_refused_with_its_fix() {
        let reg = RegistrySnapshot::new().with_version(entry("custom", 1, None));
        let diags =
            strategy_engine::compile(&graph("custom@1"), &reg, &ParamValues::new()).unwrap_err();
        let d = diags
            .iter()
            .find(|d| d.code == "model_without_contract")
            .expect("named");
        assert!(d.to_string().contains("custom@1"));
        // And an unknown one is still unknown, not "contractless".
        let diags =
            strategy_engine::compile(&graph("nope@1"), &reg, &ParamValues::new()).unwrap_err();
        assert!(diags.iter().any(|d| d.code == "unknown_model"));
    }
}
