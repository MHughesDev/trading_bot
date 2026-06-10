//! P7-T01: TigerGraph schema initialization.
//!
//! Creates the capability/compatibility graph schema idempotently via the
//! TigerGraph REST++ GSQL endpoint.  Re-running is a no-op (uses IF NOT EXISTS).

use serde::Deserialize;
use tracing::info;

use crate::{GraphError, TigerGraphClient};

/// Vertex type names in the capability graph.
pub mod vertex_types {
    pub const ASSET_CLASS: &str = "AssetClass";
    pub const INSTRUMENT: &str = "Instrument";
    pub const VENUE: &str = "Venue";
    pub const DATA_TYPE: &str = "DataType";
    pub const STRATEGY_DEFINITION: &str = "StrategyDefinition";
    pub const WIDGET: &str = "Widget";
}

/// Edge type names in the capability graph.
pub mod edge_types {
    pub const INSTRUMENT_IS_A: &str = "INSTRUMENT_IS_A";
    pub const VENUE_PROVIDES: &str = "VENUE_PROVIDES";
    pub const STRATEGY_REQUIRES_DATA: &str = "STRATEGY_REQUIRES_DATA";
    pub const INSTRUMENT_AT_VENUE: &str = "INSTRUMENT_AT_VENUE";
    pub const VENUE_SUPPORTS_ASSET_CLASS: &str = "VENUE_SUPPORTS_ASSET_CLASS";
    pub const STRATEGY_FOR_ASSET_CLASS: &str = "STRATEGY_FOR_ASSET_CLASS";
}

/// All expected vertex types in the capability graph.
pub const ALL_VERTEX_TYPES: &[&str] = &[
    vertex_types::ASSET_CLASS,
    vertex_types::INSTRUMENT,
    vertex_types::VENUE,
    vertex_types::DATA_TYPE,
    vertex_types::STRATEGY_DEFINITION,
    vertex_types::WIDGET,
];

/// All expected edge types in the capability graph.
pub const ALL_EDGE_TYPES: &[&str] = &[
    edge_types::INSTRUMENT_IS_A,
    edge_types::VENUE_PROVIDES,
    edge_types::STRATEGY_REQUIRES_DATA,
    edge_types::INSTRUMENT_AT_VENUE,
    edge_types::VENUE_SUPPORTS_ASSET_CLASS,
    edge_types::STRATEGY_FOR_ASSET_CLASS,
];

/// Embedded GSQL schema DDL.
pub const SCHEMA_GSQL: &str = include_str!("../schema/capability_graph.gsql");

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct GraphSchemaResponse {
    results: Option<GraphSchemaResults>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct GraphSchemaResults {
    #[serde(rename = "VertexTypes", default)]
    vertex_types: Vec<VertexTypeDef>,
    #[serde(rename = "EdgeTypes", default)]
    edge_types: Vec<EdgeTypeDef>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct VertexTypeDef {
    #[serde(rename = "Name")]
    name: String,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct EdgeTypeDef {
    #[serde(rename = "Name")]
    name: String,
}

impl TigerGraphClient {
    /// Initialize the capability graph schema idempotently.
    ///
    /// Submits the embedded GSQL DDL via the TigerGraph REST++ GSQL endpoint.
    /// Re-running is safe — all DDL statements use `IF NOT EXISTS`.
    pub async fn init_schema(&self) -> Result<(), GraphError> {
        info!(graph = %self.config.graph_name, "initializing TigerGraph schema");

        let url = format!(
            "http://{}:{}/gsql/v1/statements",
            self.config.host, self.config.port
        );

        let resp = self
            .http
            .post(&url)
            .basic_auth(&self.config.username, Some(&self.config.password))
            .header("Content-Type", "text/plain")
            .body(SCHEMA_GSQL)
            .send()
            .await
            .map_err(|e| GraphError::Request(e.to_string()))?;

        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(GraphError::Response(format!("schema init failed: {text}")));
        }

        Ok(())
    }

    /// List vertex type names currently in the graph.
    pub async fn list_vertex_types(&self) -> Result<Vec<String>, GraphError> {
        let schema = self.fetch_schema().await?;
        Ok(schema
            .results
            .map(|r| r.vertex_types.into_iter().map(|v| v.name).collect())
            .unwrap_or_default())
    }

    /// List edge type names currently in the graph.
    pub async fn list_edge_types(&self) -> Result<Vec<String>, GraphError> {
        let schema = self.fetch_schema().await?;
        Ok(schema
            .results
            .map(|r| r.edge_types.into_iter().map(|e| e.name).collect())
            .unwrap_or_default())
    }

    async fn fetch_schema(&self) -> Result<GraphSchemaResponse, GraphError> {
        let url = format!(
            "http://{}:{}/gsql/v1/schema/graphs/{}",
            self.config.host, self.config.port, self.config.graph_name
        );
        let resp = self
            .http
            .get(&url)
            .basic_auth(&self.config.username, Some(&self.config.password))
            .send()
            .await
            .map_err(|e| GraphError::Request(e.to_string()))?;

        if !resp.status().is_success() {
            return Err(GraphError::Response(resp.status().to_string()));
        }

        resp.json()
            .await
            .map_err(|e| GraphError::Response(e.to_string()))
    }
}
