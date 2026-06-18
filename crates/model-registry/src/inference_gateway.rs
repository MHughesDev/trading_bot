//! Inference gateway — alias resolution, prediction caching, circuit breaking, and trace logging.

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use sqlx::PgPool;
use tracing::{debug, warn};

use domain::model::forecast::{ForecastDistribution, ForecastRisk};
use domain::strategy_def::InferenceOutput;
use rust_decimal::Decimal;

use crate::sidecar::{PredictInstance, PredictRequest, SidecarClient};

/// TTL for alias → version cache entries.
const ALIAS_CACHE_TTL: Duration = Duration::from_secs(30);
/// TTL for prediction cache entries.
const PRED_CACHE_TTL: Duration = Duration::from_secs(5);
/// Number of consecutive failures before circuit breaks open.
const CIRCUIT_BREAK_THRESHOLD: u32 = 5;

#[derive(Clone, Debug)]
pub struct ForecastResult {
    pub direction: String,
    pub confidence: f64,
    pub latency_ms: u64,
    /// Raw magnitude string from the sidecar response.
    ///
    /// For `RiskSizing` models this carries the decimal-string size fraction;
    /// for other model kinds it is the model's reported magnitude. Kept as a
    /// `String` so the size-fraction path stays decimal-exact (ADR-0002).
    pub magnitude: String,
    /// Full distribution from a probabilistic forecaster (ADR-0016).
    /// `None` for point/classification models.
    pub distribution: Option<ForecastDistribution>,
}

pub struct InferenceGateway {
    pg: PgPool,
    sidecar: Arc<SidecarClient>,
    /// (`model_id`, alias) → (version, `cached_at`)
    alias_cache: RwLock<HashMap<(String, String), (i32, Instant)>>,
    /// (`model_id`, version, `feature_hash`) → (result, `cached_at`)
    pred_cache: RwLock<HashMap<(String, i32, u64), (ForecastResult, Instant)>>,
    /// Per-model consecutive failure counts for circuit breaking.
    fail_counts: RwLock<HashMap<String, u32>>,
}

impl InferenceGateway {
    pub fn new(pg: PgPool, sidecar: Arc<SidecarClient>) -> Arc<Self> {
        Arc::new(Self {
            pg,
            sidecar,
            alias_cache: RwLock::new(HashMap::new()),
            pred_cache: RwLock::new(HashMap::new()),
            fail_counts: RwLock::new(HashMap::new()),
        })
    }

    /// Resolve alias → version from cache (TTL 30s) or Postgres.
    async fn resolve_alias(&self, model_id: &str, alias: &str) -> Option<i32> {
        let key = (model_id.to_string(), alias.to_string());

        // Check cache first.
        {
            let cache = self.alias_cache.read().unwrap();
            if let Some((version, cached_at)) = cache.get(&key) {
                if cached_at.elapsed() < ALIAS_CACHE_TTL {
                    return Some(*version);
                }
            }
        }

        // Cache miss — query Postgres.
        let row: Option<(i32,)> =
            sqlx::query_as("SELECT version FROM model_aliases WHERE model_id = $1 AND alias = $2")
                .bind(model_id)
                .bind(alias)
                .fetch_optional(&self.pg)
                .await
                .ok()
                .flatten();

        if let Some((version,)) = row {
            let mut cache = self.alias_cache.write().unwrap();
            cache.insert(key, (version, Instant::now()));
            return Some(version);
        }

        None
    }

    /// Hash features deterministically for cache key.
    fn hash_features(instrument_id: &str, features: &HashMap<String, f64>) -> u64 {
        let mut hasher = DefaultHasher::new();
        instrument_id.hash(&mut hasher);
        // Sort keys for determinism.
        let mut pairs: Vec<_> = features.iter().collect();
        pairs.sort_by_key(|(k, _)| k.as_str());
        for (k, v) in pairs {
            k.hash(&mut hasher);
            v.to_bits().hash(&mut hasher);
        }
        hasher.finish()
    }

    /// Full forecast: alias resolution → cache → sidecar → fallback → abstain.
    pub async fn forecast(
        &self,
        model_ref: &str,
        alias: &str,
        instrument_id: &str,
        features: &HashMap<String, f64>,
    ) -> Option<ForecastResult> {
        // Resolve model_id from slug or model_id.
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT model_id FROM ai_models WHERE slug = $1 OR model_id = $1 LIMIT 1",
        )
        .bind(model_ref)
        .fetch_optional(&self.pg)
        .await
        .ok()
        .flatten();

        let model_id = row.map(|(id,)| id)?;

        // Circuit break check — extract bool before any .await so the guard is
        // dropped and the future stays Send (RwLockReadGuard is !Send).
        let circuit_open = {
            let counts = self.fail_counts.read().unwrap();
            counts.get(&model_id).copied().unwrap_or(0) >= CIRCUIT_BREAK_THRESHOLD
        };
        if circuit_open {
            debug!("circuit break open for model {model_id}, abstaining");
            self.write_trace(&model_id, 0, instrument_id, 0, "abstain")
                .await;
            return None;
        }

        let effective_alias = if alias.is_empty() {
            "production"
        } else {
            alias
        };

        // Resolve alias → version.
        let version = if let Some(v) = self.resolve_alias(&model_id, effective_alias).await {
            v
        } else {
            warn!("alias '{effective_alias}' not found for model {model_id}");
            self.write_trace(&model_id, 0, instrument_id, 0, "abstain")
                .await;
            return None;
        };

        let feature_hash = Self::hash_features(instrument_id, features);
        let cache_key = (model_id.clone(), version, feature_hash);

        // Check prediction cache.
        {
            let cache = self.pred_cache.read().unwrap();
            if let Some((result, cached_at)) = cache.get(&cache_key) {
                if cached_at.elapsed() < PRED_CACHE_TTL {
                    debug!("pred cache hit for {model_id} v{version}");
                    return Some(result.clone());
                }
            }
        }

        // Cache miss — look up artifact info and model kind.
        let artifact_row: Option<(String, String, String)> = sqlx::query_as(
            "SELECT ma.storage_uri, ma.sha256, am.model_kind \
             FROM model_artifacts ma \
             JOIN ai_models am ON am.model_id = ma.model_id \
             WHERE ma.model_id = $1 AND ma.version = $2 AND ma.artifact_type = 'model' LIMIT 1",
        )
        .bind(&model_id)
        .bind(version)
        .fetch_optional(&self.pg)
        .await
        .ok()
        .flatten();

        let (artifact_uri, artifact_hash, model_kind) = if let Some(row) = artifact_row {
            row
        } else {
            warn!("artifact not found for model {model_id} v{version}");
            self.write_trace(&model_id, version, instrument_id, 0, "abstain")
                .await;
            return None;
        };

        let start = Instant::now();
        let predict_req = PredictRequest {
            model_id: model_id.clone(),
            version,
            model_kind: model_kind.clone(),
            artifact_uri: artifact_uri.clone(),
            artifact_hash: artifact_hash.clone(),
            instances: vec![PredictInstance {
                instrument_id: instrument_id.to_string(),
                features: features.clone(),
            }],
        };
        let predict_result = self.sidecar.predict(predict_req).await;
        let latency_ms = start.elapsed().as_millis() as u64;

        match predict_result {
            Ok(response) => {
                // Reset failure count on success.
                {
                    let mut counts = self.fail_counts.write().unwrap();
                    counts.remove(&model_id);
                }

                // Use first prediction from the response.
                let prediction = response.predictions.into_iter().next()?;

                // Reconstruct distribution from sidecar response and validate (I-1.12).
                let distribution = Self::build_distribution(&prediction);

                let result = ForecastResult {
                    direction: prediction.direction.clone(),
                    confidence: prediction.confidence,
                    latency_ms,
                    magnitude: prediction.magnitude.clone(),
                    distribution,
                };

                // Update prediction cache.
                {
                    let mut cache = self.pred_cache.write().unwrap();
                    cache.insert(cache_key, (result.clone(), Instant::now()));
                }

                self.write_trace(&model_id, version, instrument_id, latency_ms, "hit")
                    .await;
                Some(result)
            }
            Err(e) => {
                warn!("sidecar predict failed for {model_id}: {e}");

                // Increment failure count.
                {
                    let mut counts = self.fail_counts.write().unwrap();
                    *counts.entry(model_id.clone()).or_insert(0) += 1;
                }

                // Try fallback alias if we weren't already using it.
                if effective_alias != "fallback" {
                    if let Some(fallback_version) = self.resolve_alias(&model_id, "fallback").await
                    {
                        let fallback_artifact: Option<(String, String, String)> = sqlx::query_as(
                            "SELECT ma.storage_uri, ma.sha256, am.model_kind \
                             FROM model_artifacts ma \
                             JOIN ai_models am ON am.model_id = ma.model_id \
                             WHERE ma.model_id = $1 AND ma.version = $2 AND ma.artifact_type = 'model' LIMIT 1",
                        )
                        .bind(&model_id)
                        .bind(fallback_version)
                        .fetch_optional(&self.pg)
                        .await
                        .ok()
                        .flatten();

                        if let Some((fb_uri, fb_hash, fb_kind)) = fallback_artifact {
                            let fb_req = PredictRequest {
                                model_id: model_id.clone(),
                                version: fallback_version,
                                model_kind: fb_kind,
                                artifact_uri: fb_uri,
                                artifact_hash: fb_hash,
                                instances: vec![PredictInstance {
                                    instrument_id: instrument_id.to_string(),
                                    features: features.clone(),
                                }],
                            };
                            if let Ok(fb_resp) = self.sidecar.predict(fb_req).await {
                                if let Some(fb_pred) = fb_resp.predictions.into_iter().next() {
                                    let distribution = Self::build_distribution(&fb_pred);
                                    let result = ForecastResult {
                                        direction: fb_pred.direction.clone(),
                                        confidence: fb_pred.confidence,
                                        latency_ms,
                                        magnitude: fb_pred.magnitude.clone(),
                                        distribution,
                                    };
                                    self.write_trace(
                                        &model_id,
                                        fallback_version,
                                        instrument_id,
                                        latency_ms,
                                        "miss",
                                    )
                                    .await;
                                    return Some(result);
                                }
                            }
                        }
                    }
                }

                self.write_trace(&model_id, version, instrument_id, latency_ms, "error")
                    .await;
                None
            }
        }
    }

    /// Refresh model forecast results for a node list into a cache `HashMap`.
    /// Called by the strategy runtime layer before dispatch to pre-populate sync-readable results.
    ///
    /// All `ModelForecast` nodes resolve through `forecast()`.
    pub async fn refresh_node_forecasts(
        &self,
        nodes: &[domain::strategy_def::nodes::Node],
        instrument_id: &str,
        features: &HashMap<String, f64>,
        results: &mut HashMap<String, bool>,
    ) {
        use domain::strategy_def::nodes::NodeKind;

        for node in nodes {
            if let NodeKind::ModelForecast {
                model_ref,
                alias,
                direction,
                min_confidence,
                ..
            } = &node.kind
            {
                let forecast = self
                    .forecast(model_ref, alias, instrument_id, features)
                    .await;

                let fired = match forecast {
                    None => false,
                    Some(f) => {
                        let dir_match = direction == "any" || f.direction == *direction;
                        dir_match && f.confidence >= *min_confidence
                    }
                };

                results.insert(node.id.clone(), fired);
            }
        }
    }

    /// Build the rich `InferenceOutput` carrier from a raw `ForecastResult`.
    ///
    /// Distributional fields (median, sigma, quantiles) and derived risk
    /// read-outs (`var_95`, `es_95`, `skew`, `spread_90`) are populated only
    /// when the forecast carried a validated distribution.
    fn forecast_to_output(f: &ForecastResult) -> InferenceOutput {
        let mut out = InferenceOutput {
            direction: f.direction.clone(),
            confidence: f.confidence,
            ..Default::default()
        };
        if let Some(dist) = &f.distribution {
            out.median_return = Some(dist.median_return);
            out.sigma = Some(dist.sigma);
            out.quantile_levels = Some(dist.quantile_levels.clone());
            out.quantiles_return = Some(dist.quantiles_return.clone());
            let risk = ForecastRisk::from_distribution(dist);
            out.var_95 = Some(risk.var_95.var);
            out.var_99 = Some(risk.var_99.var);
            out.es_95 = Some(risk.var_95.es);
            out.skew = Some(risk.skew);
            out.spread_90 = Some(risk.spread_90);
        }
        out
    }

    /// Clamp a decimal-string size fraction into `[min, max]` (also decimal
    /// strings).  Comparison is done with `Decimal` to stay money-safe
    /// (ADR-0002) — no `f64` ever touches a size value.  Returns the original
    /// string unchanged if it (or a bound) fails to parse.
    fn clamp_fraction(raw: &str, clamp: Option<&[String; 2]>) -> String {
        let Some([min_s, max_s]) = clamp else {
            return raw.to_owned();
        };
        let (Ok(v), Ok(min), Ok(max)) = (
            Decimal::from_str_exact(raw),
            Decimal::from_str_exact(min_s),
            Decimal::from_str_exact(max_s),
        ) else {
            return raw.to_owned();
        };
        v.clamp(min, max).normalize().to_string()
    }

    /// Refresh full `InferenceOutput`s for value-producing AI nodes
    /// (`Inference`, `Sizing`, `Decision`) into `outputs`.
    ///
    /// The strategy runtime calls this before dispatch so it can write bound
    /// fields into feature slots and route `Sizing`/`Decision` results.
    ///
    /// Abstention honours each node's `AbstainPolicy`:
    ///   - `Flat`     → inserts a default (NaN/zero) `InferenceOutput`.
    ///   - `HoldLast` → leaves any prior value in `outputs` untouched.
    ///
    /// `Sizing` nodes additionally fall back to their `fallback` fixed size
    /// (when set) on abstention before consulting the policy.
    pub async fn refresh_node_inferences(
        &self,
        nodes: &[domain::strategy_def::nodes::Node],
        instrument_id: &str,
        features: &HashMap<String, f64>,
        outputs: &mut HashMap<String, InferenceOutput>,
    ) {
        use domain::strategy_def::nodes::NodeKind;

        for node in nodes {
            match &node.kind {
                NodeKind::Inference {
                    model_ref,
                    alias,
                    abstain,
                    ..
                } => {
                    match self
                        .forecast(model_ref, alias, instrument_id, features)
                        .await
                    {
                        Some(f) => {
                            outputs.insert(node.id.clone(), Self::forecast_to_output(&f));
                        }
                        None => Self::apply_abstain(&node.id, abstain, outputs),
                    }
                }
                NodeKind::Sizing {
                    model_ref,
                    alias,
                    clamp,
                    fallback,
                    abstain,
                    ..
                } => {
                    match self
                        .forecast(model_ref, alias, instrument_id, features)
                        .await
                    {
                        Some(f) => {
                            let mut out = Self::forecast_to_output(&f);
                            out.size_fraction =
                                Some(Self::clamp_fraction(&f.magnitude, clamp.as_ref()));
                            outputs.insert(node.id.clone(), out);
                        }
                        None => {
                            if let Some(fb) = fallback {
                                outputs.insert(
                                    node.id.clone(),
                                    InferenceOutput {
                                        size_fraction: Some(fb.clone()),
                                        ..Default::default()
                                    },
                                );
                            } else {
                                Self::apply_abstain(&node.id, abstain, outputs);
                            }
                        }
                    }
                }
                NodeKind::Decision {
                    model_ref,
                    alias,
                    abstain,
                    ..
                } => {
                    match self
                        .forecast(model_ref, alias, instrument_id, features)
                        .await
                    {
                        Some(f) => {
                            let mut out = Self::forecast_to_output(&f);
                            out.action_class = Some(f.direction.clone());
                            outputs.insert(node.id.clone(), out);
                        }
                        None => Self::apply_abstain(&node.id, abstain, outputs),
                    }
                }
                _ => {}
            }
        }
    }

    /// Apply the abstention policy for one node.
    fn apply_abstain(
        node_id: &str,
        abstain: &domain::strategy_def::nodes::AbstainPolicy,
        outputs: &mut HashMap<String, InferenceOutput>,
    ) {
        use domain::strategy_def::nodes::AbstainPolicy;
        match abstain {
            // Flat: publish a default (zero/NaN) output — safe, neutral values.
            AbstainPolicy::Flat => {
                outputs.insert(node_id.to_owned(), InferenceOutput::default());
            }
            // HoldLast: keep whatever was last published (do nothing).
            AbstainPolicy::HoldLast => {}
        }
    }

    /// Write a trace record to `model_events` table.
    async fn write_trace(
        &self,
        model_id: &str,
        version: i32,
        instrument_id: &str,
        latency_ms: u64,
        result: &str,
    ) {
        let payload = serde_json::json!({
            "version": version,
            "instrument": instrument_id,
            "latency_ms": latency_ms,
            "result": result,
        });

        let _ = sqlx::query(
            "INSERT INTO model_events (model_id, kind, payload, actor, created_at) \
             VALUES ($1, 'inference_trace', $2, '00000000-0000-0000-0000-000000000000', now())",
        )
        .bind(model_id)
        .bind(&payload)
        .execute(&self.pg)
        .await;
    }

    /// Reconstruct a `ForecastDistribution` from a sidecar `ForecastResponse` (I-1.12).
    /// Returns `None` when the response carries no distribution fields, or when the
    /// reconstructed distribution fails its invariant check (contract violation is
    /// logged as a warning; the point view still serves).
    fn build_distribution(pred: &crate::sidecar::ForecastResponse) -> Option<ForecastDistribution> {
        let levels = pred.quantile_levels.as_ref()?;
        let qr = pred.quantiles_return.as_ref()?;
        let median = pred.median_return?;
        let sigma = pred.sigma?;

        // σ-units: quantiles_return / sigma (guard zero sigma).
        let sigma_safe = if sigma > 0.0 { sigma } else { return None };
        let quantiles_sigma: Vec<f64> = qr.iter().map(|&v| v / sigma_safe).collect();

        let dist = ForecastDistribution {
            quantile_levels: levels.clone(),
            quantiles_sigma,
            quantiles_return: qr.clone(),
            median_return: median,
            sigma,
        };

        match dist.validate() {
            Ok(()) => Some(dist),
            Err(e) => {
                warn!("distribute contract violation from sidecar (discarding): {e}");
                None
            }
        }
    }

    /// Return recent inference traces from `model_events`.
    pub async fn get_traces(
        &self,
        model_id: &str,
        limit: i64,
    ) -> anyhow::Result<Vec<serde_json::Value>> {
        let rows: Vec<(serde_json::Value, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
            "SELECT payload, created_at FROM model_events \
             WHERE model_id = $1 AND kind = 'inference_trace' \
             ORDER BY created_at DESC LIMIT $2",
        )
        .bind(model_id)
        .bind(limit)
        .fetch_all(&self.pg)
        .await?;

        Ok(rows
            .into_iter()
            .map(|(mut payload, created_at)| {
                payload["recorded_at"] = serde_json::json!(created_at);
                payload
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result_with_dist() -> ForecastResult {
        ForecastResult {
            direction: "up".into(),
            confidence: 0.8,
            latency_ms: 1,
            magnitude: "0.04".into(),
            distribution: Some(ForecastDistribution {
                quantile_levels: vec![0.05, 0.25, 0.50, 0.75, 0.95],
                quantiles_sigma: vec![-1.5, -0.5, 0.0, 0.5, 1.5],
                quantiles_return: vec![-0.03, -0.01, 0.002, 0.012, 0.04],
                median_return: 0.002,
                sigma: 0.02,
            }),
        }
    }

    #[test]
    fn clamp_fraction_passes_through_without_bounds() {
        assert_eq!(InferenceGateway::clamp_fraction("0.5", None), "0.5");
    }

    #[test]
    fn clamp_fraction_bounds_value() {
        let clamp = ["0.01".to_string(), "0.10".to_string()];
        // Clamped to the max bound; `normalize()` drops the trailing zero.
        assert_eq!(
            InferenceGateway::clamp_fraction("0.25", Some(&clamp)),
            "0.1"
        );
        assert_eq!(
            InferenceGateway::clamp_fraction("0.001", Some(&clamp)),
            "0.01"
        );
        assert_eq!(
            InferenceGateway::clamp_fraction("0.05", Some(&clamp)),
            "0.05"
        );
    }

    #[test]
    fn clamp_fraction_unparseable_returns_raw() {
        let clamp = ["0.01".to_string(), "0.10".to_string()];
        assert_eq!(
            InferenceGateway::clamp_fraction("oops", Some(&clamp)),
            "oops"
        );
    }

    #[test]
    fn forecast_to_output_populates_distribution_fields() {
        let out = InferenceGateway::forecast_to_output(&result_with_dist());
        assert_eq!(out.direction, "up");
        assert!((out.confidence - 0.8).abs() < f64::EPSILON);
        assert_eq!(out.median_return, Some(0.002));
        assert_eq!(out.sigma, Some(0.02));
        assert_eq!(out.var_95, Some(-0.03));
        assert!(out.spread_90.is_some());
    }

    #[test]
    fn forecast_to_output_point_model_has_no_dist_fields() {
        let f = ForecastResult {
            direction: "down".into(),
            confidence: 0.6,
            latency_ms: 1,
            magnitude: "0.0".into(),
            distribution: None,
        };
        let out = InferenceGateway::forecast_to_output(&f);
        assert_eq!(out.direction, "down");
        assert!(out.median_return.is_none());
        assert!(out.var_95.is_none());
    }

    #[test]
    fn apply_abstain_flat_inserts_default() {
        use domain::strategy_def::nodes::AbstainPolicy;
        let mut outputs = HashMap::new();
        InferenceGateway::apply_abstain("n1", &AbstainPolicy::Flat, &mut outputs);
        assert!(outputs.contains_key("n1"));
        assert!(outputs["n1"].confidence.abs() < f64::EPSILON);
    }

    #[test]
    fn apply_abstain_hold_last_keeps_prior() {
        use domain::strategy_def::nodes::AbstainPolicy;
        let mut outputs = HashMap::new();
        outputs.insert(
            "n1".to_string(),
            InferenceOutput {
                direction: "up".into(),
                confidence: 0.9,
                ..Default::default()
            },
        );
        InferenceGateway::apply_abstain("n1", &AbstainPolicy::HoldLast, &mut outputs);
        // Prior value is untouched.
        assert!((outputs["n1"].confidence - 0.9).abs() < f64::EPSILON);
    }
}
