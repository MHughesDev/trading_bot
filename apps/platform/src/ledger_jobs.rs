//! Ledger fixation jobs (SPEC §4.6, INV-19).
//!
//! * **Anchor** — once per UTC day, per tenant, the tips of the trial and decision
//!   chains as of the end of the previous day are signed and written to WORM.
//! * **Verify** — every chain, every event chain, every anchor's heads, signature and
//!   WORM copy are recomputed; each pass is recorded, and a failure is logged at
//!   error level.
//!
//! Neither job has an off switch. The signing key and the WORM location are
//! REQUIRED configuration with no default: the platform does not start without them.

use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use ledger::anchor::{AnchorSigner, FsWorm, WormStore};
use ledger::pg::PgTrialLedger;
use tracing::{error, info, warn};

pub struct LedgerFixation {
    ledger: PgTrialLedger,
    signer: AnchorSigner,
    keys: Vec<ed25519_dalek::VerifyingKey>,
    worm: Arc<dyn WormStore>,
}

impl LedgerFixation {
    /// Reads `PLATFORM_LEDGER_SIGNING_KEY` (64 hex chars, ed25519 seed),
    /// `PLATFORM_WORM_DIR`, and optionally `PLATFORM_LEDGER_RETIRED_KEYS` (comma-
    /// separated hex public keys still accepted when verifying older anchors).
    ///
    /// # Errors
    /// Missing or malformed configuration.
    pub fn from_env(pool: sqlx::PgPool) -> anyhow::Result<Self> {
        let seed = std::env::var("PLATFORM_LEDGER_SIGNING_KEY")
            .context("PLATFORM_LEDGER_SIGNING_KEY is REQUIRED (ed25519 seed, 64 hex chars): ledger anchors must be signed")?;
        let signer = AnchorSigner::from_hex_seed(&seed).context("PLATFORM_LEDGER_SIGNING_KEY")?;
        let worm_dir = std::env::var("PLATFORM_WORM_DIR")
            .context("PLATFORM_WORM_DIR is REQUIRED: ledger anchors are written to a write-once store")?;
        let mut keys = vec![signer.verifying_key()];
        for k in std::env::var("PLATFORM_LEDGER_RETIRED_KEYS").unwrap_or_default().split(',').map(str::trim).filter(|k| !k.is_empty()) {
            let bytes: [u8; 32] = hex::decode(k).ok().and_then(|b| b.try_into().ok()).context("PLATFORM_LEDGER_RETIRED_KEYS: 64 hex chars each")?;
            keys.push(ed25519_dalek::VerifyingKey::from_bytes(&bytes).context("PLATFORM_LEDGER_RETIRED_KEYS: invalid public key")?);
        }
        Ok(Self { ledger: PgTrialLedger::new(pool), signer, keys, worm: Arc::new(FsWorm::new(worm_dir)) })
    }

    /// Anchor every tenant for every day up to yesterday that is not yet anchored.
    /// Today is never anchored: its chain is still growing.
    pub async fn anchor_once(&self) {
        let yesterday = chrono::Utc::now().date_naive() - chrono::Days::new(1);
        let tenants = match self.ledger.tenants_async().await {
            Ok(t) => t,
            Err(e) => return warn!(error = %e, "ledger anchor: could not list tenants"),
        };
        for tenant in tenants {
            match self.ledger.write_anchor_async(&tenant, yesterday, &self.signer, self.worm.as_ref()).await {
                Ok(Some(id)) => info!(%tenant, date = %yesterday, anchor_id = %id, key_id = self.signer.key_id(), "ledger anchored"),
                Ok(None) => {}
                Err(ledger::anchor::AnchorError::Broken(b)) => {
                    error!(%tenant, date = %yesterday, chain_break = %b, "LEDGER CHAIN BROKEN — refusing to anchor");
                }
                Err(e) => warn!(%tenant, error = %e, "ledger anchor failed"),
            }
        }
    }

    pub async fn verify_once(&self) {
        let tenants = match self.ledger.tenants_async().await {
            Ok(t) => t,
            Err(e) => return warn!(error = %e, "ledger verify: could not list tenants"),
        };
        for tenant in tenants {
            match self.ledger.verify_and_record_async(&tenant, &self.keys, Some(self.worm.as_ref())).await {
                Ok(r) if r.ok() => info!(%tenant, trials = r.trials, events = r.events, decisions = r.decisions, anchors = r.anchors, "ledger verified"),
                Ok(r) => error!(%tenant, failures = ?r.failures, "LEDGER VERIFICATION FAILED"),
                Err(e) => warn!(%tenant, error = %e, "ledger verification could not run"),
            }
        }
    }

    /// Anchor hourly (idempotent per day) and verify every six hours.
    pub fn spawn(self: Arc<Self>) {
        let anchor = Arc::clone(&self);
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(3600));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tick.tick().await;
                anchor.anchor_once().await;
            }
        });
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(6 * 3600));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tick.tick().await;
                self.verify_once().await;
            }
        });
    }
}
