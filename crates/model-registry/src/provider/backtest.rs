//! The backtest's prediction source (Set U U-5.4).
//!
//! `backtest::models::PitModels` decides *whether* a version may answer at a
//! decision instant (point-in-time) and caches what it said; this is what it
//! asks when the rule allows and the cache is empty: the inference sidecar,
//! protocol v2, the exact artifact of the pinned version.

use std::sync::Arc;

use backtest::models::{PredictionSource, VersionEntry};
use strategy_engine::{ModelOutcome, ModelRequest};

use super::{outcome_of, v2_features, V2Client};
use crate::sidecar::{PredictV2Request, V2Row};

/// The sidecar, called synchronously from the backtest loop.
pub struct SidecarSource {
    client: Arc<dyn V2Client>,
    handle: tokio::runtime::Handle,
}

impl SidecarSource {
    #[must_use]
    pub fn new(client: Arc<dyn V2Client>, handle: tokio::runtime::Handle) -> Self {
        Self { client, handle }
    }
}

impl PredictionSource for SidecarSource {
    fn predict(&mut self, entry: &VersionEntry, req: &ModelRequest) -> ModelOutcome {
        let reference = format!("{}@{}", entry.model, entry.version);
        let Some(contract) = &entry.contract else {
            return ModelOutcome::Unavailable {
                detail: format!("{reference} has no contract"),
            };
        };
        let is_transform = entry.kind == "transform";
        let (uri, sha) = match (&entry.artifact_uri, &entry.artifact_sha256) {
            (Some(u), Some(s)) => (u.clone(), s.clone()),
            _ if is_transform && entry.transform.is_some() => (String::new(), String::new()),
            _ => {
                return ModelOutcome::Unavailable {
                    detail: format!("{reference} has no artifact to serve"),
                }
            }
        };
        let features = match v2_features(req, contract) {
            Ok(f) => f,
            Err(detail) => return ModelOutcome::Unavailable { detail },
        };
        let body = PredictV2Request {
            model_id: entry.model.clone(),
            version: i32::try_from(entry.version).unwrap_or(i32::MAX),
            model_kind: entry.kind.clone(),
            artifact_uri: uri,
            artifact_hash: sha,
            rows: vec![V2Row {
                key: "0".into(),
                features,
            }],
            transform: entry.transform.clone(),
        };
        let client = Arc::clone(&self.client);
        let reply = tokio::task::block_in_place(|| self.handle.block_on(client.predict_v2(body)));
        match reply {
            Ok(r) => r.answers.first().map_or_else(
                || ModelOutcome::Unavailable {
                    detail: "the sidecar returned no answer".into(),
                },
                |a| outcome_of(a, contract, &reference),
            ),
            Err(e) => ModelOutcome::Unavailable {
                detail: format!("sidecar: {e}"),
            },
        }
    }
}
