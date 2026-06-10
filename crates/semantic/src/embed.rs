//! P7-T03: Embedding pipeline — OpenAI text-embedding-3-small → Milvus.
//!
//! Embeds social posts, web-page snapshots, and strategy descriptions.
//! Metadata-filtered semantic search returns only the requested source type.

use serde::{Deserialize, Serialize};
use serde_json::json;
use tracing::{debug, info};
use uuid::Uuid;

use crate::{
    collection::{fields, sources},
    MilvusClient, SemanticError, EMBEDDING_MODEL, SOCIAL_COLLECTION,
};

const OPENAI_EMBED_URL: &str = "https://api.openai.com/v1/embeddings";

/// Source category for an embedding.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EmbedSource {
    SocialPost,
    WebPageSnapshot,
    StrategyDescription,
}

impl EmbedSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::SocialPost => sources::SOCIAL_POST,
            Self::WebPageSnapshot => sources::WEB_PAGE_SNAPSHOT,
            Self::StrategyDescription => sources::STRATEGY_DESCRIPTION,
        }
    }
}

/// A record to embed and upsert into Milvus.
#[derive(Debug, Clone)]
pub struct EmbeddingRequest {
    /// Text to embed (will be truncated if > 8190 chars).
    pub text: String,
    /// Source category for metadata filtering.
    pub source: EmbedSource,
    /// Opaque event ID (dedup anchor).
    pub event_id: String,
    /// Comma-separated instrument IDs mentioned in the text.
    pub instrument_ids: String,
    /// Originating venue (e.g. `"reddit"`, `"web"`, `"platform"`).
    pub venue: String,
    /// Unix millisecond timestamp of the original event.
    pub occurred_at_ms: i64,
}

/// A single semantic search result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub event_id: String,
    pub source: String,
    pub venue: String,
    pub text: String,
    pub distance: f32,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct OpenAiEmbedResponse {
    data: Vec<OpenAiEmbedDatum>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct OpenAiEmbedDatum {
    embedding: Vec<f32>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct MilvusSearchResponse {
    code: Option<i64>,
    data: Option<Vec<MilvusSearchHit>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
struct MilvusSearchHit {
    distance: f32,
    #[serde(default)]
    event_id: String,
    #[serde(default)]
    source: String,
    #[serde(default)]
    venue: String,
    #[serde(default)]
    text: String,
}

impl MilvusClient {
    /// Embed `req.text` via OpenAI and upsert the vector + metadata to Milvus.
    pub async fn embed_and_upsert(
        &self,
        req: &EmbeddingRequest,
        openai_api_key: &str,
    ) -> Result<(), SemanticError> {
        debug!(source = req.source.as_str(), event_id = %req.event_id, "embedding text");

        let vector = call_openai_embedding(&req.text, openai_api_key, &self.http).await?;

        // Truncate on a character boundary, not a byte index, to avoid a panic
        // when a multi-byte character straddles the 8190-byte limit (M-11).
        // Milvus max_length counts characters, so char-count truncation is also
        // semantically correct.
        const MAX_TEXT_CHARS: usize = 8190;
        let truncated_text: String = req.text.chars().take(MAX_TEXT_CHARS).collect();

        // Derive a deterministic Int64 primary key from event_id so that
        // re-embedding the same event replaces the existing record rather than
        // inserting a duplicate (M-1: autoId:false requires client-supplied id).
        let pk_id = event_id_to_i64(&req.event_id);

        let entity = json!({
            fields::ID:              pk_id,
            fields::VECTOR:          vector,
            fields::SOURCE:          req.source.as_str(),
            fields::EVENT_ID:        req.event_id,
            fields::INSTRUMENT_IDS:  req.instrument_ids,
            fields::VENUE:           req.venue,
            fields::TEXT:            &truncated_text,
            fields::OCCURRED_AT_MS:  req.occurred_at_ms,
        });

        let url = format!(
            "http://{}:{}/v2/vectordb/entities/upsert",
            self.config.host, self.config.http_port
        );
        let body = json!({
            "collectionName": SOCIAL_COLLECTION,
            "data": [entity],
        });

        let resp = self
            .authed_post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| SemanticError::Request(e.to_string()))?;

        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(SemanticError::Request(format!("upsert failed: {text}")));
        }

        info!(source = req.source.as_str(), event_id = %req.event_id, "embedding upserted");
        Ok(())
    }

    /// Semantic search with an optional source filter.
    ///
    /// If `source_filter` is `Some("social.post")`, only embeddings with that
    /// source are returned.  Pass `None` to search across all sources.
    pub async fn search_similar(
        &self,
        query_text: &str,
        source_filter: Option<&str>,
        limit: u32,
        openai_api_key: &str,
    ) -> Result<Vec<SearchResult>, SemanticError> {
        let vector = call_openai_embedding(query_text, openai_api_key, &self.http).await?;

        let mut body = json!({
            "collectionName": SOCIAL_COLLECTION,
            "data": [vector],
            "limit": limit,
            "outputFields": [
                fields::EVENT_ID,
                fields::SOURCE,
                fields::VENUE,
                fields::TEXT,
            ],
        });

        if let Some(filter) = source_filter {
            // Validate against the closed set of known source constants to
            // prevent Milvus DSL injection via an arbitrary filter string (H-3).
            let valid_sources = [
                sources::SOCIAL_POST,
                sources::WEB_PAGE_SNAPSHOT,
                sources::STRATEGY_DESCRIPTION,
            ];
            if !valid_sources.contains(&filter) {
                return Err(SemanticError::Request(format!(
                    "unknown source filter: {filter}"
                )));
            }
            body.as_object_mut()
                .unwrap()
                .insert("filter".into(), json!(format!("source == \"{filter}\"")));
        }

        let url = format!(
            "http://{}:{}/v2/vectordb/entities/search",
            self.config.host, self.config.http_port
        );

        let resp = self
            .authed_post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| SemanticError::Request(e.to_string()))?;

        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(SemanticError::Request(format!("search failed: {text}")));
        }

        let search_resp: MilvusSearchResponse = resp
            .json()
            .await
            .map_err(|e| SemanticError::Response(e.to_string()))?;

        Ok(search_resp
            .data
            .unwrap_or_default()
            .into_iter()
            .map(|h| SearchResult {
                event_id: h.event_id,
                source: h.source,
                venue: h.venue,
                text: h.text,
                distance: h.distance,
            })
            .collect())
    }
}

/// Convert an event_id string to a stable Int64 primary key for Milvus.
///
/// Parses the string as a UUID and folds the 128-bit value to i64 via XOR,
/// producing a deterministic key that allows upsert to replace existing records.
fn event_id_to_i64(event_id: &str) -> i64 {
    let uuid = Uuid::parse_str(event_id).unwrap_or_else(|_| Uuid::new_v5(&Uuid::NAMESPACE_OID, event_id.as_bytes()));
    let (hi, lo) = uuid.as_u64_pair();
    (hi ^ lo) as i64
}

async fn call_openai_embedding(
    text: &str,
    api_key: &str,
    http: &reqwest::Client,
) -> Result<Vec<f32>, SemanticError> {
    let body = json!({
        "model": EMBEDDING_MODEL,
        "input": text,
    });

    let resp = http
        .post(OPENAI_EMBED_URL)
        .bearer_auth(api_key)
        .json(&body)
        .send()
        .await
        .map_err(|e| SemanticError::Request(e.to_string()))?;

    if !resp.status().is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(SemanticError::Request(format!(
            "OpenAI embedding failed: {text}"
        )));
    }

    let embed_resp: OpenAiEmbedResponse = resp
        .json()
        .await
        .map_err(|e| SemanticError::Response(e.to_string()))?;

    embed_resp
        .data
        .into_iter()
        .next()
        .map(|d| d.embedding)
        .ok_or_else(|| SemanticError::Response("empty embedding response".into()))
}
