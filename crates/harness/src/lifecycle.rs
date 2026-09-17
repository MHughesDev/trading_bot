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
//!
//! Sans-IO, like the rest of this crate: each module takes what the ledger and
//! the funnel produced and returns a decision, so the decision is testable
//! without a database and cannot quietly acquire a dependency on one.

pub mod evaluate;
