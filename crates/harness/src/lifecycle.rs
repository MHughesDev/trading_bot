//! The research lifecycle, harness side (AGENT-013 §5, §6; Set S phase 10).
//!
//! The lifecycle is intake → ground → preregister → hypothesise → design →
//! execute → evaluate → diagnose → refine → report. Most of those steps are
//! things a run *does*, and they live in the catalogue as tools. What lives here
//! is what the platform decides **about** a run, which is deliberately not
//! reachable from a tool call:
//!
//! * [`evaluate`] — whether the hypothesis was supported. A verdict the model
//!   writes is a verdict the model can rewrite.
//! * [`diagnose`] — why it failed. A model that may pick its own failure class
//!   will pick the one whose retry condition is "when the platform is bigger".
//! * [`chain`] — whether another refinement is worth trying. A run that may
//!   refine can refine forever, and each step looks locally reasonable.
//! * [`r#yield`] — what the run actually produced. A run can end having produced
//!   no knowledge while every dashboard shows activity.
//! * [`workflows`] — which stages a question shape requires. A workflow may skip
//!   a stage where its skeleton says so, and may never skip a gate.
//!
//! Sans-IO, like the rest of this crate: each module takes what the ledger and
//! the funnel produced and returns a decision, so the decision is testable
//! without a database and cannot quietly acquire a dependency on one.

pub mod chain;
pub mod diagnose;
pub mod evaluate;
pub mod skeleton;
pub mod workflows;
pub mod r#yield;
