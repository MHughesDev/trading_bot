//! Nightly decision parity (Set U U-8.2; ADR-0047 decision 2; FEAT-007 §11).
//!
//! Once a day, every v3 deployment that decided anything on paper in the
//! previous UTC day has its decision journal replayed through the engine the
//! backtest builds ([`backtest::replay`]), compiled the way a backtest compiles
//! it. Every replayed segment's verdict is kept (`strategy_parity_check`,
//! migration 0149).
//!
//! A decision that comes out differently is an engine-boundary defect: the
//! backtest would not have decided what paper decided. It is a P1, raised at
//! `error` — the severity of a feature-consistency code drift
//! (`feature_jobs`, INV-14). A segment that cannot be replayed (its genesis aged
//! out, or it predates replay inputs) is a warning and never counted as a match.

use std::sync::Arc;
use std::time::Duration;

use backtest::replay::{self, RecordedStep, SegmentReport, Verdict};
use chrono::{DateTime, Utc};
use storage::postgres::decision_journal::{self as journal, JournalRecord, ParityCheck};
use strategy_engine::Plan;
use tracing::{error, info, warn};
use uuid::Uuid;

pub fn spawn(pg: sqlx::PgPool) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(24 * 3600));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tick.tick().await;
            run_once(&pg, Utc::now()).await;
        }
    });
}

/// The previous UTC day, as of `now`.
#[must_use]
pub fn yesterday(now: DateTime<Utc>) -> (DateTime<Utc>, DateTime<Utc>) {
    let end = now
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .map_or(now, |d| d.and_utc());
    (end - chrono::Duration::days(1), end)
}

/// Check every deployment that decided in the previous UTC day.
pub async fn run_once(pg: &sqlx::PgPool, now: DateTime<Utc>) {
    let (from, to) = yesterday(now);
    let deployments = match journal::decided_between(pg, from, to).await {
        Ok(d) => d,
        Err(e) => return warn!(error = %e, "decision parity: could not list deployments"),
    };
    if deployments.is_empty() {
        return info!(%from, %to, "decision parity: no paper decisions to replay");
    }
    let registry = backtest::models::RegistrySnapshot::load_or_empty(pg).await;
    for id in deployments {
        let (revision, plan) = match storage::postgres::deployments::get(pg, id).await {
            Ok(Some(row)) => {
                let plan = serde_json::from_value(row.definition_json)
                    .map_err(|e| format!("the recorded graph does not parse: {e}"))
                    .and_then(|g| {
                        backtest::runner::compile_for_run(
                            &g,
                            &backtest::run::ParamMap::new(),
                            &registry,
                        )
                    })
                    .map(Arc::new);
                (row.revision_hash, plan)
            }
            Ok(None) => {
                warn!(deployment = %id, "decision parity: journal for a deployment with no record");
                continue;
            }
            Err(e) => {
                warn!(deployment = %id, error = %e, "decision parity: could not read the deployment");
                continue;
            }
        };
        let records = match journal::for_deployment(pg, id).await {
            Ok(r) => r,
            Err(e) => {
                warn!(deployment = %id, error = %e, "decision parity: could not read the journal");
                continue;
            }
        };
        for c in check(id, &revision, plan, &records, from, to) {
            alert(&c);
            if let Err(e) = journal::record_parity(pg, &c).await {
                error!(deployment = %id, error = %e, "decision parity: verdict could not be recorded");
            }
        }
    }
}

/// A journal record as the replay reads it.
#[must_use]
pub fn recorded(r: &JournalRecord) -> RecordedStep {
    RecordedStep {
        seq: r.seq,
        digest: r.digest.clone(),
        inputs: r.inputs.clone(),
        trace: r.trace.clone(),
    }
}

/// Replay `records` (one deployment's, in `seq` order) and keep the verdict of
/// every segment with a decision in `[from, to)`.
#[must_use]
pub fn check(
    deployment_id: Uuid,
    revision: &str,
    plan: Result<Arc<Plan>, String>,
    records: &[JournalRecord],
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Vec<ParityCheck> {
    let in_window = |r: &SegmentReport| {
        records.iter().any(|x| {
            x.seq >= r.first_seq && x.seq <= r.last_seq && x.decided_at >= from && x.decided_at < to
        })
    };
    let reports: Vec<SegmentReport> = match plan {
        Ok(plan) => {
            let steps: Vec<RecordedStep> = records.iter().map(recorded).collect();
            replay::replay(&plan, &steps)
        }
        Err(why) => match (records.first(), records.last()) {
            (Some(a), Some(b)) => vec![SegmentReport {
                first_seq: a.seq,
                last_seq: b.seq,
                verdict: Verdict::Unreplayable {
                    reason: format!("the deployment's graph does not compile today: {why}"),
                },
            }],
            _ => Vec::new(),
        },
    };
    reports
        .into_iter()
        .filter(in_window)
        .map(|r| {
            let mut c = ParityCheck {
                deployment_id,
                revision: revision.to_owned(),
                window_start: from,
                window_end: to,
                first_seq: r.first_seq,
                last_seq: r.last_seq,
                verdict: String::new(),
                decisions: None,
                diverged_seq: None,
                field: None,
                recorded: None,
                replayed: None,
                reason: None,
            };
            match r.verdict {
                Verdict::Identical { decisions } => {
                    c.verdict = "identical".into();
                    c.decisions = Some(i64::try_from(decisions).unwrap_or(i64::MAX));
                }
                Verdict::Diverged(d) => {
                    c.verdict = "diverged".into();
                    c.diverged_seq = Some(d.seq);
                    c.field = Some(d.field);
                    c.recorded = Some(d.recorded);
                    c.replayed = Some(d.replayed);
                }
                Verdict::Unreplayable { reason } => {
                    c.verdict = "unreplayable".into();
                    c.reason = Some(reason);
                }
            }
            c
        })
        .collect()
}

/// How loudly a verdict is raised.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    /// Paper and the backtest's engine decided differently.
    P1,
    Warning,
    Info,
}

/// Raise a verdict. Returns its severity.
pub fn alert(c: &ParityCheck) -> Severity {
    match c.verdict.as_str() {
        "diverged" => {
            error!(
                deployment = %c.deployment_id,
                revision = %c.revision,
                seq = ?c.diverged_seq,
                field = ?c.field,
                recorded = ?c.recorded,
                replayed = ?c.replayed,
                "decision parity: P1 — paper decided differently from the engine the backtest builds (engine-boundary defect)"
            );
            Severity::P1
        }
        "unreplayable" => {
            warn!(deployment = %c.deployment_id, reason = ?c.reason, first_seq = c.first_seq, last_seq = c.last_seq, "decision parity: a segment could not be replayed");
            Severity::Warning
        }
        _ => {
            info!(deployment = %c.deployment_id, decisions = ?c.decisions, "decision parity: every decision replayed identically");
            Severity::Info
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use backtest::replay::{Genesis, JournalInputs};
    use domain::strategy_def::params::ParamValues;
    use strategy_engine::{
        Bar, Engine, Event, InstrumentSpec, PortfolioView, StdRegistries, Step, VenueCaps,
    };

    const T0: i64 = 1_767_225_600_000_000_000; // 2026-01-01T00:00:00Z, ns

    fn plan() -> Arc<Plan> {
        let g = serde_json::from_value(serde_json::json!({
          "format": "tbot.strategy/3", "kind": "trading", "name": "parity",
          "asset_class": "crypto_spot_cex", "timeframe": "1m",
          "nodes": [
            {"id":"f","op":"ema","args":{"x":"close","n":3}},
            {"id":"s","op":"ema","args":{"x":"close","n":8}},
            {"id":"up","op":"gt","args":{"a":"f","b":"s"}}
          ],
          "decide": {"rules": [
            {"when":"up","then":{"kind":"target","size":{"mode":"units","x":1}}}
          ]}
        }))
        .unwrap();
        Arc::new(
            backtest::runner::compile_for_run(&g, &backtest::run::ParamMap::new(), &StdRegistries::new())
                .unwrap(),
        )
    }

    fn spec() -> InstrumentSpec {
        InstrumentSpec {
            instrument: "PARITY-USD".into(),
            lot_size: rust_decimal::Decimal::new(1, 8),
            tick_size: rust_decimal::Decimal::new(1, 2),
            min_qty: rust_decimal::Decimal::new(1, 8),
            caps: VenueCaps::all(),
        }
    }

    fn bar(i: i64) -> Bar {
        let c = 100.0 + 5.0 * ((i as f64) / 4.0).sin();
        Bar {
            close_ns: T0 + (i + 1) * 60_000_000_000,
            open: c,
            high: c + 1.0,
            low: c - 1.0,
            close: c,
            volume: 1.0,
        }
    }

    /// A journal written the way the paper runner writes one: a genesis on the
    /// first record, every decision with the event and view it saw.
    fn journal(id: Uuid, bars: i64) -> Vec<JournalRecord> {
        let plan = plan();
        let genesis = Genesis {
            checkpoint: None,
            history: (0..5).map(|i| (0, bar(i))).collect(),
        };
        let mut engine: Engine = genesis.build(Arc::clone(&plan), &spec()).unwrap();
        let mut genesis = Some(genesis);
        let mut out = Vec::new();
        let view = PortfolioView::default();
        for i in 5..5 + bars {
            let event = Event::Bar {
                input: 0,
                bar: bar(i),
                warmup: false,
            };
            let Step::Decided(trace) = engine.on_event(event.clone(), &view, &spec()).unwrap()
            else {
                panic!("a bar-close strategy decides on every bar");
            };
            let inputs = JournalInputs {
                event,
                view: view.clone(),
                spec: spec(),
                preceding: Vec::new(),
                genesis: genesis.take(),
            };
            out.push(JournalRecord {
                deployment_id: id,
                seq: i - 4,
                tick: i64::try_from(trace.tick).unwrap(),
                decided_at: DateTime::from_timestamp_nanos(trace.at_ns),
                cause: "bar_close".into(),
                digest: trace.digest(),
                input_digest: "0".repeat(64),
                inputs: serde_json::to_value(&inputs).unwrap(),
                trace: serde_json::to_value(&*trace).unwrap(),
                gate_verdicts: serde_json::json!([]),
                missing_reasons: serde_json::json!({}),
            });
        }
        out
    }

    fn day() -> (DateTime<Utc>, DateTime<Utc>) {
        yesterday(DateTime::from_timestamp_nanos(T0) + chrono::Duration::days(1))
    }

    #[test]
    fn yesterday_is_the_previous_utc_day() {
        let (from, to) = yesterday(DateTime::from_timestamp_nanos(T0) + chrono::Duration::hours(30));
        assert_eq!(from, DateTime::from_timestamp_nanos(T0));
        assert_eq!(to - from, chrono::Duration::days(1));
    }

    #[test]
    fn an_untouched_journal_is_identical_and_raises_nothing() {
        let id = Uuid::nil();
        let records = journal(id, 30);
        let (from, to) = day();
        let checks = check(id, "rev", Ok(plan()), &records, from, to);
        assert_eq!(checks.len(), 1);
        assert_eq!(checks[0].verdict, "identical", "{checks:#?}");
        assert_eq!(checks[0].decisions, Some(30));
        assert_eq!(alert(&checks[0]), Severity::Info);
    }

    #[test]
    fn a_seeded_difference_is_a_p1_naming_the_record_and_field() {
        let id = Uuid::nil();
        let mut records = journal(id, 30);
        // Seed a difference: record 12 says the fast EMA was something else.
        let r = &mut records[11];
        r.trace["values"]["f"] = serde_json::json!({"num": 1.0});
        r.digest = "f".repeat(64);
        let (from, to) = day();
        let checks = check(id, "rev", Ok(plan()), &records, from, to);
        assert_eq!(checks[0].verdict, "diverged", "{checks:#?}");
        assert_eq!(checks[0].diverged_seq, Some(12));
        assert!(
            checks[0].field.as_deref().is_some_and(|f| f.starts_with("values.f")),
            "{checks:#?}"
        );
        assert_eq!(alert(&checks[0]), Severity::P1);
    }

    #[test]
    fn a_perturbed_engine_diverges_at_its_first_different_decision() {
        let id = Uuid::nil();
        let records = journal(id, 30);
        // The same strategy compiled with a different slow period.
        let g = serde_json::from_value(serde_json::json!({
          "format": "tbot.strategy/3", "kind": "trading", "name": "parity",
          "asset_class": "crypto_spot_cex", "timeframe": "1m",
          "nodes": [
            {"id":"f","op":"ema","args":{"x":"close","n":3}},
            {"id":"s","op":"ema","args":{"x":"close","n":9}},
            {"id":"up","op":"gt","args":{"a":"f","b":"s"}}
          ],
          "decide": {"rules": [
            {"when":"up","then":{"kind":"target","size":{"mode":"units","x":1}}}
          ]}
        }))
        .unwrap();
        let perturbed = Arc::new(
            strategy_engine::compile(&g, &StdRegistries::new(), &ParamValues::new()).unwrap(),
        );
        let (from, to) = day();
        let checks = check(id, "rev", Ok(perturbed), &records, from, to);
        assert_eq!(checks[0].verdict, "diverged", "{checks:#?}");
        assert_eq!(checks[0].diverged_seq, Some(1));
        assert!(
            checks[0].field.as_deref().is_some_and(|f| f.starts_with("values.s")),
            "{checks:#?}"
        );
    }

    #[test]
    fn a_graph_that_no_longer_compiles_is_unreplayable_not_a_match() {
        let id = Uuid::nil();
        let records = journal(id, 3);
        let (from, to) = day();
        let checks = check(id, "rev", Err("unknown operator".into()), &records, from, to);
        assert_eq!(checks[0].verdict, "unreplayable");
        assert_eq!(alert(&checks[0]), Severity::Warning);
    }

    #[test]
    fn segments_outside_the_day_are_not_reported() {
        let id = Uuid::nil();
        let records = journal(id, 3);
        let (from, to) = day();
        let later = check(
            id,
            "rev",
            Ok(plan()),
            &records,
            from + chrono::Duration::days(2),
            to + chrono::Duration::days(2),
        );
        assert!(later.is_empty());
    }

    /// U-8.2: the job is registered at boot — a source scan, for the reason
    /// `fill_stream_tests` gives: from in here, a job nobody spawns is
    /// indistinguishable from one that runs.
    #[test]
    fn the_parity_job_is_spawned_at_boot() {
        let main = include_str!("main.rs");
        assert!(main.contains("\nmod parity_jobs;"), "main declares the module");
        let at = main
            .find("parity_jobs::spawn(pg.clone());")
            .expect("main must spawn the nightly parity job");
        let feature = main
            .find("feature_jobs::spawn(")
            .expect("the feature-consistency job is the precedent this runs beside");
        // Beside its precedent, in the same unconditional block.
        assert!(at.abs_diff(feature) < 600, "spawned beside feature_jobs");
        let line_start = main[..at].rfind('\n').map_or(0, |i| i + 1);
        assert!(
            !main[line_start..at].contains("//"),
            "the spawn is not commented out"
        );
    }
}
