//! Wire types for the WebSocket protocol between panels and the UI gateway.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A single lane subscription spec sent by a panel.
#[derive(Debug, Clone, Deserialize)]
pub struct SubSpec {
    pub lane: String,
    pub instrument: String,
    /// For `ui.orderbook.snapshot`: number of price levels to include.
    pub depth: Option<u32>,
    /// For `ui.orderbook.snapshot`: maximum frames per second.
    pub max_fps: Option<u32>,
}

/// Message sent from a UI panel to the gateway over the WebSocket.
#[derive(Debug, Deserialize)]
pub struct ClientMessage {
    pub panel_id: String,
    /// Subscribe to these lanes; starts pipelines if needed.
    #[serde(default)]
    pub subscribe: Vec<SubSpec>,
    /// If `true`, remove all subscriptions for this panel.
    #[serde(default)]
    pub unsubscribe: bool,
}

/// Message sent from the gateway to a UI panel.
#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WsOutMessage {
    /// Subscription acknowledged.
    Subscribed {
        sub_id: Uuid,
        panel_id: String,
        lane: String,
        instrument: String,
    },
    /// A live data frame.
    Frame {
        sub_id: Uuid,
        lane: String,
        instrument: String,
        payload: serde_json::Value,
    },
    /// Keep-alive.
    Heartbeat { ts: String },
    /// Something went wrong.
    Error { code: String, message: String },
}
