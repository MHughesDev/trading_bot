//! Routes (AssetClass, DataType, Instrument) → VenueId and manages collector
//! lifecycle.  Collectors are never started at system init — only on demand.
//!
//! Routing:
//! * Crypto → Kraken (data)
//! * Equity → Alpaca data feed
//!
//! Execution routing is separate: live→Coinbase; paper→the platform's own paper
//! venue (`execution::paper`), never a broker's paper account (ADR-0048).

pub mod exec_router;
pub mod lifecycle;
pub mod registry;
pub mod resolver;

pub use exec_router::{ExecRouter, ExecutionMode, RouteOutcome, RoutingError};
pub use lifecycle::LifecycleManager;
pub use registry::CollectorRegistry;
pub use resolver::Resolver;
