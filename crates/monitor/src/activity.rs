//! The behaviour family's inputs that are not orders: signals, round trips and
//! positions (COMP-007 §3; S-7.7).
//!
//! # Where each one comes from, and why it is attributable
//!
//! The earlier objection to `position_count` was right about the `positions`
//! table: it is keyed `(account_id, instrument_id)` with no strategy column, so
//! any per-strategy count from it attributes a shared instrument to every
//! strategy that ever traded it. These are not read from there. They are read
//! from the strategy's **own fills** — `fills` joined to `orders` on the
//! idempotency key, where `orders.strategy_id` is the deployment id the v3 runner
//! stamps on every intent. A fill belongs to exactly one order and an order to
//! exactly one strategy, so nothing here is shared.
//!
//! - **Round trips** (holding period p50): the strategy's net position per
//!   instrument, replayed in fill order. A round trip opens when the position
//!   leaves zero and closes when it returns to, or crosses, zero.
//! - **Position count**: instruments with a non-zero net position at the end of
//!   the window, from the same replay. Only reported for a strategy whose whole
//!   fill history was read — a replay that started mid-history would count a
//!   position it never saw open as flat.
//! - **Signals per hour**: decisions in `strategy_decision_journal` (0144) that
//!   produced at least one gate verdict, i.e. that tried to place an order.
//!   Only reported for a deployment that was running for the whole window — a
//!   deployment started ten minutes ago has a rate over ten minutes, and dividing
//!   it by sixty would manufacture the "silently stopped trading" alarm the
//!   metric exists to raise.
//!
//! Slippage against the fill model is **not** measured and cannot be on paper:
//! since ADR-0044/0048 paper fills *are* the fill model's output, so realised
//! minus predicted is zero by construction, and reporting that zero would be a
//! measurement of nothing dressed as a perfect score. It stays `Unobserved`
//! until a live venue produces a fill the model did not.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use sqlx::Row;

use crate::collect::{HoldingPeriod, Window};
use crate::store::{MonitorStore, StoreError};

/// One fill, attributed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttributedFill {
    pub strategy_id: String,
    pub instrument_id: String,
    /// `+qty` for a buy, `-qty` for a sell.
    pub signed_qty: Decimal,
    pub filled_at: DateTime<Utc>,
}

/// What a strategy's fill history says over a window.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FillReplay {
    /// Round trips that **closed** inside the window.
    pub round_trips: Vec<HoldingPeriod>,
    /// Instruments with a non-zero net position at the window's end.
    pub open_positions: usize,
}

/// Replay one strategy's fills, in order, and read off round trips and the
/// open book.
///
/// Fills must be in `filled_at` order; ties keep their given order. A fill that
/// carries the position *through* zero closes one round trip and opens the next
/// at the same instant.
#[must_use]
pub fn replay(fills: &[AttributedFill], window: &Window) -> FillReplay {
    let mut net: BTreeMap<&str, (Decimal, Option<DateTime<Utc>>)> = BTreeMap::new();
    let mut out = FillReplay::default();
    for f in fills.iter().filter(|f| f.filled_at <= window.to) {
        let (pos, opened) = net.entry(f.instrument_id.as_str()).or_insert((Decimal::ZERO, None));
        let before = *pos;
        let after = before + f.signed_qty;
        if before.is_zero() && !after.is_zero() {
            *opened = Some(f.filled_at);
        } else if !before.is_zero() && (after.is_zero() || after.is_sign_positive() != before.is_sign_positive()) {
            if let Some(o) = opened.take() {
                if window.contains(f.filled_at) {
                    out.round_trips.push(HoldingPeriod {
                        opened_at: o,
                        closed_at: f.filled_at,
                    });
                }
            }
            if !after.is_zero() {
                *opened = Some(f.filled_at);
            }
        }
        *pos = after;
    }
    out.open_positions = net.values().filter(|(p, _)| !p.is_zero()).count();
    out
}

impl MonitorStore {
    /// Every fill of every strategy that has an order at or before `to`,
    /// attributed, in fill order.
    ///
    /// The whole history rather than the window, because a position opened
    /// before the window is still open in it; paper volume makes this cheap, and
    /// the bound to add when it is not is a per-strategy watermark, not a time
    /// cut that silently starts the replay mid-position.
    ///
    /// # Errors
    /// The database, or a side the journal never writes.
    pub async fn attributed_fills(
        &self,
        to: DateTime<Utc>,
    ) -> Result<BTreeMap<String, Vec<AttributedFill>>, StoreError> {
        let rows = sqlx::query(
            "SELECT o.strategy_id, f.instrument_id, f.side, f.filled_size, f.filled_at
               FROM fills f
               JOIN orders o ON o.idempotency_key = f.idempotency_key
              WHERE o.strategy_id IS NOT NULL
                AND f.filled_at <= $1
              ORDER BY f.filled_at, f.fill_id",
        )
        .bind(to)
        .fetch_all(self.pool())
        .await?;
        let mut out: BTreeMap<String, Vec<AttributedFill>> = BTreeMap::new();
        for r in rows {
            let side: String = r.get("side");
            let qty: Decimal = r.get("filled_size");
            let signed_qty = match side.to_ascii_lowercase().as_str() {
                "buy" => qty,
                "sell" => -qty,
                other => {
                    return Err(StoreError::Invalid(format!("fill side {other:?} is neither buy nor sell")))
                }
            };
            let strategy_id: String = r.get("strategy_id");
            out.entry(strategy_id.clone()).or_default().push(AttributedFill {
                strategy_id,
                instrument_id: r.get("instrument_id"),
                signed_qty,
                filled_at: r.get("filled_at"),
            });
        }
        Ok(out)
    }

    /// Decisions that tried to place an order, per deployment, inside `window` —
    /// for deployments that were running for the **whole** window only.
    ///
    /// A running deployment present here with `0` decided nothing that reached
    /// the gate for an hour: a measurement, and the one `signals_per_hour`'s floor
    /// is declared to catch. A deployment absent from the map is unmeasured.
    ///
    /// # Errors
    /// The database.
    pub async fn signal_counts(&self, window: &Window) -> Result<BTreeMap<String, usize>, StoreError> {
        let rows = sqlx::query(
            "SELECT d.deployment_id::text AS strategy_id,
                    count(j.seq) FILTER (
                      WHERE jsonb_typeof(j.gate_verdicts) = 'array'
                        AND jsonb_array_length(j.gate_verdicts) > 0
                    ) AS signals
               FROM strategy_deployment d
               LEFT JOIN strategy_decision_journal j
                 ON j.deployment_id = d.deployment_id
                AND j.decided_at >= $1 AND j.decided_at <= $2
              WHERE d.started_at <= $1
                AND (d.stopped_at IS NULL OR d.stopped_at >= $2)
              GROUP BY d.deployment_id",
        )
        .bind(window.from)
        .bind(window.to)
        .fetch_all(self.pool())
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| {
                let n: i64 = r.get("signals");
                (r.get("strategy_id"), usize::try_from(n).unwrap_or(0))
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn t(min: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_800_000_000, 0).expect("t") + Duration::minutes(min)
    }

    fn fill(instrument: &str, qty: i64, at: i64) -> AttributedFill {
        AttributedFill {
            strategy_id: "s".into(),
            instrument_id: instrument.into(),
            signed_qty: Decimal::from(qty),
            filled_at: t(at),
        }
    }

    fn window() -> Window {
        Window::ending_at(t(120), Duration::minutes(60))
    }

    #[test]
    fn a_round_trip_that_closed_inside_the_window_is_measured_from_its_open() {
        // Opened before the window, closed inside it: the holding period is the
        // whole hold, not the part that overlapped the window.
        let r = replay(&[fill("BTC", 2, 30), fill("BTC", -1, 70), fill("BTC", -1, 90)], &window());
        assert_eq!(
            r.round_trips,
            [HoldingPeriod {
                opened_at: t(30),
                closed_at: t(90)
            }]
        );
        assert_eq!(r.open_positions, 0);
    }

    #[test]
    fn a_flip_through_zero_closes_one_trip_and_opens_the_next() {
        let r = replay(&[fill("BTC", 1, 70), fill("BTC", -2, 80), fill("BTC", 1, 100)], &window());
        assert_eq!(r.round_trips.len(), 2);
        assert_eq!(r.round_trips[0].closed_at, t(80));
        assert_eq!(r.round_trips[1].opened_at, t(80));
        assert_eq!(r.open_positions, 0);
    }

    #[test]
    fn open_positions_count_instruments_not_fills() {
        let r = replay(&[fill("BTC", 1, 70), fill("BTC", 1, 75), fill("ETH", -3, 80)], &window());
        assert_eq!(r.open_positions, 2);
        assert!(r.round_trips.is_empty());
    }

    #[test]
    fn a_trip_that_closed_before_the_window_is_not_this_windows() {
        let r = replay(&[fill("BTC", 1, 10), fill("BTC", -1, 20)], &window());
        assert!(r.round_trips.is_empty());
        assert_eq!(r.open_positions, 0);
    }
}
