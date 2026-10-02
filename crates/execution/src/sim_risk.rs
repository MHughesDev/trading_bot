//! The risk gate in front of a backtest (Set U U-2.4, ADR-0046).
//!
//! Backtests have never been gated: a strategy could hold ten times the position
//! cap in simulation and only meet the cap in paper, where the "same" strategy
//! then traded differently from the one that was evaluated. [`SimulatedRisk`]
//! runs the very same [`RiskGate::check`] — same limits, same order of checks,
//! same rejections — against a context built from the simulated run: its
//! account, its clock, and a kill switch that belongs to the run alone.
//!
//! A refusal is a first-class event. Every one is kept, with the check that
//! failed and the simulated instant, so a backtest report can say "these 14
//! orders were refused by the position cap" instead of quietly trading less.
//!
//! A daily-loss breach halts the run for the rest of the simulated day: once the
//! gate refuses on daily loss, every order that day is refused, even if the
//! loss recovers intrabar. The next simulated day lifts it.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use domain::{instrument::AssetClass, order::OrderIntent, RiskRejection};
use risk::{
    GateContext, GlobalRiskLimits, InstrumentLimits, KillSwitch, RiskGate, SimulatedInputs,
    SimulatedRiskState,
};
use uuid::Uuid;

use crate::paper::PaperTradeError;
use crate::sim_venue::SimulatedVenue;
use crate::venue_port::Venue;

/// One order the simulated gate refused.
#[derive(Clone, Debug)]
pub struct GateRejection {
    pub key: Uuid,
    pub instrument_id: String,
    pub at: DateTime<Utc>,
    pub rejection: RiskRejection,
}

/// Why a gated simulated submission did not reach the venue.
#[derive(Debug)]
pub enum Refusal {
    /// The risk gate said no.
    Risk(RiskRejection),
    /// The venue refused it outright.
    Venue(PaperTradeError),
}

/// The run's gate: the production [`RiskGate`] with simulated inputs.
pub struct SimulatedRisk {
    gate: RiskGate,
    kill_switch: Arc<KillSwitch>,
    state: Mutex<SimulatedRiskState>,
    instruments: BTreeMap<String, InstrumentLimits>,
    strategy_id: Option<String>,
    rejections: Mutex<Vec<GateRejection>>,
}

impl SimulatedRisk {
    /// A gate for one run. `instruments` is the tick/lot metadata the run's
    /// universe carries; an instrument missing from it is refused, as live
    /// refuses an instrument with no metadata.
    #[must_use]
    pub fn new(
        limits: GlobalRiskLimits,
        instruments: BTreeMap<String, InstrumentLimits>,
        strategy_id: Option<String>,
    ) -> Self {
        let kill_switch = Arc::new(KillSwitch::new(false));
        Self {
            gate: RiskGate::new(limits, Arc::clone(&kill_switch)),
            kill_switch,
            state: Mutex::new(SimulatedRiskState::new()),
            instruments,
            strategy_id,
            rejections: Mutex::new(Vec::new()),
        }
    }

    /// The run's own kill switch. Tripping it stops this run and nothing else.
    #[must_use]
    pub const fn kill_switch(&self) -> &Arc<KillSwitch> {
        &self.kill_switch
    }

    /// Every refusal so far, in order.
    #[must_use]
    pub fn rejections(&self) -> Vec<GateRejection> {
        self.rejections.lock().expect("sim risk rejections").clone()
    }

    /// Gate `intent` against the run's state, then hand it to `venue`.
    ///
    /// # Errors
    /// The gate's refusal (recorded in [`Self::rejections`]) or the venue's.
    pub fn submit(
        &self,
        venue: &SimulatedVenue,
        asset_class: AssetClass,
        intent: &OrderIntent,
    ) -> Result<Uuid, Refusal> {
        let now = venue.now();
        let mut state = self.state.lock().expect("sim risk state");
        if state.halted(now) {
            let r = RiskRejection::StrategyHalted {
                strategy_id: self.strategy_id.clone().unwrap_or_default(),
                reason: "daily loss limit breached earlier this simulated day".into(),
            };
            return Err(self.refuse(intent, now, r));
        }
        let engine = venue.engine();
        let inputs = SimulatedInputs {
            position: venue
                .positions(asset_class)
                .iter()
                .find(|p| p.instrument_id == intent.instrument_id)
                .map_or(rust_decimal::Decimal::ZERO, |p| p.quantity),
            mark: engine.mark(&intent.instrument_id),
            limits: self.instruments.get(&intent.instrument_id).copied(),
            daily_loss_usd: engine.daily_loss_usd(asset_class),
            strategy_id: self.strategy_id.clone(),
        };
        let ctx = GateContext::simulated(inputs, state.rate_at(now));
        match self.gate.check(intent.clone(), &ctx) {
            Ok(approved) => {
                state.record(now);
                drop(state);
                venue
                    .submit(asset_class, &approved.intent)
                    .map_err(Refusal::Venue)
            }
            Err(r) => {
                if matches!(r, RiskRejection::DailyLossLimitExceeded { .. }) {
                    state.halt_for_day(now);
                }
                drop(state);
                Err(self.refuse(intent, now, r))
            }
        }
    }

    fn refuse(&self, intent: &OrderIntent, at: DateTime<Utc>, rejection: RiskRejection) -> Refusal {
        self.rejections
            .lock()
            .expect("sim risk rejections")
            .push(GateRejection {
                key: intent.idempotency_key,
                instrument_id: intent.instrument_id.clone(),
                at,
                rejection: rejection.clone(),
            });
        Refusal::Risk(rejection)
    }
}
