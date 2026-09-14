//! Instrument identity service (SPEC §1.1, INV-04).
//!
//! Postgres `dataplane.instrument` / `instrument_symbol` / `venue` are the system of
//! record. This service allocates surrogate ids for `(venue, symbol)` pairs the
//! platform has not seen, declares each source's vendor lag, and replicates the
//! identity dimensions to ClickHouse so the single PIT reader can resolve symbols
//! inside its query.

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use clickhouse::Row;
use serde::Serialize;
use sqlx::PgPool;
use tokio::sync::RwLock;

use dataplane::identity::{AssetClass, InstrumentKey, VenueKey};

#[derive(Debug, thiserror::Error)]
pub enum IdentityError {
    #[error("postgres: {0}")]
    Pg(#[from] sqlx::Error),
    #[error("clickhouse: {0}")]
    Ch(String),
    #[error("unknown venue {0:?}: add it to dataplane.venue")]
    UnknownVenue(String),
    #[error("unknown source {0:?}: add it to dataplane.source with a declared vendor lag")]
    UnknownSource(String),
}

/// `(instrument_id, venue_id, symbol, valid_from, valid_to, knowledge_time)` as read
/// from `dataplane.instrument_symbol`.
type SymbolFactRow = (i64, i32, String, DateTime<Utc>, Option<DateTime<Utc>>, DateTime<Utc>);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceInfo {
    pub source_id: i32,
    pub declared_vendor_lag: std::time::Duration,
    pub live: bool,
}

/// Identity resolution used by bar writers.
#[async_trait::async_trait]
pub trait IdentityResolver: Send + Sync {
    /// The surrogate id for `(venue, symbol)` at `at`, allocating one if the pair has
    /// never been seen.
    async fn ensure_instrument(&self, venue: &str, symbol: &str, asset_class: AssetClass, at: DateTime<Utc>) -> Result<(InstrumentKey, VenueKey), IdentityError>;

    async fn source(&self, name: &str) -> Result<SourceInfo, IdentityError>;
}

pub struct PgIdentityService {
    pg: PgPool,
    ch_url: String,
    cache: RwLock<HashMap<(String, String), (InstrumentKey, VenueKey)>>,
    sources: RwLock<HashMap<String, SourceInfo>>,
}

#[derive(Row, Serialize)]
struct SymbolDimRow {
    instrument_id: i64,
    venue_id: i32,
    symbol: String,
    valid_from: i64,
    valid_to: Option<i64>,
    knowledge_time: i64,
}

#[derive(Row, Serialize)]
struct VenueDimRow {
    venue_id: i32,
    name: String,
    quality_tier: u8,
}

fn ns(t: DateTime<Utc>) -> i64 {
    t.timestamp_nanos_opt().unwrap_or(0)
}

impl PgIdentityService {
    #[must_use]
    pub fn new(pg: PgPool, ch_url: impl Into<String>) -> Arc<Self> {
        Arc::new(Self { pg, ch_url: ch_url.into(), cache: RwLock::new(HashMap::new()), sources: RwLock::new(HashMap::new()) })
    }

    async fn venue_id(&self, venue: &str) -> Result<VenueKey, IdentityError> {
        let id: Option<i32> = sqlx::query_scalar("SELECT venue_id FROM dataplane.venue WHERE name = $1")
            .bind(venue)
            .fetch_optional(&self.pg)
            .await?;
        id.map(VenueKey).ok_or_else(|| IdentityError::UnknownVenue(venue.to_string()))
    }

    /// Replicate identity dimensions to ClickHouse. Idempotent.
    ///
    /// # Errors
    /// Backend failures.
    pub async fn sync_dims(&self) -> Result<usize, IdentityError> {
        let symbols: Vec<SymbolFactRow> = sqlx::query_as(
            "SELECT instrument_id, venue_id, symbol, valid_from, valid_to, knowledge_time FROM dataplane.instrument_symbol",
        )
        .fetch_all(&self.pg)
        .await?;
        let venues: Vec<(i32, String, i16)> = sqlx::query_as("SELECT venue_id, name, quality_tier FROM dataplane.venue").fetch_all(&self.pg).await?;
        let ch = crate::clickhouse::connect(&self.ch_url);
        let mut ins = ch.insert("venue_dim").map_err(|e| IdentityError::Ch(e.to_string()))?;
        for (venue_id, name, tier) in venues {
            ins.write(&VenueDimRow { venue_id, name, quality_tier: u8::try_from(tier).unwrap_or(3) }).await.map_err(|e| IdentityError::Ch(e.to_string()))?;
        }
        ins.end().await.map_err(|e| IdentityError::Ch(e.to_string()))?;
        let n = symbols.len();
        let mut ins = ch.insert("instrument_symbol_dim").map_err(|e| IdentityError::Ch(e.to_string()))?;
        for (instrument_id, venue_id, symbol, from, to, kt) in symbols {
            ins.write(&SymbolDimRow { instrument_id, venue_id, symbol, valid_from: ns(from), valid_to: to.map(ns), knowledge_time: ns(kt) })
                .await
                .map_err(|e| IdentityError::Ch(e.to_string()))?;
        }
        ins.end().await.map_err(|e| IdentityError::Ch(e.to_string()))?;
        Ok(n)
    }
}

#[async_trait::async_trait]
impl IdentityResolver for PgIdentityService {
    async fn ensure_instrument(&self, venue: &str, symbol: &str, asset_class: AssetClass, at: DateTime<Utc>) -> Result<(InstrumentKey, VenueKey), IdentityError> {
        let key = (venue.to_string(), symbol.to_string());
        if let Some(hit) = self.cache.read().await.get(&key) {
            return Ok(*hit);
        }
        let venue_id = self.venue_id(venue).await?;
        let mut tx = self.pg.begin().await?;
        // Serialize allocation for this pair so two writers cannot mint two ids.
        sqlx::query("SELECT pg_advisory_xact_lock(hashtext('identity:' || $1 || ':' || $2))")
            .bind(venue)
            .bind(symbol)
            .execute(&mut *tx)
            .await?;
        let existing: Option<i64> = sqlx::query_scalar("SELECT dataplane.resolve_symbol($1, $2, $3, 'infinity'::timestamptz)")
            .bind(venue_id.0)
            .bind(symbol)
            .bind(at)
            .fetch_one(&mut *tx)
            .await?;
        let (id, created) = if let Some(id) = existing {
            (id, false)
        } else {
            let id: i64 = sqlx::query_scalar(
                "INSERT INTO dataplane.instrument (asset_class, first_seen, static_attrs) VALUES ($1, $2, '{}'::jsonb) RETURNING instrument_id",
            )
            .bind(asset_class.as_str())
            .bind(at)
            .fetch_one(&mut *tx)
            .await?;
            // An auto-allocated mapping has no recorded start: when the symbol began
            // denoting this instrument, and when that became known, were never
            // observed. Both take the dataplane sentinel and the row is flagged, so
            // history collected later (gap fills run backwards) still resolves
            // (CLAUDE.md §6). Real symbol changes arrive as corporate actions.
            sqlx::query(
                "INSERT INTO dataplane.instrument_symbol (instrument_id, venue_id, symbol, valid_from, knowledge_time, backfilled_knowledge_time)
                 VALUES ($1, $2, $3, '2000-01-01T00:00:00Z', '2000-01-01T00:00:00Z', TRUE)",
            )
            .bind(id)
            .bind(venue_id.0)
            .bind(symbol)
            .execute(&mut *tx)
            .await?;
            (id, true)
        };
        tx.commit().await?;
        if created {
            self.sync_dims().await?;
        }
        let out = (InstrumentKey(id), venue_id);
        self.cache.write().await.insert(key, out);
        Ok(out)
    }

    async fn source(&self, name: &str) -> Result<SourceInfo, IdentityError> {
        if let Some(s) = self.sources.read().await.get(name) {
            return Ok(s.clone());
        }
        let row: Option<(i32, i64, String)> =
            sqlx::query_as("SELECT source_id, declared_vendor_lag_ms, kind FROM dataplane.source WHERE name = $1")
                .bind(name)
                .fetch_optional(&self.pg)
                .await?;
        let (source_id, lag_ms, kind) = row.ok_or_else(|| IdentityError::UnknownSource(name.to_string()))?;
        let info = SourceInfo {
            source_id,
            declared_vendor_lag: std::time::Duration::from_millis(lag_ms.max(0) as u64),
            live: kind == "live_stream",
        };
        self.sources.write().await.insert(name.to_string(), info.clone());
        Ok(info)
    }
}

/// An in-memory resolver for tests and offline tools.
#[derive(Default)]
pub struct MemoryIdentity {
    next: std::sync::atomic::AtomicI64,
    map: std::sync::Mutex<HashMap<(String, String), (InstrumentKey, VenueKey)>>,
    venues: std::sync::Mutex<HashMap<String, VenueKey>>,
    sources: std::sync::Mutex<HashMap<String, SourceInfo>>,
}

impl MemoryIdentity {
    #[must_use]
    pub fn new() -> Self {
        Self { next: std::sync::atomic::AtomicI64::new(1000), ..Self::default() }
    }

    pub fn add_source(&self, name: &str, info: SourceInfo) {
        self.sources.lock().expect("poisoned").insert(name.into(), info);
    }

    /// Write this resolver's mappings to a (scratch) ClickHouse database's identity
    /// dimensions, with the sentinel validity auto-allocated identities get.
    ///
    /// # Errors
    /// Backend failures.
    pub async fn sync_dims(&self, ch_url: &str) -> Result<(), IdentityError> {
        let ch = crate::clickhouse::connect(ch_url);
        let venues: Vec<(String, VenueKey)> = self.venues.lock().expect("poisoned").iter().map(|(k, v)| (k.clone(), *v)).collect();
        let mut ins = ch.insert("venue_dim").map_err(|e| IdentityError::Ch(e.to_string()))?;
        for (name, v) in venues {
            ins.write(&VenueDimRow { venue_id: v.0, quality_tier: u8::try_from(v.0.min(3)).unwrap_or(3), name }).await.map_err(|e| IdentityError::Ch(e.to_string()))?;
        }
        ins.end().await.map_err(|e| IdentityError::Ch(e.to_string()))?;
        let sentinel = ns(DateTime::parse_from_rfc3339("2000-01-01T00:00:00Z").expect("sentinel").with_timezone(&Utc));
        let pairs: Vec<((String, String), (InstrumentKey, VenueKey))> = self.map.lock().expect("poisoned").iter().map(|(k, v)| (k.clone(), *v)).collect();
        let mut ins = ch.insert("instrument_symbol_dim").map_err(|e| IdentityError::Ch(e.to_string()))?;
        for ((_, symbol), (i, v)) in pairs {
            ins.write(&SymbolDimRow { instrument_id: i.0, venue_id: v.0, symbol, valid_from: sentinel, valid_to: None, knowledge_time: sentinel })
                .await
                .map_err(|e| IdentityError::Ch(e.to_string()))?;
        }
        ins.end().await.map_err(|e| IdentityError::Ch(e.to_string()))?;
        Ok(())
    }
}

#[async_trait::async_trait]
impl IdentityResolver for MemoryIdentity {
    async fn ensure_instrument(&self, venue: &str, symbol: &str, _asset_class: AssetClass, _at: DateTime<Utc>) -> Result<(InstrumentKey, VenueKey), IdentityError> {
        let mut venues = self.venues.lock().expect("poisoned");
        let n = venues.len() as i32 + 1;
        let v = *venues.entry(venue.into()).or_insert(VenueKey(n));
        let mut map = self.map.lock().expect("poisoned");
        let next = &self.next;
        Ok(*map
            .entry((venue.into(), symbol.into()))
            .or_insert_with(|| (InstrumentKey(next.fetch_add(1, std::sync::atomic::Ordering::SeqCst)), v)))
    }

    async fn source(&self, name: &str) -> Result<SourceInfo, IdentityError> {
        self.sources.lock().expect("poisoned").get(name).cloned().ok_or_else(|| IdentityError::UnknownSource(name.into()))
    }
}

static INSTALLED: std::sync::OnceLock<Arc<dyn IdentityResolver>> = std::sync::OnceLock::new();

/// Install the process-wide resolver bar writers use. Set once at boot; a second
/// install is ignored so a running writer can never switch identity sources.
pub fn install(resolver: Arc<dyn IdentityResolver>) {
    let _ = INSTALLED.set(resolver);
}

/// The installed resolver, if any. Writers refuse to run without one.
#[must_use]
pub fn installed() -> Option<Arc<dyn IdentityResolver>> {
    INSTALLED.get().cloned()
}
