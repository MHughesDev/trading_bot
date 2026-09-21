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
//! # The `judgement` suite measures something none of the others can
//!
//! Every suite above scores whether the harness *behaved*. None of them can tell
//! a run that found a real effect from a run that reported noise, because there
//! is nothing to compare an answer against: "no edge" and "an edge" both look
//! like a run that reported something. A tier that completes every workflow and
//! cannot tell those apart is a tier that will confidently report noise.
//!
//! [`planted`] generates data whose answer is **known and verified** — an edge of
//! a declared size, or a null — from a seed, so a score is reproducible and a
//! regression is attributable. [`discrimination`] scores a tier on it, and
//! reports **two rates that are never combined**: how often it missed a planted
//! edge, and how often it called a planted null real. A single accuracy over a
//! mixed corpus can be gamed by answering the same thing every time, and would
//! be. A tier the suite has not been run on reports
//! [`discrimination::TierScore::Unmeasured`], which has no numeric form at all —
//! `0.0` for a tier nobody measured reads as "measured, and bad".
//!
//! **The answer key never leaves the scoring process.** It is generated rather
//! than stored, [`planted::AnswerKey`] implements no `Serialize`, its fields are
//! private, its `Debug` is redacted, and the only operation it offers is scoring
//! a run. `auth::scopes::NEVER_FOR_AGENTS` lists `evals.truth` because an agent
//! that can read the planted mechanism does not have to find it, and every
//! number on the scorecard stops meaning what it says.
//!
//! # This is not a second eval harness
//!
//! `evals/runner/run.py` and `evals/tests` exist and gate CI. This module holds
//! the parts that need the `Loop` itself — replay, tapes, the scorecard's
//! arithmetic — and `evals/runner/gate.py` calls the same arithmetic on the
//! Python side. A second harness beside the Python one is a recorded risk, so
//! the split is by *what needs the loop*, not by language preference.

pub mod cases;
pub mod discrimination;
pub mod holdout;
pub mod judgement;
pub mod planted;
pub mod scorecard;
pub mod tape;

pub use discrimination::{
    done_4, score_tier, Called, Discrimination, Done4, Measurement, NotMeasured, Threshold,
    TierScore,
};
pub use holdout::{Holdout, HoldoutDraw};
pub use judgement::{Criterion, Judgement, Observed, Planted, Score, Truth};
pub use planted::{CaseBrief, PlantedCase, PlantedCorpus};
pub use scorecard::{
    gate, Attribution, GateVerdict, Rung, Scorecard, SuiteKind, SuiteScore, SuiteVerdict,
};
pub use tape::{replay, Diff, Replay, Tape, TapeEntry, Trajectory};
