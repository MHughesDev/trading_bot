//! Bar sampling schemes other than the clock (Set P Phase 4).
//!
//! `crate::bar` is the *stored* contract — the canonical clock bar, with its four
//! timestamps, that everything is written as. This module holds derivations
//! computed at read time from that stored series and never stored themselves:
//! see [`information_driven`] for volume, dollar and tick bars, and
//! [`crate::resample`] for coarser clock bars.
//!
//! The rule both share: a derived series inherits the **worst** grade and the
//! **largest** declared lag of its inputs, through
//! [`crate::resample::inherit_provenance`], so a derivation can never present
//! itself as cleaner than what it was derived from.

pub mod information_driven;
