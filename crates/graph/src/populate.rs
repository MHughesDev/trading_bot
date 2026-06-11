//! P7-T02: Graph population from registries (derived, rebuildable).
//!
//! Populates the TigerGraph capability graph from Postgres/code registries.
//! The graph is a projection — any data it holds is derivable from the
//! canonical registries.  A full rebuild drops and re-derives exactly (C-102).

use serde_json::{json, Map, Value};
use tracing::info;

use crate::{
    schema::{edge_types, vertex_types},
    GraphError, TigerGraphClient,
};

/// Specification for a single instrument in the registry.
#[derive(Debug, Clone)]
pub struct InstrumentSpec {
    pub instrument_id: String,
    pub asset_class: String,
    pub venue_id: String,
    pub active: bool,
}

/// Specification for a single venue in the registry.
#[derive(Debug, Clone)]
pub struct VenueSpec {
    pub venue_id: String,
    pub name: String,
    /// DataType keys this venue provides (e.g. `["market.ohlcv", "market.trade"]`).
    pub data_types: Vec<String>,
    /// AssetClass keys this venue supports.
    pub asset_classes: Vec<String>,
}

/// Specification for a strategy definition.
#[derive(Debug, Clone)]
pub struct StrategySpec {
    pub strategy_id: String,
    pub name: String,
    pub version: String,
    pub asset_class: String,
    /// DataType keys this strategy requires.
    pub required_data_types: Vec<String>,
}

/// A full snapshot of registry data to be projected into the graph.
#[derive(Debug, Default)]
pub struct RegistrySnapshot {
    /// Asset class string keys (serde snake_case, e.g. `"crypto_spot_cex"`).
    pub asset_classes: Vec<String>,
    pub instruments: Vec<InstrumentSpec>,
    pub venues: Vec<VenueSpec>,
    /// DataType keys (e.g. `"market.ohlcv"`, `"social.post"`).
    pub data_types: Vec<String>,
    pub strategies: Vec<StrategySpec>,
}

impl RegistrySnapshot {
    /// Build a snapshot seeded from the domain's static registries.
    ///
    /// Produces a deterministic baseline for seeding a fresh TigerGraph
    /// instance and for use in tests.
    pub fn from_domain_defaults() -> Self {
        use domain::AssetClass;

        let asset_classes: Vec<String> = [
            AssetClass::CryptoSpotCex,
            AssetClass::Equity,
            AssetClass::Etf,
            AssetClass::CryptoSpotDex,
            AssetClass::FuturesExpiring,
            AssetClass::PerpetualSwap,
            AssetClass::Option,
            AssetClass::Bond,
            AssetClass::Fx,
            AssetClass::Nft,
            AssetClass::PredictionMarket,
        ]
        .iter()
        .map(|a| a.as_str().to_owned())
        .collect();

        let data_types: Vec<String> = domain::DataType::all()
            .iter()
            .map(|dt| dt.as_key().to_owned())
            .collect();

        Self {
            asset_classes,
            data_types,
            instruments: vec![],
            venues: vec![],
            strategies: vec![],
        }
    }
}

impl TigerGraphClient {
    /// Upsert all vertices and edges from `snapshot` into the graph.
    pub async fn populate(&self, snapshot: &RegistrySnapshot) -> Result<(), GraphError> {
        info!(
            instruments = snapshot.instruments.len(),
            venues = snapshot.venues.len(),
            strategies = snapshot.strategies.len(),
            "populating capability graph from registry snapshot"
        );

        let mut vertices: Map<String, Value> = Map::new();

        // AssetClass vertices.
        let mut ac_map = Map::new();
        for ac in &snapshot.asset_classes {
            ac_map.insert(
                ac.clone(),
                json!({ "name": {"value": ac}, "label": {"value": ac} }),
            );
        }
        vertices.insert(vertex_types::ASSET_CLASS.to_owned(), Value::Object(ac_map));

        // DataType vertices.
        let mut dt_map = Map::new();
        for dt in &snapshot.data_types {
            dt_map.insert(dt.clone(), json!({ "name": {"value": dt} }));
        }
        vertices.insert(vertex_types::DATA_TYPE.to_owned(), Value::Object(dt_map));

        // Instrument vertices.
        let mut inst_map = Map::new();
        for inst in &snapshot.instruments {
            inst_map.insert(
                inst.instrument_id.clone(),
                json!({
                    "name":        {"value": &inst.instrument_id},
                    "asset_class": {"value": &inst.asset_class},
                    "venue_id":    {"value": &inst.venue_id},
                    "active":      {"value": inst.active},
                }),
            );
        }
        vertices.insert(vertex_types::INSTRUMENT.to_owned(), Value::Object(inst_map));

        // Venue vertices.
        let mut venue_map = Map::new();
        for v in &snapshot.venues {
            venue_map.insert(v.venue_id.clone(), json!({ "name": {"value": &v.name} }));
        }
        vertices.insert(vertex_types::VENUE.to_owned(), Value::Object(venue_map));

        // StrategyDefinition vertices.
        let mut strat_map = Map::new();
        for s in &snapshot.strategies {
            strat_map.insert(
                s.strategy_id.clone(),
                json!({
                    "name":        {"value": &s.name},
                    "version":     {"value": &s.version},
                    "asset_class": {"value": &s.asset_class},
                }),
            );
        }
        vertices.insert(
            vertex_types::STRATEGY_DEFINITION.to_owned(),
            Value::Object(strat_map),
        );

        // ── Edges ─────────────────────────────────────────────────────────────

        let mut edges: Map<String, Value> = Map::new();

        // INSTRUMENT_IS_A edges.
        let mut isa_inst: Map<String, Value> = Map::new();
        for inst in &snapshot.instruments {
            let mut to_ac = Map::new();
            to_ac.insert(inst.asset_class.clone(), json!({}));
            isa_inst.insert(
                inst.instrument_id.clone(),
                json!({ vertex_types::ASSET_CLASS: to_ac }),
            );
        }
        edges.insert(
            edge_types::INSTRUMENT_IS_A.to_owned(),
            json!({ vertex_types::INSTRUMENT: isa_inst }),
        );

        // VENUE_PROVIDES edges.
        let mut vp_venue: Map<String, Value> = Map::new();
        for v in &snapshot.venues {
            let dt_targets: Map<String, Value> = v
                .data_types
                .iter()
                .map(|dt| (dt.clone(), json!({})))
                .collect();
            vp_venue.insert(
                v.venue_id.clone(),
                json!({ vertex_types::DATA_TYPE: dt_targets }),
            );
        }
        edges.insert(
            edge_types::VENUE_PROVIDES.to_owned(),
            json!({ vertex_types::VENUE: vp_venue }),
        );

        // STRATEGY_REQUIRES_DATA edges.
        let mut srd_strat: Map<String, Value> = Map::new();
        for s in &snapshot.strategies {
            let dt_targets: Map<String, Value> = s
                .required_data_types
                .iter()
                .map(|dt| (dt.clone(), json!({})))
                .collect();
            srd_strat.insert(
                s.strategy_id.clone(),
                json!({ vertex_types::DATA_TYPE: dt_targets }),
            );
        }
        edges.insert(
            edge_types::STRATEGY_REQUIRES_DATA.to_owned(),
            json!({ vertex_types::STRATEGY_DEFINITION: srd_strat }),
        );

        // INSTRUMENT_AT_VENUE edges.
        let mut iav_inst: Map<String, Value> = Map::new();
        for inst in &snapshot.instruments {
            let mut to_venue = Map::new();
            to_venue.insert(inst.venue_id.clone(), json!({}));
            iav_inst.insert(
                inst.instrument_id.clone(),
                json!({ vertex_types::VENUE: to_venue }),
            );
        }
        edges.insert(
            edge_types::INSTRUMENT_AT_VENUE.to_owned(),
            json!({ vertex_types::INSTRUMENT: iav_inst }),
        );

        // VENUE_SUPPORTS_ASSET_CLASS edges (M-6).
        let mut vsac_venue: Map<String, Value> = Map::new();
        for v in &snapshot.venues {
            let ac_targets: Map<String, Value> = v
                .asset_classes
                .iter()
                .map(|ac| (ac.clone(), json!({})))
                .collect();
            vsac_venue.insert(
                v.venue_id.clone(),
                json!({ vertex_types::ASSET_CLASS: ac_targets }),
            );
        }
        edges.insert(
            edge_types::VENUE_SUPPORTS_ASSET_CLASS.to_owned(),
            json!({ vertex_types::VENUE: vsac_venue }),
        );

        // STRATEGY_FOR_ASSET_CLASS edges (M-6).
        let mut sfac_strat: Map<String, Value> = Map::new();
        for s in &snapshot.strategies {
            let mut to_ac = Map::new();
            to_ac.insert(s.asset_class.clone(), json!({}));
            sfac_strat.insert(
                s.strategy_id.clone(),
                json!({ vertex_types::ASSET_CLASS: to_ac }),
            );
        }
        edges.insert(
            edge_types::STRATEGY_FOR_ASSET_CLASS.to_owned(),
            json!({ vertex_types::STRATEGY_DEFINITION: sfac_strat }),
        );

        let body = json!({
            "vertices": vertices,
            "edges":    edges,
        });

        // REST++ data API requires the /restpp path prefix (H-10).
        let url = format!(
            "http://{}:{}/restpp/graph/{}",
            self.config.host, self.config.restpp_port, self.config.graph_name
        );
        let resp = self
            .http
            .post(&url)
            .basic_auth(&self.config.username, Some(&self.config.password))
            .json(&body)
            .send()
            .await
            .map_err(|e| GraphError::Request(e.to_string()))?;

        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(GraphError::Response(format!("populate failed: {text}")));
        }

        Ok(())
    }

    /// Delete all vertices (and their incident edges) of every type.
    ///
    /// Used as the first step of a full rebuild.
    ///
    /// Collects all deletion errors rather than returning on the first failure
    /// so a partial wipe does not silently leave stale data in place (M-7).
    pub async fn drop_all_data(&self) -> Result<(), GraphError> {
        info!("dropping all graph data for rebuild");
        let mut errors: Vec<String> = Vec::new();

        for vtype in crate::schema::ALL_VERTEX_TYPES {
            // REST++ data API requires the /restpp path prefix (H-10).
            let url = format!(
                "http://{}:{}/restpp/graph/{}/vertices/{}",
                self.config.host, self.config.restpp_port, self.config.graph_name, vtype
            );
            match self
                .http
                .delete(&url)
                .basic_auth(&self.config.username, Some(&self.config.password))
                .send()
                .await
            {
                Err(e) => errors.push(format!("{vtype}: request failed: {e}")),
                Ok(resp) => {
                    let status = resp.status().as_u16();
                    if !resp.status().is_success() && status != 404 {
                        let text = resp.text().await.unwrap_or_default();
                        errors.push(format!("{vtype}: {text}"));
                    }
                }
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(GraphError::Response(format!(
                "drop_all_data had {} failure(s): {}",
                errors.len(),
                errors.join("; ")
            )))
        }
    }

    /// Full rebuild: drop all data then repopulate from `snapshot`.
    ///
    /// Produces an identical graph on every run — derived and rebuildable (C-102).
    pub async fn rebuild(&self, snapshot: &RegistrySnapshot) -> Result<(), GraphError> {
        self.drop_all_data().await?;
        self.populate(snapshot).await
    }
}
