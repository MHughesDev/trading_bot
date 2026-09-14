//! **Strategy research** — the inner loop of FEAT-003.
//!
//! * [`space`] — a typed [`SearchSpace`] built from a definition's
//!   `parameters` block, optionally *narrowed* (never widened) by the agent.
//! * [`sampler`] — where to look next: seeded random and a compact
//!   Tree-structured Parzen Estimator. No external RNG crate; every sweep is
//!   reproducible from its seed.
//! * [`surface`] — the [`SurfaceSummary`] the LLM reads instead of an argmax:
//!   per-parameter plateaus, cliffs and sensitivity, plus a ≤ 1 KB rendering.
//! * [`sweep`] — the orchestration: batches of samples become sealed
//!   `ParameterSweep` Studies on an Experiment; the only carry-forward is a
//!   final neighbourhood Study's `MedianStableCentroid` (INV-2, §7.3).
//! * [`calibration`] — rank correlation between fidelity tiers; racing is
//!   enabled only when it is measured ≥ 0.8 (P5).
//!
//! The crate depends on `backtest` for the Set J types but holds no store,
//! pool or executor: everything that touches data goes through the
//! [`SweepBackend`] trait the API layer implements.

pub mod calibration;
pub mod search;
pub mod sampler;
pub mod space;
pub mod surface;
pub mod sweep;

pub use calibration::{spearman, FidelityCalibration, RACING_MIN_RHO};
pub use sampler::{RandomSampler, Rng, Sampler, SamplerKind, TpeSampler};
pub use space::{Narrow, SearchSpace, SpaceError};
pub use surface::{ParamSurface, Sensitivity, SurfaceSummary};
pub use sweep::{
    run_sweep, NoopObserver, SweepBackend, SweepError, SweepObserver, SweepReport, SweepRequest,
};
