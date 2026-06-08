//! TODO(Phase 1): venue connectors — normalize raw WS messages into typed EventEnvelopes.
//! Each venue is built deliberately differently to prove the abstraction.
pub mod normalizer;
pub mod reconnect;
pub mod gap;
pub mod crypto;
pub mod equity;
