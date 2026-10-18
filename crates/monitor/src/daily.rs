//! The P&L family's input: one deployment's daily return series, measured from
//! Postgres alone (COMP-007 §3; S-7.8).
//!
//! # Why not the account's equity
//!
//! The decision journal records the equity the engine saw, but for a
//! hand-bound deployment that is the **shared** paper account's equity — every
//! other strategy's P&L is in it. A CUSUM on that series would halt a strategy
//! for its neighbour's loss. So the series is rebuilt from what belongs to this
//! deployment and nothing else:
//!
//! - **cash**: `strategy_deployment.starting_capital`, less every fill of this
//!   deployment's own orders (`orders.strategy_id` is the deployment id) at its
//!   price, less its commission;
//! - **position**: the same fills, signed;
//! - **mark**: the close of the last bar this deployment *decided on* each day —
//!   `strategy_decision_journal.inputs.event.bar.close` — which is the price the
//!   strategy itself was looking at, carried forward over a day with no bar.
//!
//! Equity at each UTC day's end is cash plus position at the mark, and the
//! return is day-over-day. Only **complete** days: today is still moving.
//!
//! # When there is no series
//!
//! A deployment whose fills span more than one instrument (a v3 deployment
//! trades one), or that holds a position on a day before any mark is known,
//! has no honest valuation; the series stops there rather than inventing a
//! price. An empty series is unobserved, never flat.

use std::collections::BTreeMap;

use chrono::{DateTime, Duration, NaiveDate, Utc};
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use sqlx::Row;
use uuid::Uuid;

use crate::pnl::{DailyReturn, ReturnBaseline};
use crate::store::{MonitorStore, StoreError};

/// One of the deployment's own fills, priced.
#[derive(Clone, Debug, PartialEq)]
pub struct PricedFill {
    pub instrument_id: String,
    /// `+qty` buy, `-qty` sell.
    pub signed_qty: f64,
    pub price: f64,
    pub commission: f64,
    pub filled_at: DateTime<Utc>,
}

/// Everything the series is built from.
#[derive(Clone, Debug, PartialEq)]
pub struct PnlInputs {
    pub starting_capital: f64,
    pub started_at: DateTime<Utc>,
    pub fills: Vec<PricedFill>,
    /// The last decided-on close per UTC day.
    pub marks: BTreeMap<NaiveDate, f64>,
}

/// The daily return series up to and including `last_complete_day`.
#[must_use]
pub fn daily_returns(inputs: &PnlInputs, last_complete_day: NaiveDate) -> Vec<DailyReturn> {
    let mut out = Vec::new();
    if inputs.starting_capital <= 0.0 || !inputs.starting_capital.is_finite() {
        return out;
    }
    let instruments: std::collections::BTreeSet<&str> = inputs
        .fills
        .iter()
        .map(|f| f.instrument_id.as_str())
        .collect();
    if instruments.len() > 1 {
        return out;
    }
    let mut fills = inputs.fills.clone();
    fills.sort_by_key(|f| f.filled_at);

    let mut cash = inputs.starting_capital;
    let mut position = 0.0_f64;
    let mut prev_equity = inputs.starting_capital;
    let mut mark: Option<f64> = None;
    let mut next_fill = 0usize;
    let mut day = inputs.started_at.date_naive();
    while day <= last_complete_day {
        let day_end = day.and_hms_opt(0, 0, 0).expect("midnight").and_utc() + Duration::days(1);
        while next_fill < fills.len() && fills[next_fill].filled_at < day_end {
            let f = &fills[next_fill];
            cash -= f.signed_qty * f.price + f.commission;
            position += f.signed_qty;
            next_fill += 1;
        }
        if let Some(m) = inputs.marks.get(&day) {
            mark = Some(*m);
        }
        let equity = if position.abs() <= f64::EPSILON {
            cash
        } else {
            match mark {
                Some(m) => cash + position * m,
                // Holding something nobody has priced: stop, do not guess.
                None => break,
            }
        };
        if prev_equity <= 0.0 || !equity.is_finite() {
            break;
        }
        out.push(DailyReturn {
            date: day,
            value: equity / prev_equity - 1.0,
        });
        prev_equity = equity;
        day += Duration::days(1);
    }
    out
}

impl MonitorStore {
    /// The inputs for one v3 deployment, or `None` when `strategy_id` is not a
    /// deployment id (a v2 strategy, a manual id) — those have no starting
    /// capital on record and therefore no series.
    ///
    /// # Errors
    /// The database.
    pub async fn pnl_inputs(&self, strategy_id: &str) -> Result<Option<PnlInputs>, StoreError> {
        let Ok(deployment) = strategy_id.parse::<Uuid>() else {
            return Ok(None);
        };
        let Some(row) = sqlx::query(
            "SELECT starting_capital, started_at FROM strategy_deployment WHERE deployment_id = $1",
        )
        .bind(deployment)
        .fetch_optional(self.pool())
        .await?
        else {
            return Ok(None);
        };
        let capital: Decimal = row.get("starting_capital");
        let started_at: DateTime<Utc> = row.get("started_at");

        let fills = sqlx::query(
            "SELECT f.instrument_id, f.side, f.filled_size, f.fill_price,
                    coalesce(f.commission, 0) AS commission, f.filled_at
               FROM fills f
               JOIN orders o ON o.idempotency_key = f.idempotency_key
              WHERE o.strategy_id = $1
              ORDER BY f.filled_at, f.fill_id",
        )
        .bind(strategy_id)
        .fetch_all(self.pool())
        .await?
        .into_iter()
        .map(|r| {
            let side: String = r.get("side");
            let qty: Decimal = r.get("filled_size");
            let qty = qty.to_f64().unwrap_or(f64::NAN);
            let signed_qty = match side.to_ascii_lowercase().as_str() {
                "buy" => qty,
                "sell" => -qty,
                other => return Err(StoreError::Invalid(format!("fill side {other:?}"))),
            };
            let price: Decimal = r.get("fill_price");
            let commission: Decimal = r.get("commission");
            Ok(PricedFill {
                instrument_id: r.get("instrument_id"),
                signed_qty,
                price: price.to_f64().unwrap_or(f64::NAN),
                commission: commission.to_f64().unwrap_or(f64::NAN),
                filled_at: r.get("filled_at"),
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

        let marks = sqlx::query(
            "SELECT DISTINCT ON ((decided_at AT TIME ZONE 'UTC')::date)
                    (decided_at AT TIME ZONE 'UTC')::date AS day,
                    (inputs -> 'event' -> 'bar' ->> 'close')::float8 AS close
               FROM strategy_decision_journal
              WHERE deployment_id = $1
                AND inputs -> 'event' ->> 'event' = 'bar'
                AND jsonb_typeof(inputs -> 'event' -> 'bar' -> 'close') = 'number'
              ORDER BY (decided_at AT TIME ZONE 'UTC')::date, decided_at DESC, seq DESC",
        )
        .bind(deployment)
        .fetch_all(self.pool())
        .await?
        .into_iter()
        .map(|r| (r.get::<NaiveDate, _>("day"), r.get::<f64, _>("close")))
        .collect();

        Ok(Some(PnlInputs {
            starting_capital: capital.to_f64().unwrap_or(f64::NAN),
            started_at,
            fills,
            marks,
        }))
    }

    /// Declare a return baseline — a new version; the old stays legible.
    ///
    /// # Errors
    /// The database, or an `agent:` author.
    pub async fn declare_return_baseline(
        &self,
        strategy_id: &str,
        mean: f64,
        sigma: f64,
        periods_per_year: f64,
        declared_from: &str,
        declared_by: &str,
    ) -> Result<i32, StoreError> {
        if declared_by.starts_with("agent:") {
            return Err(StoreError::Invalid(format!(
                "{declared_by} may not declare the baseline it is measured against"
            )));
        }
        ReturnBaseline::declared(mean, sigma, periods_per_year, declared_from)
            .map_err(|e| StoreError::Invalid(e.to_string()))?;
        let version: i32 = sqlx::query_scalar(
            "INSERT INTO monitor.return_baseline
               (strategy_id, version, mean_return, sigma, periods_per_year, declared_from, declared_by)
             SELECT $1, coalesce(max(version), 0) + 1, $2, $3, $4, $5, $6
               FROM monitor.return_baseline WHERE strategy_id = $1
             RETURNING version",
        )
        .bind(strategy_id)
        .bind(mean)
        .bind(sigma)
        .bind(periods_per_year)
        .bind(declared_from)
        .bind(declared_by)
        .fetch_one(self.pool())
        .await?;
        Ok(version)
    }

    /// Every strategy's current baseline.
    ///
    /// # Errors
    /// The database, or a stored row `ReturnBaseline::declared` refuses.
    pub async fn return_baselines(&self) -> Result<BTreeMap<String, ReturnBaseline>, StoreError> {
        let rows = sqlx::query(
            "SELECT DISTINCT ON (strategy_id)
                    strategy_id, mean_return, sigma, periods_per_year, declared_from
               FROM monitor.return_baseline
              ORDER BY strategy_id, version DESC",
        )
        .fetch_all(self.pool())
        .await?;
        let mut out = BTreeMap::new();
        for r in rows {
            let id: String = r.get("strategy_id");
            let from: String = r.get("declared_from");
            let b = ReturnBaseline::declared(
                r.get("mean_return"),
                r.get("sigma"),
                r.get("periods_per_year"),
                &from,
            )
            .map_err(|e| StoreError::Invalid(e.to_string()))?;
            out.insert(id, b);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(i: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 9, i).expect("date")
    }

    fn at(day: u32, hour: u32) -> DateTime<Utc> {
        d(day).and_hms_opt(hour, 0, 0).expect("t").and_utc()
    }

    fn fill(qty: f64, price: f64, day: u32) -> PricedFill {
        PricedFill {
            instrument_id: "BTC-USD".into(),
            signed_qty: qty,
            price,
            commission: 1.0,
            filled_at: at(day, 12),
        }
    }

    #[test]
    fn equity_is_cash_plus_position_at_the_decided_mark_net_of_fees() {
        let inputs = PnlInputs {
            starting_capital: 1000.0,
            started_at: at(1, 9),
            fills: vec![fill(1.0, 100.0, 1)],
            marks: [(d(1), 110.0), (d(2), 99.0)].into_iter().collect(),
        };
        let r = daily_returns(&inputs, d(3));
        // Day 1: cash 899, position 1 @ 110 -> 1009. Day 2: 899 + 99 = 998.
        // Day 3: no bar, mark carried -> 998, a flat day.
        assert_eq!(r.len(), 3);
        assert!((r[0].value - (1009.0 / 1000.0 - 1.0)).abs() < 1e-12);
        assert!((r[1].value - (998.0 / 1009.0 - 1.0)).abs() < 1e-12);
        assert!(r[2].value.abs() < 1e-12);
    }

    #[test]
    fn a_position_nobody_priced_stops_the_series_rather_than_guessing() {
        let inputs = PnlInputs {
            starting_capital: 1000.0,
            started_at: at(1, 9),
            fills: vec![fill(1.0, 100.0, 2)],
            marks: BTreeMap::new(),
        };
        let r = daily_returns(&inputs, d(4));
        assert_eq!(
            r.len(),
            1,
            "day 1 is flat and priced; day 2 holds an unpriced position"
        );
    }

    #[test]
    fn fills_on_two_instruments_are_not_one_deployments_series() {
        let mut other = fill(1.0, 5.0, 1);
        other.instrument_id = "ETH-USD".into();
        let inputs = PnlInputs {
            starting_capital: 1000.0,
            started_at: at(1, 9),
            fills: vec![fill(1.0, 100.0, 1), other],
            marks: [(d(1), 100.0)].into_iter().collect(),
        };
        assert!(daily_returns(&inputs, d(2)).is_empty());
    }

    #[test]
    fn today_is_not_a_complete_day() {
        let inputs = PnlInputs {
            starting_capital: 1000.0,
            started_at: at(5, 9),
            fills: vec![],
            marks: BTreeMap::new(),
        };
        assert!(daily_returns(&inputs, d(4)).is_empty());
    }
}
