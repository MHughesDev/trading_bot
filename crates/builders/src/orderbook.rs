/// L2 order-book reconstruction — Phase 2.
///
/// Applies snapshot + incremental delta updates to maintain a local view of
/// the order book.  Not yet implemented; wired up in Phase 2 alongside the
/// orderbook feature engine.
pub struct OrderBookBuilder;

impl OrderBookBuilder {
    pub fn new() -> Self {
        Self
    }
}

impl Default for OrderBookBuilder {
    fn default() -> Self {
        Self::new()
    }
}
