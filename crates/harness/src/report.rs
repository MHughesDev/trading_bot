//! The report layer (AGENT-006 §5.8, AGENT-008 §3.4; Set S phase 8).
//!
//! A report a human can trust cold is the set's exit, and what makes one is not
//! better prose: it is that every number in it resolves to something the
//! platform measured, and that the check is made by the platform rather than
//! asserted by the writer.
//!
//! [`claims`] is the first half — decomposing what the model wrote and aligning
//! each assertion against the evidence it actually saw.

pub mod claims;
pub mod critic;
pub mod dossier;
