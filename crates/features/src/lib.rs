//! PURE technical indicator / feature computation.
//!
//! Same purity contract as `builders`: no I/O, same code live and in replay.
//!
//! TODO(Phase 4): implement EMA, RSI, and rolling-window primitives.
pub mod ema;
pub mod rsi;
pub mod window;
