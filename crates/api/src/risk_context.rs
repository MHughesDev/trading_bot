//! The one place a real [`risk::GateContext`] is built.
//!
//! Both order doors — `POST /api/orders` and the hot path's stage 4 — used to
//! assemble their own context, and the hot path's was eight hardcoded
//! constants.  They now call the same provider, so a limit that fires for a
//! manual order fires for a strategy order, and an input nobody can measure
//! refuses at both doors identically.
//!
//! ## Which fields are measured, and which are not
//!
//! Measured, from live state:
//!
//! * `current_position` — the paper account's net position in the instrument.
//! * `market_price` — the latest mark on the pipeline's mark board.
//! * `tick_size` / `lot_size` — the venue's published increments, from
//!   `instruments` (see [`crate::instrument_meta`]).
//! * `daily_loss_usd` — today's realized P&L net of fees, plus open unrealized
//!   P&L, from the paper account.
//! * `instrument_active`, `is_in_session`, `halt_policy` — from instrument
//!   metadata and the venue calendar.
//! * order-rate counts — kept by the gate itself, scoped per door.
//!
//! **Not** fully measured, and named here so it is not mistaken for enforced:
//!
//! * `event_trust_tier` is the *instrument's venue* tier, not the trust tier of
//!   the specific `EventEnvelope` that triggered the intent (COMP-002 §3.2
//!   check 8 asks for the latter).  The envelope's tier is not plumbed through
//!   the ring pipeline.  The instrument tier is a real, measured property and
//!   strictly better than the constant it replaces, but a per-event downgrade
//!   — the case the trust gate exists for — would not be caught.
//! * `is_halted` is always `false`: the platform does not subscribe to exchange
//!   halt feeds, so an intra-session halt is not seen.  Session boundaries are.

use std::sync::Arc;

use rust_decimal::Decimal;

use domain::{order::OrderIntent, RiskRejection, TrustTier};
use execution::paper::PaperTradingEngine;
use reconciliation::freshness::is_within_trading_hours;
use risk::{
    GateContext, InstrumentLimits, Measured, RateKey, RateSource, RiskContextProvider,
};

use crate::instrument_meta::InstrumentMetaCache;

/// Builds gate contexts from paper-account state plus instrument metadata.
pub struct PaperRiskContextProvider {
    paper_engine: Arc<PaperTradingEngine>,
    meta: Arc<InstrumentMetaCache>,
}

impl PaperRiskContextProvider {
    #[must_use]
    pub fn new(paper_engine: Arc<PaperTradingEngine>, meta: Arc<InstrumentMetaCache>) -> Self {
        Self { paper_engine, meta }
    }

    /// The rate-limit scope for an intent: per strategy instance for automated
    /// orders, per instrument for manual ones.  A strategy therefore cannot get
    /// a second budget by also placing orders by hand, and vice versa.
    fn rate_key(intent: &OrderIntent) -> RateKey {
        match &intent.strategy_id {
            Some(sid) => RateKey::strategy(sid, &intent.instrument_id),
            None => RateKey::manual(&intent.instrument_id),
        }
    }
}

impl RiskContextProvider for PaperRiskContextProvider {
    fn instrument_limits(&self, instrument_id: &str) -> Measured<InstrumentLimits> {
        match self.meta.get(instrument_id) {
            Some(m) => Measured::Yes(InstrumentLimits {
                tick_size: m.tick_size,
                lot_size: m.lot_size,
            }),
            None => Measured::Unavailable,
        }
    }

    fn context_for(&self, intent: &OrderIntent) -> Result<GateContext, RiskRejection> {
        let instrument_id = intent.instrument_id.as_str();

        // Which internal account does this instrument trade against?  Without
        // one there is no position, no equity and no mark — the pipeline that
        // would supply them is not running.
        let Some(asset_class) = self.paper_engine.asset_class_of(instrument_id) else {
            return Err(RiskRejection::MarketDataUnavailable {
                instrument_id: instrument_id.to_owned(),
            });
        };

        let meta = self.meta.get(instrument_id);

        let current_position = self
            .paper_engine
            .positions(asset_class)
            .into_iter()
            .find(|p| p.instrument_id == instrument_id)
            .map(|p| p.quantity)
            .unwrap_or(Decimal::ZERO);

        let market_price = Measured::from_option(self.paper_engine.mark(instrument_id));

        let instrument_limits = match &meta {
            Some(m) => Measured::Yes(InstrumentLimits {
                tick_size: m.tick_size,
                lot_size: m.lot_size,
            }),
            None => Measured::Unavailable,
        };

        // Today's realized P&L net of fees plus open unrealized P&L, as a loss.
        let daily_loss_usd = Measured::Yes(self.paper_engine.daily_loss_usd(asset_class));

        let (instrument_active, is_in_session, halt_policy, venue_trust) = match &meta {
            Some(m) => (
                m.active,
                is_within_trading_hours(chrono::Utc::now(), &m.trading_hours),
                m.halt_policy.clone(),
                m.trust_tier,
            ),
            // No metadata: the values below are never reached, because
            // `instrument_limits` is `Unavailable` and the gate refuses first.
            // They are written as the strictest possible values anyway, so a
            // future reordering of the checks cannot turn this into a pass.
            None => (
                false,
                false,
                domain::instrument::HaltPolicy::Haltable,
                TrustTier::SocialDerived,
            ),
        };

        // The strategy's declared minimum trust tier, from the definition that
        // is actually running.  A manual order declares none.
        let strategy_min_trust_tier = intent
            .strategy_id
            .as_deref()
            .and_then(|sid| live_strategy_min_tier(instrument_id, sid))
            .unwrap_or(TrustTier::SocialDerived);

        // Per-strategy risk overrides are tighten-only and come from the same
        // running definition.
        let risk_overrides = intent
            .strategy_id
            .as_deref()
            .and_then(|sid| live_strategy_overrides(instrument_id, sid))
            .unwrap_or_default();

        Ok(GateContext {
            // The two the monitor scopes on. A manual order names no strategy;
            // the venue comes from the instrument's metadata, and is `None`
            // when that metadata is missing — in which case `instrument_limits`
            // is `Unavailable` and the gate refuses before the scope matters.
            strategy_id: intent.strategy_id.clone(),
            venue_id: meta.as_ref().map(|m| m.venue_id.clone()),
            current_position,
            market_price,
            instrument_limits,
            daily_loss_usd,
            event_trust_tier: venue_trust,
            strategy_min_trust_tier,
            risk_overrides,
            instrument_active,
            rate: RateSource::Tracked(Self::rate_key(intent)),
            is_in_session,
            is_halted: false,
            halt_policy,
        })
    }
}

/// The `min_trust_tier` of the live definition that emitted this intent.
fn live_strategy_min_tier(instrument_id: &str, strategy_id: &str) -> Option<TrustTier> {
    strategy_runtime::live::global()
        .snapshot_for(instrument_id)
        .into_iter()
        .find(|s| s.definition.strategy_id == strategy_id)
        .map(|s| s.definition.min_trust_tier)
}

/// The tighten-only `risk_overrides` of the live definition that emitted this
/// intent.
fn live_strategy_overrides(
    instrument_id: &str,
    strategy_id: &str,
) -> Option<domain::strategy_def::risk_overrides::RiskOverrides> {
    strategy_runtime::live::global()
        .snapshot_for(instrument_id)
        .into_iter()
        .find(|s| s.definition.strategy_id == strategy_id)
        .map(|s| s.definition.risk_overrides.clone())
}
