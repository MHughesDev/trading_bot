//! DataSource node — selects the source DataType lane for the pipeline branch.
//!
//! At evaluation time the DataSource node simply passes the initial universe
//! through unchanged.  Its `data_type` field is used by the manifest compiler
//! to record a `required_lane` entry; no evaluation logic is needed here.

use crate::nodes::Universe;

/// Pass-through: returns the initial universe unchanged.
/// The data_type field is handled by the manifest compiler, not here.
pub fn pass_through(universe: Universe) -> Universe {
    universe
}
