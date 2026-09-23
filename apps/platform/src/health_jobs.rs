//! Hourly platform self-monitoring pass (SPEC §16.2; plan 5.3).
//!
//! The endpoint answers when someone asks. This asks on its own, because the
//! signals that matter most are the ones nobody thinks to check: an exploration
//! fraction that has quietly fallen below its floor, a leakage suite that stopped
//! running, a cited artifact that is no longer pinned. Each of those degrades
//! silently and none of them surfaces anywhere else.
//!
//! P1 breaches log at error, P2 at warn, and signals that could not be computed
//! log separately from both — a check that is broken is a different problem from
//! a check that failed, and treating them alike is how the broken one stays
//! broken.

use std::time::Duration;

use tracing::{error, info, warn};

pub fn spawn(pg: sqlx::PgPool, ch_url: String) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(3600));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tick.tick().await;
            run_once(&pg, &ch_url).await;
        }
    });
}

pub async fn run_once(pg: &sqlx::PgPool, ch_url: &str) {
    self_monitoring_pass(pg).await;
    silence_pass(pg, ch_url).await;
}

/// The per-tenant self-monitoring signals.
async fn self_monitoring_pass(pg: &sqlx::PgPool) {
    // Every tenant with a ledger. A tenant with no trials has nothing to report
    // on, and inventing a clean bill of health for one would be the exact failure
    // §16.2 is about.
    let led = ledger::pg::PgTrialLedger::new(pg.clone());
    let tenants = match led.tenants_async().await {
        Ok(t) => t,
        Err(e) => return warn!(error = %e, "platform health: could not list tenants"),
    };
    for tenant in tenants {
        match api::self_monitor::health(pg, &tenant).await {
            Ok(report) => {
                for s in report.alarms() {
                    match s.severity {
                        api::self_monitor::Severity::P1 => {
                            error!(%tenant, signal = s.id, spec = s.spec_ref, detail = %s.detail,
                                "P1: platform self-monitoring alarm");
                        }
                        api::self_monitor::Severity::P2 => {
                            warn!(%tenant, signal = s.id, spec = s.spec_ref, detail = %s.detail,
                                "platform self-monitoring alarm");
                        }
                    }
                }
                for s in report.unavailable() {
                    warn!(%tenant, signal = s.id, detail = %s.detail,
                        "platform self-monitoring: signal could not be computed");
                }
                if report.healthy() {
                    info!(summary = ?api::self_monitor::HealthSummary::from(&report),
                        "platform self-monitoring clean");
                }
            }
            Err(e) => warn!(%tenant, error = %e, "platform health pass failed"),
        }
    }
}

/// The silence alarm (Set P P-0.6): the caller
/// `storage::clickhouse::census`'s module docs name.
///
/// Not tenant-scoped — a collector that died is one fact about one process, not
/// one per tenant — and the hourly tick `spawn` already owns is the right cadence,
/// since the tightest deadline any registered series declares is three hours.
///
/// [`silence_report`] never returns `Err`: every failure, including a store it
/// could not read at all, comes back as a finding whose severity says so, and
/// `emit` logs alarms at error, unjudgeable feeds at warn and quiet-but-fine feeds
/// at info — the same three-way split [`self_monitoring_pass`] uses, for the same
/// reason.
///
/// Its own function, called after the tenant pass rather than appended inside it,
/// because that pass returns early when Postgres will not list tenants. Inside its
/// body, a database hiccup would silence the silence alarm — which is the one
/// failure this whole mechanism exists to make impossible.
///
/// [`silence_report`]: storage::clickhouse::census::silence_report
async fn silence_pass(pg: &sqlx::PgPool, ch_url: &str) {
    observability::silence::emit(
        &storage::clickhouse::census::silence_report(
            pg,
            ch_url,
            chrono::Utc::now(),
            std::num::NonZeroU32::new(
                storage::clickhouse::census::REGISTRY_DEFAULT_TOLERANCE_CADENCES,
            )
            .expect("the registry default is positive"),
        )
        .await,
    );
}
