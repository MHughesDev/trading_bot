//! Rich Run outputs: fills, positions, decisions and per-leg trade costs (Set M M-2.5;
//! FEAT-005 §8; BT-10). DDL: `clickhouse/11_backtest_outputs_v2.sql`.
//!
//! [`build`] is pure: it turns one evaluated Layer 1 run into the four row sets, so what is
//! written can be tested without a database. [`write`] puts them in ClickHouse through the same
//! client the bar store uses. The tables are `ReplacingMergeTree` on the full key, so a second
//! write of one run's outputs collapses onto the first: **idempotent by `run_id`**, with no
//! delete-then-insert window in which a reader sees half a run.
//!
//! Decisions are one row per decision bar, and for a position series a "slot" is the one thing
//! the function computed, its target. A graph strategy has more slots, and the same table holds
//! them: the shape is `slots` / `signals` / `intents` as JSON, so the Layer 2 environment can
//! write into it without a new table (its writer is Set U territory and is not wired here).

use clickhouse::Row;
use serde::Serialize;
use serde_json::json;

use crate::position_run::{trades_and_legs, Layer1Artifact};
use crate::position_series::{Bar, PositionRun};

/// The DDL, so the writer can create its own tables (the shared migrator list is not ours).
pub const DDL: &str = include_str!("../../../clickhouse/11_backtest_outputs_v2.sql");

#[derive(Clone, Debug, PartialEq, Row, Serialize)]
pub struct FillRow {
    pub run_id: String,
    pub seq: u32,
    pub decided_ns: i64,
    pub filled_ns: i64,
    pub side: String,
    pub qty_str: String,
    pub price_str: String,
    pub fee_str: String,
    pub spread_cost_str: String,
    pub impact_cost_str: String,
}

#[derive(Clone, Debug, PartialEq, Row, Serialize)]
pub struct PositionRow {
    pub run_id: String,
    pub ts_ns: i64,
    pub qty: f64,
    pub exposure: f64,
    pub equity: f64,
}

#[derive(Clone, Debug, PartialEq, Row, Serialize)]
pub struct DecisionRow {
    pub run_id: String,
    pub ts_ns: i64,
    pub slots: String,
    pub signals: String,
    pub intents: String,
    pub regime: String,
    pub path: String,
}

#[derive(Clone, Debug, PartialEq, Row, Serialize)]
pub struct TradeDetailRow {
    pub run_id: String,
    pub trade_idx: u32,
    pub symbol: String,
    pub side: String,
    pub entry_ts_ns: i64,
    pub exit_ts_ns: i64,
    pub mae: f64,
    pub mfe: f64,
    pub exit_reason: String,
    pub entry_order_type: String,
    pub fee_str: String,
    pub spread_cost_str: String,
    pub impact_cost_str: String,
    pub borrow_cost_str: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RunOutputs {
    pub fills: Vec<FillRow>,
    pub positions: Vec<PositionRow>,
    pub decisions: Vec<DecisionRow>,
    pub trade_detail: Vec<TradeDetailRow>,
}

fn s(x: f64) -> String {
    format!("{x}")
}

/// The four row sets for one evaluated Layer 1 run.
#[must_use]
#[allow(clippy::float_cmp)] // exact: targets are copied, never computed
pub fn build(run_id: &str, a: &Layer1Artifact, run: &PositionRun) -> RunOutputs {
    let bars: Vec<Bar> = a.bars.iter().copied().map(Bar::from).collect();
    let mut out = RunOutputs::default();

    for (i, f) in run.fills.iter().enumerate() {
        out.fills.push(FillRow {
            run_id: run_id.into(),
            seq: u32::try_from(i).unwrap_or(u32::MAX),
            decided_ns: f.decided_ns,
            filled_ns: f.filled_bar_ns,
            side: if f.qty > 0.0 { "buy" } else { "sell" }.into(),
            qty_str: s(f.qty.abs()),
            price_str: s(f.price),
            fee_str: s(f.fee),
            spread_cost_str: s(f.spread_cost),
            impact_cost_str: s(f.impact_cost),
        });
    }

    let mut qty = 0.0;
    let mut fi = 0;
    for (b, (_, eq)) in bars.iter().zip(&run.equity) {
        while fi < run.fills.len() && run.fills[fi].filled_bar_ns <= b.ts_ns {
            qty += run.fills[fi].qty;
            fi += 1;
        }
        out.positions.push(PositionRow {
            run_id: run_id.into(),
            ts_ns: b.ts_ns,
            qty,
            exposure: if *eq > 0.0 { qty * b.close / eq } else { 0.0 },
            equity: *eq,
        });
    }

    let mut prev: f64 = 0.0;
    for p in &a.series.points {
        let changed = p.target.is_some_and(|t| t != prev);
        if let Some(t) = p.target {
            prev = t;
        }
        out.decisions.push(DecisionRow {
            run_id: run_id.into(),
            ts_ns: p.ts_ns,
            slots: json!({ "target": p.target }).to_string(),
            signals: "{}".into(),
            intents: if changed {
                json!([{ "kind": "target", "value": p.target }])
            } else {
                json!([])
            }
            .to_string(),
            regime: String::new(),
            path: "position_series".into(),
        });
    }

    for (i, (trade, legs)) in trades_and_legs(&a.instrument, &run.fills, &bars)
        .iter()
        .enumerate()
    {
        out.trade_detail.push(TradeDetailRow {
            run_id: run_id.into(),
            trade_idx: u32::try_from(i).unwrap_or(u32::MAX),
            symbol: trade.symbol.clone(),
            side: if trade.side == crate::run::Side::Long {
                "long"
            } else {
                "short"
            }
            .into(),
            entry_ts_ns: trade.entry_time.timestamp_nanos_opt().unwrap_or(0),
            exit_ts_ns: trade.exit_time.timestamp_nanos_opt().unwrap_or(0),
            mae: trade.mae,
            mfe: trade.mfe,
            exit_reason: "target_change".into(),
            entry_order_type: "market".into(),
            fee_str: s(legs.fee),
            spread_cost_str: s(legs.spread),
            impact_cost_str: s(legs.impact),
            borrow_cost_str: "0".into(),
        });
    }
    out
}

/// Create the four tables if they are not there.
///
/// # Errors
/// ClickHouse refusing a statement.
pub async fn ensure_schema(url: &str) -> anyhow::Result<()> {
    let client = storage::clickhouse::connect(url);
    for stmt in DDL.split(';').map(str::trim).filter(|t| !t.is_empty()) {
        // Comment-only chunks are skipped; a chunk that starts with comments and then holds a
        // statement is sent whole, which ClickHouse accepts.
        if stmt
            .lines()
            .all(|l| l.trim().is_empty() || l.trim_start().starts_with("--"))
        {
            continue;
        }
        client.query(stmt).execute().await?;
    }
    Ok(())
}

/// Write one run's outputs. Idempotent by `run_id` (see the module docs).
///
/// # Errors
/// ClickHouse refusing an insert.
pub async fn write(url: &str, out: &RunOutputs) -> anyhow::Result<()> {
    let client = storage::clickhouse::connect(url);
    macro_rules! put {
        ($table:literal, $rows:expr) => {
            for chunk in $rows.chunks(5_000) {
                let mut insert = client.insert($table)?;
                for r in chunk {
                    insert.write(r).await?;
                }
                insert.end().await?;
            }
        };
    }
    put!("backtest_run_fills", out.fills);
    put!("backtest_run_positions", out.positions);
    put!("backtest_run_decisions", out.decisions);
    put!("backtest_run_trade_detail", out.trade_detail);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::position_run::{ArtifactBar, PositionRunExecutor};
    use crate::position_series::{PositionPoint, PositionSeries, PositionUnits};
    use std::sync::Arc;

    fn artifact() -> Layer1Artifact {
        let bars: Vec<ArtifactBar> = (0..90)
            .map(|i| {
                #[allow(clippy::cast_precision_loss)]
                let p = 100.0 + (f64::from(i) * 0.3).sin() * 4.0;
                ArtifactBar {
                    ts_ns: (i64::from(i) + 1) * 3_600_000_000_000,
                    open: p,
                    close: p + 0.1,
                }
            })
            .collect();
        let points = bars
            .iter()
            .enumerate()
            .map(|(i, b)| PositionPoint::new(b.ts_ns, if (i / 12) % 2 == 0 { 1.0 } else { -1.0 }))
            .collect();
        Layer1Artifact {
            code_sha256: "ab".repeat(32),
            instrument: "BTC-USD".into(),
            timeframe: "1h".into(),
            series: PositionSeries {
                points,
                position_units: PositionUnits::Fraction,
                allow_short: true,
            },
            bars,
            initial_equity: 10_000.0,
            fee_bps: 10.0,
            half_spread_bps: 2.0,
            cost_model_ref: "cost:flat".into(),
            data_snapshot: "s".into(),
            lot_size: None,
            impact: Some(crate::position_series::SqrtImpact {
                y: 1.0,
                sigma: 0.01,
                volume_usd: 1e6,
            }),
            params: crate::run::ParamMap::new(),
            seed: 1,
        }
    }

    fn built() -> (Layer1Artifact, RunOutputs) {
        let a = artifact();
        let run = PositionRunExecutor::new(Arc::new(a.clone()), None)
            .evaluate()
            .unwrap();
        let out = build("run-1", &a, &run);
        (a, out)
    }

    #[test]
    fn a_run_writes_one_decisions_row_per_decision_bar_and_one_position_row_per_bar() {
        let (a, out) = built();
        assert_eq!(out.decisions.len(), a.series.points.len());
        assert_eq!(out.positions.len(), a.bars.len());
        assert!(out
            .decisions
            .iter()
            .all(|d| d.run_id == "run-1" && d.path == "position_series"));
        assert!(
            out.decisions.iter().any(|d| d.intents != "[]"),
            "a changed target is an intent"
        );
    }

    #[test]
    fn the_costs_on_a_trade_equal_the_sum_of_its_legs_and_of_the_fills_it_came_from() {
        let (a, _) = built();
        let run = PositionRunExecutor::new(Arc::new(a.clone()), None)
            .evaluate()
            .unwrap();
        let bars: Vec<Bar> = a.bars.iter().copied().map(Bar::from).collect();
        let pairs = trades_and_legs("BTC-USD", &run.fills, &bars);
        assert!(!pairs.is_empty());
        for (trade, legs) in &pairs {
            let paid: f64 = rust_decimal::prelude::ToPrimitive::to_f64(&trade.costs_paid).unwrap();
            assert!((paid - legs.total()).abs() < 1e-9, "{paid} vs {legs:?}");
            assert!(legs.impact > 0.0, "impact was supplied and is charged");
        }
        // Every fill's cost is on some trade or on the position still open at the end.
        let on_trades: f64 = pairs.iter().map(|(_, l)| l.total()).sum();
        let all_fills: f64 = run
            .fills
            .iter()
            .map(|f| f.fee + f.spread_cost + f.impact_cost)
            .sum();
        assert!(on_trades <= all_fills + 1e-9);
    }

    #[test]
    fn rebuilding_the_same_run_gives_identical_rows_so_a_rewrite_collapses_onto_them() {
        let (a, first) = built();
        let run = PositionRunExecutor::new(Arc::new(a.clone()), None)
            .evaluate()
            .unwrap();
        assert_eq!(build("run-1", &a, &run), first);
        assert_ne!(
            build("run-2", &a, &run).fills[0].run_id,
            first.fills[0].run_id
        );
    }

    #[test]
    fn the_ddl_makes_every_table_replacing_on_its_full_key() {
        for t in [
            "backtest_run_fills",
            "backtest_run_positions",
            "backtest_run_decisions",
            "backtest_run_trade_detail",
        ] {
            assert!(
                DDL.contains(&format!("CREATE TABLE IF NOT EXISTS {t}")),
                "{t}"
            );
        }
        assert_eq!(
            DDL.matches("ENGINE = ReplacingMergeTree()").count(),
            4,
            "idempotent by run_id"
        );
        assert!(!DDL.contains("ENGINE = MergeTree()"));
    }
}
