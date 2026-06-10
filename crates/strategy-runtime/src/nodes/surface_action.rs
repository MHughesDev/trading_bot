//! SurfaceAction node — discovery terminal.
//!
//! Passes the final universe through unchanged.  The caller reads the instrument
//! IDs to populate a scanner panel.  A strategy whose only terminal is
//! `SurfaceAction` (no `PlaceOrder` action) infers `StrategyKind::Discovery`.

use crate::nodes::Universe;

/// Pass-through: returns the input universe as the surfaced set.
pub fn surface(universe: Universe) -> Universe {
    universe
}
