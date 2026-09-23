//! Corporate actions (DATA-005 §11 S2; INV-03; Set P, P-3.3).
//!
//! **Prices stay unadjusted in storage and factors are applied at read time.**
//! That is INV-03, and the platform has never had to honour it because it has
//! never held an equity corporate action. This module is where the first one
//! lands, so the rule stops being vacuous.
//!
//! An adjusted close is a function of the *entire future* action stream. Storing
//! adjusted prices means every split silently rewrites history, every dataset
//! hash built before it becomes unreproducible, and a backtest run today over
//! 2019 sees prices no 2019 strategy could have seen. So the store holds the
//! prices that printed, the actions are facts with their own `knowledge_time`,
//! and `dataplane::corporate::factors_at` composes the ones known by `as_of`.

pub mod alpaca;
