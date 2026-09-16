//! The two loops (COMP-007 §1 P1, §4, §6).
//!
//! [`MonitorService`] is the one that runs in `tbot-monitor`: renew the lease,
//! evaluate, write events, trip. [`LeaseWatcher`] is the one that runs *inside* the
//! process being watched: read the lease, read the halted scopes, push both into
//! the in-memory mirror the order gate reads.
//!
//! They are deliberately different shapes. The service writes and the watcher only
//! reads, and the watcher's failure mode — cannot reach the database — closes the
//! gate rather than leaving it where it was.

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use risk::KillSwitch;

use crate::lease::{LeaseStatus, LeaseTerms};
use crate::store::MonitorStore;

/// The independent monitor process's loop.
pub struct MonitorService {
    store: MonitorStore,
    terms: LeaseTerms,
}

impl MonitorService {
    #[must_use]
    pub fn new(store: MonitorStore, terms: LeaseTerms) -> Self {
        Self { store, terms }
    }

    #[must_use]
    pub fn store(&self) -> &MonitorStore {
        &self.store
    }

    #[must_use]
    pub fn terms(&self) -> &LeaseTerms {
        &self.terms
    }

    /// One cadence: renew the lease and beat.
    ///
    /// Renewal comes first and its failure is loud. A monitor that keeps evaluating
    /// while it cannot renew is a monitor that believes it is protecting a platform
    /// which has already stopped listening to it.
    pub async fn tick(&self) -> Result<(), crate::store::StoreError> {
        self.store.renew_lease(&self.terms, Utc::now()).await?;
        self.store
            .heartbeat("tbot-monitor", &self.terms.holder, self.terms.cadence_ms)
            .await?;
        Ok(())
    }

    /// Run until cancelled.
    pub async fn run(self) {
        let interval = Duration::from_millis(
            u64::try_from(self.terms.renew_interval_ms()).unwrap_or(crate::DEFAULT_CADENCE_MS),
        );
        tracing::info!(
            holder = %self.terms.holder,
            cadence_ms = self.terms.cadence_ms,
            "tbot-monitor started"
        );
        loop {
            if let Err(e) = self.tick().await {
                // Not fatal, and not silent. The lease will lapse on its own and
                // the platform will stop; crashing here would only make the same
                // thing happen with less in the log.
                tracing::error!(error = %e, "monitor cadence failed");
            }
            tokio::time::sleep(interval).await;
        }
    }
}

/// Refreshes the order gate's in-memory mirror from the monitor's tables.
///
/// This runs in the API / strategy-runtime process. It is the only thing that
/// writes to [`KillSwitch`] in production, and it is why `is_active` can stay an
/// atomic read on the hot path while the authority for what it says lives in
/// another process.
pub struct LeaseWatcher {
    store: MonitorStore,
    kill_switch: Arc<KillSwitch>,
    cadence: Duration,
}

impl LeaseWatcher {
    /// Build a watcher and **arm** the lease gate.
    ///
    /// Arming happens in the constructor, before the first refresh, so there is no
    /// window in which the process is running with the gate open and no monitor
    /// seen. Construction is the moment the process commits to being watched.
    #[must_use]
    pub fn new(store: MonitorStore, kill_switch: Arc<KillSwitch>, cadence: Duration) -> Self {
        kill_switch.require_monitor_lease();
        Self {
            store,
            kill_switch,
            cadence,
        }
    }

    /// One refresh. Returns what the lease said.
    pub async fn refresh(&self) -> LeaseStatus {
        let status = self.store.lease_status(Utc::now()).await;
        self.kill_switch.observe_lease(status.permits_orders());

        match self.store.halted_scopes().await {
            Ok(scopes) => self.kill_switch.replace_halted(&scopes),
            Err(e) => {
                // The scoped view could not be read. The lease observation above
                // has already closed the gate if the lease is gone; if the lease
                // is valid but this query failed, leaving the previous scoped view
                // in place is the conservative choice — it can only be too
                // restrictive, never too permissive, because a scope that was
                // halted stays halted until a successful read says otherwise.
                tracing::error!(error = %e, "halted scopes unreadable — keeping the previous mirror");
            }
        }
        status
    }

    /// Run until cancelled.
    pub async fn run(self) {
        loop {
            let status = self.refresh().await;
            if !status.permits_orders() {
                tracing::warn!(?status, "monitor lease not valid — orders refused");
            }
            tokio::time::sleep(self.cadence).await;
        }
    }
}
