//! L0/L1 data plane (SPEC §1–§3).

pub mod align;
pub mod asof;
pub mod bar;
pub mod calendar;
pub mod corporate;
pub mod crypto;
pub mod dataset;
pub mod defi;
pub mod feature;
pub mod futures;
pub mod hash;
pub mod identity;
pub mod label;
pub mod options;
pub mod quality;
pub mod restatement;
pub mod split;

pub use hash::content_hash;
pub use identity::{InstrumentKey, VenueKey};
pub use quality::QualityFlags;
