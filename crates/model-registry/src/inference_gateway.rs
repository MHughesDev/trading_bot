//! Inference gateway — alias resolution, prediction caching, circuit breaking, and trace logging.

use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use sqlx::PgPool;
use tracing::{debug, warn};

use crate::sidecar::{PredictInstance, PredictRequest, SidecarClient};

/// TTL for alias → version cache entries.
const ALIAS_CACHE_TTL: Duration = Duration::from_secs(30);
/// TTL for prediction cache entries.
const PRED_CACHE_TTL: Duration = Duration::from_secs(5);
/// Number of consecutive failures before circuit breaks open.
const CIRCUIT_BREAK_THRESHOLD: u32 = 5;

#[derive(Clone, Debug)]
pub struct ForecastResult {
    pub direction: String,   // "bullish" | "bearish" | "neutral"
    pub confidence: f64,
    pub latency_ms: u64,
}

pub struct InferenceGateway {
    pg: PgPool,
    sidecar: Arc<SidecarClient>,
    /// (model_id, alias) → (version, cached_at)
    alias_cache: RwLock<HashMap<(String, String), (i32, Instant)>>,
    /// (model_id, version, feature_hash) → (result, cached_at)
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
        let row: Option<(i32,)> = sqlx::query_as(
            "SELECT version FROM model_aliases WHERE model_id = $1 AND alias = $2",
        )
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

        // Circuit break check.
        {
            let counts = self.fail_counts.read().unwrap();
            if counts.get(&model_id).copied().unwrap_or(0) >= CIRCUIT_BREAK_THRESHOLD {
                debug!("circuit break open for model {model_id}, abstaining");
                self.write_trace(&model_id, 0, instrument_id, 0, "abstain").await;
                return None;
            }
        }

        let effective_alias = if alias.is_empty() { "production" } else { alias };

        // Resolve alias → version.
        let version = match self.resolve_alias(&model_id, effective_alias).await {
            Some(v) => v,
            None => {
                warn!("alias '{effective_alias}' not found for model {model_id}");
                self.write_trace(&model_id, 0, instrument_id, 0, "abstain").await;
                return None;
            }
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

        let (artifact_uri, artifact_hash, model_kind) = match artifact_row {
            Some(row) => row,
            None => {
                warn!("artifact not found for model {model_id} v{version}");
                self.write_trace(&model_id, version, instrument_id, 0, "abstain").await;
                return None;
            }
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

                let result = ForecastResult {
                    direction: prediction.direction.clone(),
                    confidence: prediction.confidence,
                    latency_ms,
                };

                // Update prediction cache.
                {
                    let mut cache = self.pred_cache.write().unwrap();
                    cache.insert(cache_key, (result.clone(), Instant::now()));
                }

                self.write_trace(&model_id, version, instrument_id, latency_ms, "hit").await;
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
                    if let Some(fallback_version) = self.resolve_alias(&model_id, "fallback").await {
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
                                    let result = ForecastResult {
                                        direction: fb_pred.direction.clone(),
                                        confidence: fb_pred.confidence,
                                        latency_ms,
                                    };
                                    self.write_trace(&model_id, fallback_version, instrument_id, latency_ms, "miss").await;
                                    return Some(result);
                                }
                            }
                        }
                    }
                }

                self.write_trace(&model_id, version, instrument_id, latency_ms, "error").await;
                None
            }
        }
    }

    /// Refresh model forecast results for a node list into a cache HashMap.
    /// Called by the strategy runtime layer before dispatch to pre-populate sync-readable results.
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

    /// Write a trace record to model_events table.
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

    /// Return recent inference traces from model_events.
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
