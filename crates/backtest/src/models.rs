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
            });
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
    fn put(&self, key: CacheKey, outcome: ModelOutcome);
}

/// An in-process cache.
#[derive(Debug, Default)]
pub struct MemoryCache(std::sync::Mutex<BTreeMap<CacheKey, ModelOutcome>>);

impl PredictionCache for MemoryCache {
    fn get(&self, key: &CacheKey) -> Option<ModelOutcome> {
        self.0.lock().ok()?.get(key).cloned()
    }
    fn put(&self, key: CacheKey, outcome: ModelOutcome) {
        if let Ok(mut m) = self.0.lock() {
            m.insert(key, outcome);
        }
    }
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
    answered: BTreeMap<(String, u32), u64>,
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
        let consumption = ModelConsumption {
            at_ns: req.at_ns,
            model: req.model.clone(),
            version: req.version,
            trained_through_ns: entry.trained_through.and_then(|t| t.timestamp_nanos_opt()),
            embargo_ns,
        };
        if !self.optimistic && !consumption.point_in_time() {
            let trained = entry
                .trained_through
                .map_or_else(|| "an unknown instant".to_string(), |t| t.to_rfc3339());
            return ModelOutcome::Unavailable {
                detail: format!(
                    "{}@{} was trained through {trained} (embargo {}s): no fold trained early \
                     enough to answer at {}",
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
            let fresh = self.source.predict(&entry, req);
            // Only deterministic answers are cached: a timeout or an outage
            // is a fact about that call, not about the model.
            if matches!(
                fresh,
                ModelOutcome::Answered { .. } | ModelOutcome::Abstained { .. }
            ) {
                self.cache.put(key, fresh.clone());
            }
            fresh
        };
        if matches!(outcome, ModelOutcome::Answered { .. }) {
            *self
                .answered
                .entry((req.model.clone(), req.version))
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
            .map(|((model, version), n)| crate::run::FoldUse {
                model: model.clone(),
                version: *version,
                fold: self
                    .snapshot
                    .pinned(model, *version)
                    .and_then(|e| e.trained_through)
                    .map_or_else(
                        || "trained_through unknown".into(),
                        |t| format!("trained through {}", t.to_rfc3339()),
                    ),
                answered: *n,
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
