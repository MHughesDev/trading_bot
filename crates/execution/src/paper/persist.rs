//! Durable paper accounts (Set E audit item #5, migration 0153).
//!
//! The paper engine is the paper account (ADR-0048), and until this module it
//! lived only in memory: a restart silently reset every account's cash,
//! positions, realized P&L, the day's realized tally (which the risk gate's
//! daily-loss limit reads) and the transaction ledger to the seeded opening
//! balance.
//!
//! Two calls, both made by the binary at boot:
//!
//! 1. [`restore`] — before the engine takes an order, load every stored
//!    account. A row the binary cannot read is an **error**, not a reseed: a
//!    silently reseeded account is the exact defect being removed.
//! 2. [`spawn_saver`] — once a second, write each account that was touched
//!    since the last pass *and* whose state actually changed.
//!
//! The loss window is therefore at most one save interval. Resting (unfilled)
//! orders are not restored: they hold no reservation in the account state, and
//! the `orders` table keeps their record.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use domain::instrument::AssetClass;
use sqlx::PgPool;

use super::account::PaperAccountState;
use super::engine::PaperTradingEngine;
use super::policy::ALL_ASSET_CLASSES;

/// The shape version of [`PaperAccountState`]'s JSON. Bump it with any
/// incompatible change; [`restore`] refuses a version it does not know.
pub const STATE_VERSION: i32 = 1;

/// How often the saver looks for changed accounts.
pub const SAVE_EVERY: Duration = Duration::from_secs(1);

#[derive(Debug, thiserror::Error)]
pub enum PersistError {
    #[error("paper state database error: {0}")]
    Db(#[from] sqlx::Error),
    #[error("paper state for {asset_class} is version {found}; this binary reads version {STATE_VERSION}")]
    UnknownVersion { asset_class: String, found: i32 },
    #[error("paper state for {asset_class} is unreadable: {detail}")]
    Unreadable { asset_class: String, detail: String },
}

fn class_named(name: &str) -> Option<AssetClass> {
    ALL_ASSET_CLASSES.into_iter().find(|ac| ac.as_str() == name)
}

/// Load every stored account into `engine`. Returns how many were restored.
///
/// # Errors
/// A database error, a row of an unknown version, or a row that does not
/// deserialize. The caller must not start trading on an error.
pub async fn restore(engine: &PaperTradingEngine, pg: &PgPool) -> Result<usize, PersistError> {
    let rows: Vec<(String, i32, serde_json::Value)> =
        sqlx::query_as("SELECT asset_class, state_version, state FROM paper_account_state")
            .fetch_all(pg)
            .await?;
    let mut restored = 0;
    for (name, version, state) in rows {
        if version != STATE_VERSION {
            return Err(PersistError::UnknownVersion {
                asset_class: name,
                found: version,
            });
        }
        let Some(asset_class) = class_named(&name) else {
            // An account class this binary no longer has: nothing to load it
            // into. Logged, not fatal — no live account is being discarded.
            tracing::warn!(asset_class = %name, "stored paper state for an unknown asset class — ignored");
            continue;
        };
        let state: PaperAccountState =
            serde_json::from_value(state).map_err(|e| PersistError::Unreadable {
                asset_class: name.clone(),
                detail: e.to_string(),
            })?;
        engine
            .restore_account(asset_class, state)
            .map_err(|detail| PersistError::Unreadable {
                asset_class: name.clone(),
                detail,
            })?;
        restored += 1;
    }
    // Each deployment's own account (Set U U-4.7, migration 0188): the same
    // state shape and version, the same refusal of a row it cannot read.
    let rows: Vec<(uuid::Uuid, String, i32, serde_json::Value)> = sqlx::query_as(
        "SELECT deployment_id, asset_class, state_version, state FROM paper_deployment_account_state",
    )
    .fetch_all(pg)
    .await?;
    let mut deployments = 0;
    for (id, name, version, state) in rows {
        let label = format!("{name} (deployment {id})");
        if version != STATE_VERSION {
            return Err(PersistError::UnknownVersion {
                asset_class: label,
                found: version,
            });
        }
        let Some(asset_class) = class_named(&name) else {
            return Err(PersistError::Unreadable {
                asset_class: label,
                detail: "an asset class this binary does not have".into(),
            });
        };
        let state: PaperAccountState =
            serde_json::from_value(state).map_err(|e| PersistError::Unreadable {
                asset_class: label.clone(),
                detail: e.to_string(),
            })?;
        engine
            .restore_deployment_account(id, asset_class, state)
            .map_err(|detail| PersistError::Unreadable {
                asset_class: label,
                detail,
            })?;
        deployments += 1;
    }
    tracing::info!(
        accounts = restored,
        deployments,
        "paper account state restored"
    );
    Ok(restored + deployments)
}

/// One saver pass over the deployment accounts (U-4.7): write each one that
/// changed since the last pass. Returns how many rows were written.
///
/// # Errors
/// The first database error; that account is marked dirty again.
pub async fn save_dirty_deployments(
    engine: &PaperTradingEngine,
    pg: &PgPool,
    last: &mut HashMap<uuid::Uuid, serde_json::Value>,
) -> Result<usize, PersistError> {
    let mut written = 0;
    for (id, _) in engine.deployment_accounts() {
        if !engine.take_deployment_dirty(id) {
            continue;
        }
        let Some((ac, state)) = engine.export_deployment_account(id) else {
            continue;
        };
        let json = serde_json::to_value(&state).map_err(|e| PersistError::Unreadable {
            asset_class: format!("{} (deployment {id})", ac.as_str()),
            detail: e.to_string(),
        })?;
        if last.get(&id) == Some(&json) {
            continue;
        }
        let saved = sqlx::query(
            "INSERT INTO paper_deployment_account_state \
               (deployment_id, asset_class, state_version, state, saved_at) \
             VALUES ($1, $2, $3, $4, now()) \
             ON CONFLICT (deployment_id) DO UPDATE SET \
               state_version = EXCLUDED.state_version, state = EXCLUDED.state, saved_at = now()",
        )
        .bind(id)
        .bind(ac.as_str())
        .bind(STATE_VERSION)
        .bind(&json)
        .execute(pg)
        .await;
        match saved {
            Ok(_) => {
                last.insert(id, json);
                written += 1;
            }
            Err(e) => {
                // Not saved: exporting re-locks it, which marks it dirty again.
                let _ = engine.export_deployment_account(id);
                return Err(e.into());
            }
        }
    }
    Ok(written)
}

/// Write one account's state.
///
/// # Errors
/// A database or serialization error.
pub async fn save_account(
    engine: &PaperTradingEngine,
    pg: &PgPool,
    asset_class: AssetClass,
    generation: i64,
) -> Result<serde_json::Value, PersistError> {
    let state = engine.export_account(asset_class);
    let json = serde_json::to_value(&state).map_err(|e| PersistError::Unreadable {
        asset_class: asset_class.as_str().to_owned(),
        detail: e.to_string(),
    })?;
    sqlx::query(
        "INSERT INTO paper_account_state (asset_class, state_version, state, generation, saved_at) \
         VALUES ($1, $2, $3, $4, now()) \
         ON CONFLICT (asset_class) DO UPDATE SET \
           state_version = EXCLUDED.state_version, state = EXCLUDED.state, \
           generation = EXCLUDED.generation, saved_at = now()",
    )
    .bind(asset_class.as_str())
    .bind(STATE_VERSION)
    .bind(&json)
    .bind(generation)
    .execute(pg)
    .await?;
    Ok(json)
}

/// One saver pass: write every account touched since the last pass whose state
/// differs from what was last written. Returns how many rows were written.
///
/// `last` is the saver's memory of what it wrote, so an account that was only
/// *read* (a dashboard snapshot also takes the lock) is not rewritten.
///
/// # Errors
/// The first database error. The account's dirty flag is restored so the next
/// pass retries it.
pub async fn save_dirty(
    engine: &PaperTradingEngine,
    pg: &PgPool,
    last: &mut HashMap<AssetClass, serde_json::Value>,
    generation: &mut i64,
) -> Result<usize, PersistError> {
    let mut written = 0;
    for ac in ALL_ASSET_CLASSES {
        if !engine.take_dirty(ac) {
            continue;
        }
        let state = serde_json::to_value(engine.export_account(ac)).map_err(|e| {
            PersistError::Unreadable {
                asset_class: ac.as_str().to_owned(),
                detail: e.to_string(),
            }
        })?;
        if last.get(&ac) == Some(&state) {
            continue;
        }
        *generation += 1;
        match save_account(engine, pg, ac, *generation).await {
            Ok(json) => {
                last.insert(ac, json);
                written += 1;
                observability::metrics::inc(
                    "paper_state_saves_total",
                    "Paper account state writes, by outcome.",
                    &[("outcome", "ok")],
                );
            }
            Err(e) => {
                observability::metrics::inc(
                    "paper_state_saves_total",
                    "Paper account state writes, by outcome.",
                    &[("outcome", "error")],
                );
                // Not saved: mark it dirty again (by locking it) so it is
                // retried rather than forgotten.
                let _ = engine.export_account(ac);
                return Err(e);
            }
        }
    }
    Ok(written)
}

/// Spawn the background saver. Holds the engine for the life of the process.
pub fn spawn_saver(engine: Arc<PaperTradingEngine>, pg: PgPool) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut last: HashMap<AssetClass, serde_json::Value> = HashMap::new();
        let mut last_deployments: HashMap<uuid::Uuid, serde_json::Value> = HashMap::new();
        let mut generation: i64 = 0;
        let mut tick = tokio::time::interval(SAVE_EVERY);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
            match save_dirty(&engine, &pg, &mut last, &mut generation).await {
                Ok(0) => {}
                Ok(n) => tracing::debug!(accounts = n, "paper account state saved"),
                Err(e) => tracing::error!(
                    error = %e,
                    "paper account state NOT saved — a restart now would lose recent paper fills"
                ),
            }
            match save_dirty_deployments(&engine, &pg, &mut last_deployments).await {
                Ok(0) => {}
                Ok(n) => tracing::debug!(accounts = n, "deployment paper accounts saved"),
                Err(e) => tracing::error!(
                    error = %e,
                    "deployment paper accounts NOT saved — a restart now would lose recent fills"
                ),
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_asset_class_round_trips_by_name() {
        for ac in ALL_ASSET_CLASSES {
            assert_eq!(class_named(ac.as_str()), Some(ac));
        }
    }
}
