//! Discovery tools: `list_lanes` and `list_instruments`.
//!
//! Returns static metadata about available data lanes and instruments.
//! In production these would query the platform; in Phase 5 they return
//! representative static data so an agent can reason about what is available.

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct LaneInfo {
    pub lane: String,
    pub description: String,
    pub example_instruments: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct InstrumentInfo {
    pub instrument_id: String,
    pub asset_class: String,
    pub venue_id: String,
    pub tick_size: String,
    pub trust_tier: String,
}

/// `list_lanes` — return available data lanes.
pub fn list_lanes() -> Vec<LaneInfo> {
    vec![
        LaneInfo {
            lane: "market.bars.1m".into(),
            description: "1-minute OHLCV bars for a given instrument".into(),
            example_instruments: vec!["BTC-USDT".into(), "ETH-USDT".into()],
        },
        LaneInfo {
            lane: "features.technical".into(),
            description: "Technical indicator feature values (EMA, RSI, …)".into(),
            example_instruments: vec!["BTC-USDT".into(), "ETH-USDT".into()],
        },
    ]
}

/// `list_instruments` — return instruments, optionally filtered by `asset_class`.
pub fn list_instruments(asset_class: Option<&str>) -> Vec<InstrumentInfo> {
    let all = vec![
        InstrumentInfo {
            instrument_id: "BTC-USDT".into(),
            asset_class: "crypto_spot_cex".into(),
            venue_id: "binance".into(),
            tick_size: "0.01".into(),
            trust_tier: "centralized_exchange".into(),
        },
        InstrumentInfo {
            instrument_id: "ETH-USDT".into(),
            asset_class: "crypto_spot_cex".into(),
            venue_id: "binance".into(),
            tick_size: "0.01".into(),
            trust_tier: "centralized_exchange".into(),
        },
        InstrumentInfo {
            instrument_id: "SOL-USDT".into(),
            asset_class: "crypto_spot_cex".into(),
            venue_id: "binance".into(),
            tick_size: "0.001".into(),
            trust_tier: "centralized_exchange".into(),
        },
    ];

    match asset_class {
        None => all,
        Some(ac) => all.into_iter().filter(|i| i.asset_class == ac).collect(),
    }
}
