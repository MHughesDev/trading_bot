//! The **Study** — a deliberate set of Runs answering one question along one
//! varying dimension, reporting a **sealed** distribution (spec §1.2).
//!
//! A Study is where INV-2 is enforced: the [`StudyResult`] exposes the
//! distribution's properties (median / IQR / worst-5% / spread) but no API to
//! select, return, or promote the best-performing member. A single config may be
//! carried forward only through a pre-declared [`SelectionRule`], never an argmax.

pub mod config;
pub mod engine;
pub mod result;
pub mod store;

pub use config::{
    SelectionRule, StudyBudget, StudyConfig, StudyConfigError, StudyKind, VarySpec,
};
pub use engine::{cpcv_assignments, combinations, CpcvSplit, StudyEngine};
pub use result::{percentile, Distribution, StudyResult, StudyVerdict};
pub use store::{InMemoryStudyStore, StudyStore};
