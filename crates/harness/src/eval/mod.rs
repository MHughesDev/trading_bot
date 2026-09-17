//! Evaluation and calibration (AGENT-009; Set S tasks S-4.8…S-4.13, S-4.17).
//!
//! # Why this is a harness, and is treated like one
//!
//! The one eval that existed scored the local tier **0/10** and nearly
//! "confirmed" that a 7B could not multi-step. It was the eval's own bug: it
//! drove exposure off the call *count*, so `read_bars` was retired the moment it
//! was tried and was absent on the step where recovery had to happen. The check
//! was unsatisfiable by construction. After the harness bugs were fixed the same
//! chain scored 10/10, twice (`LOCAL_TIER_FINDINGS.md` §9).
//!
//! What that cost is the reason this module exists: *the trace agreed with the
//! hypothesis, and a result that confirms what you already believe is the one
//! nobody re-reads.* So an evaluation of an agent harness is itself a harness and
//! gets the same discipline — deterministic where possible, replayable, and
//! measured against a reference rather than a belief.
//!
//! **[`cases::eval_bug_0_of_10`] is that exact trajectory, as a passing replay
//! case.** An eval harness that cannot reproduce its own historical failure is
//! not an eval harness.
//!
//! # Two kinds of suite, and they are gated differently
//!
//! | | Deterministic ([`SuiteKind::Replay`]) | Stochastic ([`SuiteKind::Live`]) |
//! |---|---|---|
//! | What runs | a recorded tape through `Loop::next` | a model |
//! | Sampling error | none | yes |
//! | Gate | any newly failing case blocks, no threshold | one-sided 95% Wilson bound below zero, n ≥ 30, else `UNDERPOWERED` |
//! | Cadence | every PR, seconds | nightly, per tier, under a cost cap |
//!
//! That split is [`scorecard::gate`], and it is the part of this module most
//! worth getting right: a bare two-point threshold on a pass rate would be the
//! spec that demands a deflated Sharpe ratio of a strategy gating *itself* on a
//! number with no interval.
//!
//! # This is not a second eval harness
//!
//! `evals/runner/run.py` and `evals/tests` exist and gate CI. This module holds
//! the parts that need the `Loop` itself — replay, tapes, the scorecard's
//! arithmetic — and `evals/runner/gate.py` calls the same arithmetic on the
//! Python side. A second harness beside the Python one is a recorded risk, so
//! the split is by *what needs the loop*, not by language preference.

pub mod cases;
pub mod holdout;
pub mod judgement;
pub mod scorecard;
pub mod tape;

pub use holdout::{Holdout, HoldoutDraw};
pub use scorecard::{
    gate, Attribution, GateVerdict, Rung, Scorecard, SuiteKind, SuiteScore, SuiteVerdict,
};
pub use tape::{replay, Diff, Replay, Tape, TapeEntry, Trajectory};
