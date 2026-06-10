//! TakeTopN node — retains the first N entries from the (already-ranked) universe.

use crate::nodes::Universe;

/// Keep the first `n` entries.  If `universe.len() < n`, returns all entries.
pub fn take_top_n(universe: Universe, n: usize) -> Universe {
    universe.into_iter().take(n).collect()
}
