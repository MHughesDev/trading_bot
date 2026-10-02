//! The process-wide pieces every order-path test in this binary shares.
//!
//! `risk::context::install` and `execution::journal::install` are `OnceLock`s —
//! first install wins — so every test here shares one provider, one journal
//! and one paper engine, and uses its own instrument to stay independent.

use std::str::FromStr;
use std::sync::{Arc, OnceLock};

use api::instrument_meta::{InstrumentMeta, InstrumentMetaCache};
use api::risk_context::PaperRiskContextProvider;
use domain::instrument::{AssetClass, HaltPolicy, TradingSchedule};
use domain::money::Price;
use domain::TrustTier;
use execution::journal::{self, memory::MemoryOrderJournal};
use execution::paper::PaperTradingEngine;
use rust_decimal::Decimal;

pub struct Env {
    pub paper: Arc<PaperTradingEngine>,
    pub meta: Arc<InstrumentMetaCache>,
}

static ENV: OnceLock<Env> = OnceLock::new();

pub fn env() -> &'static Env {
    ENV.get_or_init(|| {
        let paper = Arc::new(PaperTradingEngine::new());
        let meta = Arc::new(InstrumentMetaCache::new());

        assert!(
            risk::context::install(Arc::new(PaperRiskContextProvider::new(
                Arc::clone(&paper),
                Arc::clone(&meta),
            ))),
            "risk context provider must install"
        );
        assert!(
            journal::install(Arc::new(MemoryOrderJournal::new())),
            "order journal must install"
        );

        Env { paper, meta }
    })
}

/// Register an instrument the way a live pipeline and asset-init would: an
/// asset class, a mark, and the venue's published increments.
pub fn ready_instrument(instrument_id: &str, mark: &str) {
    let e = env();
    e.paper
        .register_instrument(instrument_id, AssetClass::CryptoSpotCex);
    e.paper
        .on_mark(instrument_id, Price::from_str(mark).unwrap());
    e.meta.put(InstrumentMeta {
        venue_id: "paper".into(),
        instrument_id: instrument_id.to_owned(),
        // Kraken's published increments for a USD spot pair.
        tick_size: Decimal::from_str("0.1").unwrap(),
        lot_size: Decimal::from_str("0.00000001").unwrap(),
        trading_hours: TradingSchedule::always_open(),
        halt_policy: HaltPolicy::NonHaltable,
        trust_tier: TrustTier::CentralizedExchange,
        active: true,
    });
}
