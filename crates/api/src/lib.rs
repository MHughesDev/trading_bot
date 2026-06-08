//! TODO(Phase 1): axum REST routes + WebSocket upgrade + auth (control plane).
//! REST is the control plane, not the data plane.
pub mod state;
pub mod auth;
pub mod routes;
pub mod ws;
