//! Discovery tools: `list_lanes` and `list_instruments`.
//!
//! `list_lanes` returns canonical lane names from domain constants.
//! `list_instruments` queries the live Postgres `instruments` table; the
//! caller must supply a `PgPool` obtained from the environment's `DATABASE_URL`.

use domain::lanes::ALL_LANES;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

#[derive(Debug, Serialize, Deserialize)]
pub struct LaneInfo {
    pub lane: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct InstrumentInfo {
    pub instrument_id: String,
    pub asset_class: String,
    pub venue_id: String,
    pub tick_size: String,
    pub trust_tier: String,
    pub active: bool,
}

/// `list_lanes` — return canonical data lanes from the shared domain constant.
pub fn list_lanes() -> Vec<LaneInfo> {
    ALL_LANES
        .iter()
        .map(|&lane| LaneInfo { lane: lane.to_owned() })
        .collect()
}

/// `list_instruments` — query the Postgres `instruments` table.
/// Returns an empty list (with a tracing warning) when the DB is unavailable.
pub async fn list_instruments(pg: &PgPool, asset_class: Option<&str>) -> Vec<InstrumentInfo> {
    let rows: Result<Vec<(String, String, String, String, String, bool)>, _> = match asset_class {
        None => {
            sqlx::query_as(
                "SELECT instrument_id, asset_class, venue_id, \
                        tick_size::TEXT, trust_tier, active \
                 FROM instruments ORDER BY instrument_id",
            )
            .fetch_all(pg)
            .await
        }
        Some(ac) => {
            sqlx::query_as(
                "SELECT instrument_id, asset_class, venue_id, \
                        tick_size::TEXT, trust_tier, active \
                 FROM instruments WHERE asset_class = $1 ORDER BY instrument_id",
            )
            .bind(ac)
            .fetch_all(pg)
            .await
        }
    };

    match rows {
        Ok(rs) => rs
            .into_iter()
            .map(
                |(instrument_id, asset_class, venue_id, tick_size, trust_tier, active)| {
                    InstrumentInfo {
                        instrument_id,
                        asset_class,
                        venue_id,
                        tick_size,
                        trust_tier,
                        active,
                    }
                },
            )
            .collect(),
        Err(e) => {
            tracing::warn!(error = %e, "list_instruments: DB query failed; returning empty list");
            vec![]
        }
    }
}
