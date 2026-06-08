//! TODO(Phase 3): intentionally lossy, frontend-shaped live views via WebSocket/SSE.
//! Never the canonical stream — strategy runtime NEVER reads from here.
pub mod shaping;
pub mod snapshot;
pub mod subscriptions;
pub mod throttle;
pub mod transport;
