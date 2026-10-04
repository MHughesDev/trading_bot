//! Translate every stored v1 strategy to `tbot.strategy/3`, re-run its latest
//! backtest on the one engine, and publish old against new (Set U U-8.5;
//! FEAT-007 §12).
//!
//! ```text
//! DATABASE_URL=… CLICKHOUSE_URL=… cargo run -p api --bin migrate_strategies_v3 -- \
//!     [--apply] [--out docs/reports/strategy-v3-migration.md]
//! ```
//!
//! For each strategy's head definition:
//!
//! - **translate** it (`strategy_lang::v3::translate_v1`) at the timeframe its
//!   latest backtest ran at — a v1 definition carries no timeframe of its own;
//!   with no backtest, 1h — or record the translator's named refusal;
//! - **compile** the translation against the model registry, as a backtest
//!   would, or record why it does not compile;
//! - **re-run** its latest completed backtest — same instrument, window and
//!   starting balance — on the engine (`backtest::runner`), and set the old
//!   result beside the new with every difference attributed to a listed
//!   behaviour change, or marked unattributed;
//! - with `--apply`, **save** the translation as the strategy's new head
//!   revision through the platform's one revision write
//!   (`api::routes::strategies::append_revision`), authored as `scheduler`.
//!   The v1 revision stays in the append-only history. Without `--apply`
//!   nothing is written to the database.
//!
//! The report lists every strategy, so no result changes silently.

use std::fmt::Write as _;
use std::sync::Arc;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use domain::strategy_def::StrategyDefinition;
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use serde::Serialize;
use serde_json::Value;
use strategy_engine::planner::OrderAction;
use strategy_lang::v3::translate_v1::{translate, BehaviourChange};

/// Who a migrated revision is attributed to (0058's `actor_kind`, `actor_id`).
const ACTOR_KIND: &str = "scheduler";
const ACTOR_ID: &str = "migrate_strategies_v3";
/// The timeframe a strategy with no backtest is translated at.
const DEFAULT_TIMEFRAME: &str = "1h";

#[derive(Debug, Serialize)]
struct Report {
    generated_at: DateTime<Utc>,
    applied: bool,
    strategies: Vec<Entry>,
}

#[derive(Debug, Serialize)]
struct Entry {
    strategy_id: String,
    v1_revision: String,
    timeframe: String,
    timeframe_source: &'static str,
    #[serde(flatten)]
    outcome: Outcome,
}

#[derive(Debug, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
enum Outcome {
    /// A construct with no v3 translation yet — named, never approximated.
    Refused { construct: String, at: String, why: String },
    /// v1 JSON this build cannot read as a v1 definition.
    Unreadable { why: String },
    /// Translated, but the graph does not compile today.
    DoesNotCompile { text: String, errors: String },
    Translated {
        v3_revision: String,
        text: String,
        changes: Vec<Change>,
        rerun: Option<Rerun>,
        /// Saved as the strategy's head revision.
        applied: bool,
    },
}

#[derive(Debug, Serialize)]
struct Change {
    code: String,
    detail: String,
}

impl From<&BehaviourChange> for Change {
    fn from(c: &BehaviourChange) -> Self {
        Self {
            code: c.code.to_string(),
            detail: c.detail.clone(),
        }
    }
}

#[derive(Debug, Serialize)]
struct Rerun {
    instrument: String,
    window: (DateTime<Utc>, DateTime<Utc>),
    initial_balance: f64,
    old: OldResult,
    new: Result<NewResult, String>,
    /// Each difference, and the listed change it is attributed to.
    differences: Vec<Difference>,
}

#[derive(Debug, Serialize)]
struct OldResult {
    /// Bars the old run processed (`iterations`).
    bars: Option<i64>,
    orders: Option<i64>,
    positions: Option<i64>,
    pnl: Option<f64>,
}

#[derive(Debug, Serialize)]
struct NewResult {
    orders: i64,
    round_trips: usize,
    pnl: f64,
    decisions: u64,
    gate_rejections: usize,
    holds: u64,
}

#[derive(Debug, Serialize)]
struct Difference {
    metric: &'static str,
    old: Option<f64>,
    new: f64,
    /// Behaviour-change codes present in this run that move this metric; empty
    /// means unattributed.
    attributed_to: Vec<String>,
}

struct Stored {
    strategy_id: String,
    head_revision: String,
    definition: Value,
}

struct Job {
    timeframe: String,
    instrument: String,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    initial_balance: Decimal,
    result: Value,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let apply = args.iter().any(|a| a == "--apply");
    let out = args
        .iter()
        .position(|a| a == "--out")
        .and_then(|i| args.get(i + 1).cloned())
        .unwrap_or_else(|| "docs/reports/strategy-v3-migration.md".into());
    let pg = sqlx::PgPool::connect(&std::env::var("DATABASE_URL").context("DATABASE_URL")?)
        .await
        .context("connect to Postgres")?;
    let ch_url = std::env::var("CLICKHOUSE_URL").unwrap_or_else(|_| "http://localhost:8123".into());
    let registry = Arc::new(backtest::models::RegistrySnapshot::load_or_empty(&pg).await);

    let stored: Vec<(String, String, Value)> = sqlx::query_as(
        "SELECT strategy_id, head_revision, definition_json FROM strategy_definitions \
         ORDER BY strategy_id",
    )
    .fetch_all(&pg)
    .await?;
    let mut entries = Vec::new();
    for (strategy_id, head_revision, definition) in stored {
        let s = Stored {
            strategy_id,
            head_revision,
            definition,
        };
        if backtest::runner::is_v3(&s.definition) {
            continue;
        }
        let job = latest_job(&pg, &s.strategy_id).await?;
        let entry = migrate(&s, job, &registry, &ch_url, &pg, apply).await;
        eprintln!("{}: {}", s.strategy_id, headline(&entry.outcome));
        entries.push(entry);
    }
    let report = Report {
        generated_at: Utc::now(),
        applied: apply,
        strategies: entries,
    };
    std::fs::write(&out, markdown(&report)).with_context(|| format!("write {out}"))?;
    let json_out = std::path::Path::new(&out).with_extension("json");
    std::fs::write(&json_out, serde_json::to_string_pretty(&report)?)?;
    eprintln!("wrote {out} and {}", json_out.display());
    Ok(())
}

async fn latest_job(pg: &sqlx::PgPool, strategy_id: &str) -> Result<Option<Job>> {
    let row: Option<(String, String, DateTime<Utc>, DateTime<Utc>, Decimal, Value)> =
        sqlx::query_as(
            "SELECT timeframe, instrument_id, start_time, end_time, initial_balance, result \
             FROM backtest_jobs WHERE strategy_slug = $1 AND status = 'completed' \
               AND result IS NOT NULL \
             ORDER BY coalesce(finished_at, created_at) DESC LIMIT 1",
        )
        .bind(strategy_id)
        .fetch_optional(pg)
        .await?;
    Ok(row.map(
        |(timeframe, instrument, start, end, initial_balance, result)| Job {
            timeframe,
            instrument,
            start,
            end,
            initial_balance,
            result,
        },
    ))
}

async fn migrate(
    s: &Stored,
    job: Option<Job>,
    registry: &Arc<backtest::models::RegistrySnapshot>,
    ch_url: &str,
    pg: &sqlx::PgPool,
    apply: bool,
) -> Entry {
    let (timeframe, timeframe_source) = job.as_ref().map_or(
        (DEFAULT_TIMEFRAME.to_string(), "default: no backtest on record"),
        |j| (j.timeframe.clone(), "its latest backtest"),
    );
    let entry = |outcome| Entry {
        strategy_id: s.strategy_id.clone(),
        v1_revision: s.head_revision.clone(),
        timeframe: timeframe.clone(),
        timeframe_source,
        outcome,
    };
    let def: StrategyDefinition = match serde_json::from_value(s.definition.clone()) {
        Ok(d) => d,
        Err(e) => return entry(Outcome::Unreadable { why: e.to_string() }),
    };
    let t = match translate(&def, &timeframe) {
        Ok(t) => t,
        Err(r) => {
            return entry(Outcome::Refused {
                construct: r.construct,
                at: r.at,
                why: r.why,
            })
        }
    };
    let plan = match backtest::runner::compile_for_run(
        &t.graph,
        &backtest::run::ParamMap::new(),
        registry.as_ref(),
    ) {
        Ok(p) => p,
        Err(errors) => {
            return entry(Outcome::DoesNotCompile {
                text: t.text,
                errors,
            })
        }
    };
    let revision = match t.graph.revision() {
        Ok(r) => r.as_str().to_string(),
        Err(e) => {
            return entry(Outcome::DoesNotCompile {
                text: t.text,
                errors: format!("no revision: {e}"),
            })
        }
    };
    let changes: Vec<Change> = t.changes.iter().map(Change::from).collect();
    let rerun = match job {
        Some(j) => Some(rerun(&s.strategy_id, plan, &t.graph, j, &changes, pg, ch_url).await),
        None => None,
    };
    let applied = if apply {
        match save(pg, &s.strategy_id, &t.graph, &revision).await {
            Ok(()) => true,
            Err(e) => {
                eprintln!("{}: not saved: {e:#}", s.strategy_id);
                false
            }
        }
    } else {
        false
    };
    entry(Outcome::Translated {
        v3_revision: revision,
        text: t.text,
        changes,
        rerun,
        applied,
    })
}

async fn rerun(
    strategy_id: &str,
    plan: strategy_engine::Plan,
    graph: &domain::strategy_graph::StrategyGraph,
    j: Job,
    changes: &[Change],
    pg: &sqlx::PgPool,
    ch_url: &str,
) -> Rerun {
    let old = old_result(&j.result);
    let executor =
        backtest::runner::V3RunExecutor::new(tokio::runtime::Handle::current(), pg.clone(), ch_url)
            .with_initial_balance(j.initial_balance);
    let new = match executor
        .load_plan(
            plan,
            &graph.asset_class,
            &j.instrument,
            j.start,
            j.end,
            execution::sim_venue::FillPolicy::NextBarOpen,
            strategy_id,
        )
        .await
    {
        Err((_, e)) => Err(e),
        Ok(run) => tokio::task::spawn_blocking(move || {
            backtest::runner::run(run, &mut strategy_engine::env::NoModels)
                .map_err(|e| e.to_string())
                .map(|o| new_result(&o, j.initial_balance))
        })
        .await
        .unwrap_or_else(|e| Err(format!("the run panicked: {e}"))),
    };
    let differences = match &new {
        Ok(n) => attribute(&old, n, changes),
        Err(_) => Vec::new(),
    };
    Rerun {
        instrument: j.instrument,
        window: (j.start, j.end),
        initial_balance: j.initial_balance.to_f64().unwrap_or(0.0),
        old,
        new,
        differences,
    }
}

/// What `market_simulator` reported (NautilusTrader's summary).
fn old_result(r: &Value) -> OldResult {
    let int = |v: Option<&Value>| {
        v.and_then(|x| x.as_i64().or_else(|| x.as_str().and_then(|s| s.parse().ok())))
    };
    OldResult {
        bars: int(r.get("iterations")),
        orders: int(r.get("total_orders")).or_else(|| int(r.pointer("/summary/orders.total"))),
        positions: int(r.get("total_positions")),
        pnl: r
            .get("stats_pnls")
            .and_then(Value::as_object)
            .and_then(|m| m.values().next())
            .and_then(|c| c.get("PnL (total)"))
            .and_then(Value::as_f64),
    }
}

fn new_result(o: &backtest::runner::V3Outcome, initial: Decimal) -> NewResult {
    let orders = o
        .traces
        .iter()
        .flat_map(|t| t.actions.iter())
        .filter(|a| matches!(a, OrderAction::Submit(_)))
        .count();
    let last = o.equity.last().map_or(0.0, |(_, e)| *e);
    NewResult {
        orders: i64::try_from(orders).unwrap_or(i64::MAX),
        round_trips: o.trades.len(),
        pnl: last - initial.to_f64().unwrap_or(0.0),
        decisions: o.manifest.decisions,
        gate_rejections: o.manifest.gate_rejections.len(),
        holds: o.manifest.holds_by_reason.values().sum(),
    }
}

/// Attribute each difference to the behaviour changes present in this run
/// that move that metric. A mechanism being present is the attribution — the
/// runs are not ablated one change at a time, and the report says so.
fn attribute(old: &OldResult, new: &NewResult, changes: &[Change]) -> Vec<Difference> {
    let listed = |code: &str| changes.iter().any(|c| c.code == code);
    let mut out = Vec::new();
    #[allow(clippy::cast_precision_loss)]
    let old_orders = old.orders.map(|n| n as f64);
    #[allow(clippy::cast_precision_loss)]
    let new_orders = new.orders as f64;
    if old_orders != Some(new_orders) {
        // Only mechanisms that change how many orders are placed. Fill timing
        // moves prices, not counts — attributing a count to it is how a
        // level-triggered translation placing 30× the orders hid (U-8.5).
        let mut why = Vec::new();
        if bars_differ(old, new) {
            why.push("bars_differ".into());
        }
        if new.gate_rejections > 0 && listed("risk_gate_applied") {
            why.push("risk_gate_applied".into());
        }
        if new.holds > 0 && listed("missing_holds") {
            why.push("missing_holds".into());
        }
        if listed("exits_are_orders") {
            why.push("exits_are_orders".into());
        }
        if listed("models_called") {
            why.push("models_called".into());
        }
        out.push(Difference {
            metric: "orders",
            old: old_orders,
            new: new_orders,
            attributed_to: why,
        });
    }
    if old.pnl.map_or(true, |p| (p - new.pnl).abs() > 1e-6) {
        let mut why = Vec::new();
        // The old venue facade recorded orders and reported no fills back
        // (FEAT-007 §1 defect 9), so its P&L was zero however it traded.
        if old.pnl == Some(0.0) && old.orders.is_some_and(|n| n > 0) {
            why.push("v1_venue_reported_no_fills".into());
        }
        if bars_differ(old, new) {
            why.push("bars_differ".into());
        }
        if listed("fills_next_bar_open") {
            why.push("fills_next_bar_open".into());
        }
        if new.gate_rejections > 0 && listed("risk_gate_applied") {
            why.push("risk_gate_applied".into());
        }
        out.push(Difference {
            metric: "pnl",
            old: old.pnl,
            new: new.pnl,
            attributed_to: why,
        });
    }
    out
}

/// The old run saw a different number of bars than the store now holds for
/// the same window — bars backfilled since, or gaps it skipped. Measured, not
/// assumed: one decision per primary bar against the old run's iterations.
fn bars_differ(old: &OldResult, new: &NewResult) -> bool {
    old.bars
        .is_some_and(|b| u64::try_from(b).ok() != Some(new.decisions))
}

/// Save the translation as the head, through the platform's one revision write.
async fn save(
    pg: &sqlx::PgPool,
    strategy_id: &str,
    graph: &domain::strategy_graph::StrategyGraph,
    revision: &str,
) -> Result<()> {
    let def_json = serde_json::to_value(graph)?;
    let canonical = graph.canonical_json()?;
    let mut tx = pg.begin().await?;
    api::routes::strategies::append_revision(
        &mut tx,
        ACTOR_KIND,
        ACTOR_ID,
        &api::routes::strategies::RevisionWrite {
            strategy_id,
            definition_version: "3",
            asset_class: &graph.asset_class,
            def_json: &def_json,
            revision,
            canonical: &canonical,
            parent_strategy_id: None,
        },
    )
    .await
    .map_err(|e| anyhow::anyhow!("{e}"))?;
    tx.commit().await?;
    Ok(())
}

fn headline(o: &Outcome) -> String {
    match o {
        Outcome::Refused { construct, why, .. } => format!("refused — `{construct}`: {why}"),
        Outcome::Unreadable { why } => format!("unreadable — {why}"),
        Outcome::DoesNotCompile { errors, .. } => format!("does not compile — {errors}"),
        Outcome::Translated {
            rerun, applied, ..
        } => format!(
            "translated{}{}",
            match rerun {
                Some(Rerun { new: Ok(_), .. }) => ", re-run",
                Some(Rerun { new: Err(_), .. }) => ", re-run failed",
                None => ", no backtest to re-run",
            },
            if *applied { ", saved" } else { "" }
        ),
    }
}

fn fmt_opt(v: Option<f64>) -> String {
    v.map_or_else(|| "—".into(), |x| format!("{x:.2}"))
}

fn markdown(r: &Report) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "# Stored strategies on the one engine — v1 → v3 migration\n");
    let _ = writeln!(
        s,
        "Generated {} by `cargo run -p api --bin migrate_strategies_v3{}` (Set U U-8.5; \
         FEAT-007 §12). {}\n",
        r.generated_at.format("%Y-%m-%d %H:%M UTC"),
        if r.applied { " -- --apply" } else { "" },
        if r.applied {
            "Translations that compile were saved as each strategy's head revision \
             (authored `scheduler` / `migrate_strategies_v3`); the v1 revision stays in the \
             append-only history."
        } else {
            "**Dry run: nothing was written to the database.**"
        }
    );
    let _ = writeln!(
        s,
        "Translated strategies are not expected to backtest as they did: FEAT-007 §12. \
         Every difference below names the listed behaviour change it is attributed to. \
         Attribution is by mechanism — the change is present in the run and moves that \
         metric — not by re-running with each change removed. Models answer `unavailable` \
         in these re-runs (no inference service), so a model-driven rule holds.\n"
    );
    let _ = writeln!(s, "| strategy | timeframe | outcome | orders old → new | P&L old → new |");
    let _ = writeln!(s, "|---|---|---|---|---|");
    for e in &r.strategies {
        let (orders, pnl) = match &e.outcome {
            Outcome::Translated {
                rerun: Some(Rerun { old, new: Ok(n), .. }),
                ..
            } => (
                format!("{} → {}", old.orders.map_or("—".into(), |o| o.to_string()), n.orders),
                format!("{} → {:.2}", fmt_opt(old.pnl), n.pnl),
            ),
            _ => ("—".into(), "—".into()),
        };
        let _ = writeln!(
            s,
            "| `{}` | {} ({}) | {} | {orders} | {pnl} |",
            e.strategy_id,
            e.timeframe,
            e.timeframe_source,
            headline(&e.outcome)
        );
    }
    for e in &r.strategies {
        let _ = writeln!(s, "\n## `{}`\n", e.strategy_id);
        let _ = writeln!(s, "v1 revision `{}`.\n", e.v1_revision);
        match &e.outcome {
            Outcome::Refused { construct, at, why } => {
                let _ = writeln!(
                    s,
                    "**Refused**, not approximated: `{construct}` at `{at}` — {why}. It keeps \
                     running on v1 until the translator covers it."
                );
            }
            Outcome::Unreadable { why } => {
                let _ = writeln!(s, "**Unreadable** as a v1 definition: {why}.");
            }
            Outcome::DoesNotCompile { text, errors } => {
                let _ = writeln!(s, "**Translated but does not compile today**: {errors}\n");
                let _ = writeln!(s, "```slv3\n{}\n```", text.trim_end());
            }
            Outcome::Translated {
                v3_revision,
                text,
                changes,
                rerun,
                applied,
            } => {
                let _ = writeln!(
                    s,
                    "v3 revision `{v3_revision}`{}.\n",
                    if *applied { ", saved as head" } else { "" }
                );
                let _ = writeln!(s, "```slv3\n{}\n```\n", text.trim_end());
                let _ = writeln!(s, "What changes:\n");
                for c in changes {
                    let _ = writeln!(s, "- `{}` — {}", c.code, c.detail);
                }
                match rerun {
                    None => {
                        let _ = writeln!(s, "\nNo completed backtest on record to re-run.");
                    }
                    Some(r) => {
                        let _ = writeln!(
                            s,
                            "\nRe-run of its latest backtest: `{}`, {} → {}, starting balance {:.2}.\n",
                            r.instrument,
                            r.window.0.format("%Y-%m-%d"),
                            r.window.1.format("%Y-%m-%d"),
                            r.initial_balance
                        );
                        match &r.new {
                            Err(e) => {
                                let _ = writeln!(s, "The v3 re-run did not run: {e}.");
                            }
                            Ok(n) => {
                                let _ = writeln!(
                                    s,
                                    "| | old (`market_simulator`) | new (engine) |\n|---|---:|---:|\n\
                                     | bars | {} | {} |\n| orders | {} | {} |\n| positions / round trips | {} | {} |\n\
                                     | P&L | {} | {:.2} |\n| decisions | — | {} |\n\
                                     | gate rejections | — | {} |\n| holds | — | {} |\n",
                                    r.old.bars.map_or("—".into(), |o| o.to_string()),
                                    n.decisions,
                                    r.old.orders.map_or("—".into(), |o| o.to_string()),
                                    n.orders,
                                    r.old.positions.map_or("—".into(), |o| o.to_string()),
                                    n.round_trips,
                                    fmt_opt(r.old.pnl),
                                    n.pnl,
                                    n.decisions,
                                    n.gate_rejections,
                                    n.holds
                                );
                                for d in &r.differences {
                                    let _ = writeln!(
                                        s,
                                        "- **{}** {} → {:.2}: {}",
                                        d.metric,
                                        fmt_opt(d.old),
                                        d.new,
                                        if d.attributed_to.is_empty() {
                                            "**unattributed**".to_string()
                                        } else {
                                            d.attributed_to
                                                .iter()
                                                .map(|c| format!("`{c}`"))
                                                .collect::<Vec<_>>()
                                                .join(", ")
                                        }
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    let _ = writeln!(
        s,
        "\n`v1_venue_reported_no_fills`: the v1 backtest venue facade recorded orders and \
         reported no fills back (FEAT-007 §1 defect 9), so a v1 backtest's P&L was zero \
         however the strategy traded.\n\n`bars_differ`: the old run processed a different number of bars than the bar store now holds for the same window (bars backfilled since, or gaps the old run skipped), so the two runs did not see the same market."
    );
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn changes(codes: &[&str]) -> Vec<Change> {
        codes
            .iter()
            .map(|c| Change {
                code: (*c).into(),
                detail: String::new(),
            })
            .collect()
    }

    fn new(orders: i64, pnl: f64) -> NewResult {
        NewResult {
            orders,
            round_trips: 1,
            pnl,
            decisions: 10,
            gate_rejections: 0,
            holds: 0,
        }
    }

    #[test]
    fn the_old_simulator_summary_is_read() {
        let r = serde_json::json!({
            "total_orders": 58, "total_positions": 1,
            "summary": {"orders.total": "58"},
            "stats_pnls": {"USD": {"PnL (total)": 0.0}}
        });
        let o = old_result(&r);
        assert_eq!((o.orders, o.positions, o.pnl), (Some(58), Some(1), Some(0.0)));
    }

    #[test]
    fn a_zero_pnl_with_orders_is_the_old_venue_and_every_difference_is_named() {
        let old = OldResult {
            bars: None,
            orders: Some(58),
            positions: Some(1),
            pnl: Some(0.0),
        };
        let d = attribute(&old, &new(40, -12.5), &changes(&["fills_next_bar_open", "exits_are_orders"]));
        let orders = d.iter().find(|x| x.metric == "orders").unwrap();
        assert!(!orders.attributed_to.contains(&"fills_next_bar_open".into()));
        assert_eq!(d.len(), 2);
        assert!(d.iter().all(|x| !x.attributed_to.is_empty()));
        let pnl = d.iter().find(|x| x.metric == "pnl").unwrap();
        assert!(pnl.attributed_to.contains(&"v1_venue_reported_no_fills".into()));
    }

    #[test]
    fn a_difference_with_no_listed_mechanism_is_unattributed() {
        let old = OldResult {
            bars: None,
            orders: Some(5),
            positions: None,
            pnl: Some(3.0),
        };
        let d = attribute(&old, &new(7, 3.0), &changes(&[]));
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].metric, "orders");
        assert!(d[0].attributed_to.is_empty());
    }

    #[test]
    fn fill_timing_never_explains_an_order_count() {
        let old = OldResult {
            bars: None,
            orders: Some(333),
            positions: None,
            pnl: Some(0.0),
        };
        let d = attribute(&old, &new(10_079, -3_771.6), &changes(&["fills_next_bar_open"]));
        let orders = d.iter().find(|x| x.metric == "orders").unwrap();
        assert!(orders.attributed_to.is_empty(), "{orders:?}");
    }

    #[test]
    fn a_different_bar_count_is_measured_and_named() {
        let old = OldResult {
            bars: Some(6_022),
            orders: Some(333),
            positions: None,
            pnl: Some(0.0),
        };
        // `new` has 10 decisions; the old run saw 6 022 bars.
        let d = attribute(&old, &new(567, -3_770.1), &changes(&["fills_next_bar_open"]));
        let orders = d.iter().find(|x| x.metric == "orders").unwrap();
        assert_eq!(orders.attributed_to, vec!["bars_differ".to_string()]);
    }

    #[test]
    fn identical_results_report_no_difference() {
        let old = OldResult {
            bars: None,
            orders: Some(4),
            positions: None,
            pnl: Some(2.0),
        };
        assert!(attribute(&old, &new(4, 2.0), &changes(&["fills_next_bar_open"])).is_empty());
    }
}
