//! Intentionally-lossy, frontend-shaped live views via WebSocket.
//! The strategy runtime must NEVER consume this feed — it is a separate consumer view.
pub mod nats_ws;
pub mod shaping;
pub mod snapshot;
pub mod subscriptions;
pub mod throttle;
pub mod transport;

pub use subscriptions::{Subscription, SubscriptionError, SubscriptionRegistry};
pub use transport::{ClientMessage, SubSpec, WsOutMessage};
