//! External inputs to indicators: macro vintages and EDGAR facts (Set P P-3.6;
//! DATA-006; INV-13, INV-14).
//!
//! A `macro_<series>` or `fundamental_<cik>_<tag>` indicator is not computed
//! from bars: its value at a decision is a published number *as it stood then*.
//! The engine stays pure, so the rows come from the environment as data —
//! [`ExternalData`], handed over with [`crate::Event::External`] — and the
//! engine evaluates each such column itself, through
//! `features::external_features::value_at_external`, the one routine backtest
//! and live serving share.
//!
//! **Point-in-time is the resolver's, not the environment's.** A row counts at a
//! decision only if its `knowledge_time` is at or before the decision instant
//! (`ExternalInputs::resolve`); handing a backtest every row of its window up
//! front cannot leak a later vintage into an earlier bar. A series whose newest
//! knowable vintage is missing has no value — never an older number, never zero.
//!
//! Because the rows arrive as an event (and with a runner's genesis), a replay
//! sees exactly what the decision saw.

use dataplane::fundamentals::FundamentalFact;
use dataplane::vintage::MacroObservation;
use features::external_inputs::ExternalInputs;
use serde::{Deserialize, Serialize};

/// The rows every external indicator of a plan reads. Each delivery replaces
/// the last whole: a refresh carries everything knowable at its instant.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalData {
    #[serde(default)]
    pub macro_rows: Vec<MacroObservation>,
    #[serde(default)]
    pub facts: Vec<FundamentalFact>,
}

impl ExternalData {
    /// The resolver's view of these rows.
    #[must_use]
    pub fn inputs(&self) -> ExternalInputs<'_> {
        ExternalInputs {
            macro_rows: &self.macro_rows,
            facts: &self.facts,
        }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.macro_rows.is_empty() && self.facts.is_empty()
    }
}
