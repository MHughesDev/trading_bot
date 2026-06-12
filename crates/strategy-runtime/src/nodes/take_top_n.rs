//! TakeTopN node — retains the first N entries from the (already-ranked) universe.

use crate::nodes::UniverseEntry;

/// Keep the first `n` entries.  If `universe.len() < n`, returns all entries.
pub fn take_top_n(universe: &[UniverseEntry], n: usize) -> Vec<UniverseEntry> {
    universe.iter().take(n).cloned().collect()
}
