//! SLv2 — the text a model writes a strategy in (AGENT-011, revising FEAT-004 §3).
//!
//! # Why this crate exists
//!
//! `crates/features` computes about fifty feature families and
//! `crates/backtest/src/sim.rs` resolves strategy features through it, so the
//! *engine* has never been the limit. The limit is what a model can **say**: it
//! writes a `StrategyDefinition` as JSON, and a JSON document is a shape to fill
//! in rather than a thing to think in. AGENT-011's argument is that the
//! authoring surface is the capability ceiling, and this crate is that surface.
//!
//! Nothing here reaches a database, a model or the network. It is text in,
//! either a definition or a diagnostic out — which is what makes the whole of it
//! testable without services, and what keeps the validator's answers the same
//! whoever asks.

#![allow(clippy::module_name_repetitions)]
