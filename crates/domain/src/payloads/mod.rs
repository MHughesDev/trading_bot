//! Versioned payload types and the `Payload` trait.
//!
//! The compiled Rust structs *are* the schema registry: adding a new payload
//! requires a new struct + a new `AnyPayload` variant.  There is no separate
//! schema registry service.
//!
//! All OHLCV / price / size fields use `Price` / `Size` — never `f64`.

pub mod bar;
pub mod orderbook;
pub mod quote;
pub mod trade;

/// Trait implemented by every v1 payload struct.
pub trait Payload: serde::Serialize + for<'de> serde::Deserialize<'de> {
    /// Dotted event-type name (e.g. `"market.trade.v1"`).
    fn event_type() -> &'static str;
    /// Schema version string (e.g. `"1"`).
    fn schema_version() -> &'static str;
}

/// Discriminated union of all concrete payload types.
///
/// Use this when you need to store a heterogeneous collection of events or
/// pattern-match over payload kinds without knowing the concrete type at compile
/// time.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AnyPayload {
    Trade(trade::TradePayload),
    Quote(quote::QuotePayload),
    OrderBook(orderbook::OrderBookPayload),
    Bar(bar::BarPayload),
}
