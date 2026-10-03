//! Universe membership from the database (Set U U-6.1; FEAT-007 §4; Set P P-4.3).
//!
//! The one loader every environment uses: a backtest loads a universe once and
//! asks it at each decision instant; a live allocator loads it and asks *now*.
//! Both resolve through [`strategy_engine::Membership`], so the same definition
//! gives the same members for the same instant wherever it is asked.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use dataplane::universe::{MembershipChange, MembershipEvent};
use dataplane::InstrumentKey;
use domain::strategy_graph::UniverseRef;
use strategy_engine::Membership;

/// Why a universe could not be loaded.
#[derive(Debug, thiserror::Error)]
pub enum UniverseLoadError {
    #[error("there is no universe named '{0}' (dataplane.universe)")]
    UnknownUniverse(String),
    #[error("universe '{name}': {source}")]
    Inconsistent {
        name: String,
        source: dataplane::universe::UniverseError,
    },
    #[error("an asset-class universe for '{0}' holds no initialised instruments; initialise an asset first")]
    EmptyAssetClass(String),
    #[error("an explicit universe lists no instruments")]
    EmptyList,
    #[error("database: {0}")]
    Db(#[from] sqlx::Error),
}

/// Load a selection strategy's universe.
///
/// - `list` — the instruments as written; not point-in-time.
/// - `asset_class` — every initialised instrument of the class
///   (`asset_lifecycle`), as it is today; not point-in-time.
/// - `named` — the append-only membership log, resolved on both of its clocks.
///
/// # Errors
/// An unknown universe, an inconsistent log, an empty one, or the database.
pub async fn load(
    pg: &sqlx::PgPool,
    universe: &UniverseRef,
) -> Result<Membership, UniverseLoadError> {
    match universe {
        UniverseRef::List { instruments } => {
            if instruments.is_empty() {
                return Err(UniverseLoadError::EmptyList);
            }
            Ok(Membership::fixed(instruments.iter().cloned()))
        }
        UniverseRef::AssetClass { asset_class } => {
            let rows: Vec<(String,)> = sqlx::query_as(
                "SELECT symbol FROM asset_lifecycle WHERE asset_class = $1 ORDER BY symbol",
            )
            .bind(asset_class)
            .fetch_all(pg)
            .await?;
            if rows.is_empty() {
                return Err(UniverseLoadError::EmptyAssetClass(asset_class.clone()));
            }
            Ok(Membership::fixed(rows.into_iter().map(|(s,)| s)))
        }
        UniverseRef::Named { name } => {
            let exists: Option<(String,)> =
                sqlx::query_as("SELECT name FROM dataplane.universe WHERE name = $1")
                    .bind(name)
                    .fetch_optional(pg)
                    .await?;
            if exists.is_none() {
                return Err(UniverseLoadError::UnknownUniverse(name.clone()));
            }
            type Row = (i64, String, DateTime<Utc>, DateTime<Utc>, Option<String>);
            let rows: Vec<Row> = sqlx::query_as(
                "SELECT instrument_id, change, effective, recorded_at, reason \
                   FROM dataplane.universe_membership_event \
                  WHERE universe = $1 ORDER BY recorded_seq",
            )
            .bind(name)
            .fetch_all(pg)
            .await?;
            let log = rows
                .into_iter()
                .map(
                    |(key, change, effective, recorded_at, reason)| MembershipEvent {
                        universe: name.clone(),
                        instrument_id: InstrumentKey(key),
                        change: if change == "removed" {
                            MembershipChange::Removed
                        } else {
                            MembershipChange::Added
                        },
                        effective,
                        recorded_at,
                        reason: reason.unwrap_or_default(),
                    },
                )
                .collect();
            Membership::logged(log, symbols(pg).await?).map_err(|source| {
                UniverseLoadError::Inconsistent {
                    name: name.clone(),
                    source,
                }
            })
        }
    }
}

/// Instrument key → the symbol its bars are stored under. The platform's own
/// instrument table wins; the bitemporal symbol table names the rest by their
/// latest symbol.
async fn symbols(pg: &sqlx::PgPool) -> Result<BTreeMap<i64, String>, sqlx::Error> {
    let mut out = BTreeMap::new();
    let latest: Vec<(i64, String)> = sqlx::query_as(
        "SELECT DISTINCT ON (instrument_id) instrument_id, symbol \
           FROM dataplane.instrument_symbol \
          ORDER BY instrument_id, valid_from DESC, knowledge_time DESC",
    )
    .fetch_all(pg)
    .await?;
    out.extend(latest);
    let platform: Vec<(i64, String)> = sqlx::query_as(
        "SELECT instrument_key, instrument_id FROM instruments WHERE instrument_key IS NOT NULL",
    )
    .fetch_all(pg)
    .await?;
    out.extend(platform);
    Ok(out)
}
