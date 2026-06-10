//! P7-T03: Milvus collection management.
//!
//! Creates and manages the `trading_social_posts` collection with 1536-dim
//! vectors and source/instrument/venue metadata for metadata-filtered search.

use serde::{Deserialize, Serialize};
use serde_json::json;
use tracing::info;

use crate::{MilvusClient, SemanticError, EMBEDDING_DIMS, SOCIAL_COLLECTION};

/// Field names in the Milvus collection schema.
pub mod fields {
    pub const ID: &str = "id";
    pub const VECTOR: &str = "vector";
    pub const SOURCE: &str = "source";
    pub const EVENT_ID: &str = "event_id";
    pub const INSTRUMENT_IDS: &str = "instrument_ids";
    pub const VENUE: &str = "venue";
    pub const TEXT: &str = "text";
    pub const OCCURRED_AT_MS: &str = "occurred_at_ms";
}

/// Known `source` metadata values.
pub mod sources {
    pub const SOCIAL_POST: &str = "social.post";
    pub const WEB_PAGE_SNAPSHOT: &str = "web.page_snapshot";
    pub const STRATEGY_DESCRIPTION: &str = "strategy.description";
}

#[derive(Debug, Serialize, Deserialize)]
#[allow(dead_code)]
struct CollectionListResponse {
    code: Option<i64>,
    data: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[allow(dead_code)]
struct MilvusResponse {
    code: Option<i64>,
    message: Option<String>,
}

impl MilvusClient {
    /// Ensure the social-posts collection exists, creating it if necessary.
    pub async fn ensure_collection(&self) -> Result<(), SemanticError> {
        if self.collection_exists(SOCIAL_COLLECTION).await? {
            info!(collection = SOCIAL_COLLECTION, "collection already exists");
            return Ok(());
        }
        self.create_collection(SOCIAL_COLLECTION, EMBEDDING_DIMS)
            .await
    }

    /// Return `true` if `collection_name` exists in Milvus.
    pub async fn collection_exists(&self, collection_name: &str) -> Result<bool, SemanticError> {
        let url = format!(
            "http://{}:{}/v2/vectordb/collections/list",
            self.config.host, self.config.http_port
        );
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| SemanticError::Request(e.to_string()))?;

        if !resp.status().is_success() {
            return Err(SemanticError::Collection(format!(
                "list collections failed: {}",
                resp.status()
            )));
        }

        let list: CollectionListResponse = resp
            .json()
            .await
            .map_err(|e| SemanticError::Collection(e.to_string()))?;

        Ok(list
            .data
            .unwrap_or_default()
            .iter()
            .any(|n| n == collection_name))
    }

    /// Create a collection with the trading-platform schema.
    pub async fn create_collection(
        &self,
        collection_name: &str,
        dims: u32,
    ) -> Result<(), SemanticError> {
        info!(
            collection = collection_name,
            dims, "creating Milvus collection"
        );

        let body = json!({
            "collectionName": collection_name,
            "schema": {
                "fields": [
                    {
                        "fieldName": fields::ID,
                        "dataType": "Int64",
                        "isPrimary": true,
                        "autoId": true
                    },
                    {
                        "fieldName": fields::VECTOR,
                        "dataType": "FloatVector",
                        "elementTypeParams": { "dim": dims.to_string() }
                    },
                    {
                        "fieldName": fields::SOURCE,
                        "dataType": "VarChar",
                        "elementTypeParams": { "max_length": "64" }
                    },
                    {
                        "fieldName": fields::EVENT_ID,
                        "dataType": "VarChar",
                        "elementTypeParams": { "max_length": "64" }
                    },
                    {
                        "fieldName": fields::INSTRUMENT_IDS,
                        "dataType": "VarChar",
                        "elementTypeParams": { "max_length": "512" }
                    },
                    {
                        "fieldName": fields::VENUE,
                        "dataType": "VarChar",
                        "elementTypeParams": { "max_length": "64" }
                    },
                    {
                        "fieldName": fields::TEXT,
                        "dataType": "VarChar",
                        "elementTypeParams": { "max_length": "8192" }
                    },
                    {
                        "fieldName": fields::OCCURRED_AT_MS,
                        "dataType": "Int64"
                    }
                ]
            },
            "indexParams": [
                {
                    "fieldName": fields::VECTOR,
                    "indexName": "vector_index",
                    "metricType": "COSINE",
                    "params": { "nlist": "128" }
                }
            ]
        });

        let url = format!(
            "http://{}:{}/v2/vectordb/collections/create",
            self.config.host, self.config.http_port
        );
        let resp = self
            .http
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| SemanticError::Request(e.to_string()))?;

        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(SemanticError::Collection(format!(
                "create collection failed: {text}"
            )));
        }

        Ok(())
    }

    /// Drop a collection by name.
    pub async fn drop_collection(&self, collection_name: &str) -> Result<(), SemanticError> {
        let url = format!(
            "http://{}:{}/v2/vectordb/collections/drop",
            self.config.host, self.config.http_port
        );
        let body = json!({ "collectionName": collection_name });
        let resp = self
            .http
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| SemanticError::Request(e.to_string()))?;

        let status = resp.status().as_u16();
        if !resp.status().is_success() && status != 404 {
            let text = resp.text().await.unwrap_or_default();
            return Err(SemanticError::Collection(format!(
                "drop collection failed: {text}"
            )));
        }
        Ok(())
    }
}
