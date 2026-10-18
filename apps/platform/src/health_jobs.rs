//! Hourly platform self-monitoring pass (SPEC §16.2; plan 5.3).
//!
//! The endpoint answers when someone asks. This asks on its own, because the
//! signals that matter most are the ones nobody thinks to check: an exploration
//! fraction that has quietly fallen below its floor, a leakage suite that stopped
//! running, a cited artifact that is no longer pinned. Each of those degrades
//! silently and none of them surfaces anywhere else.
//!
//! P1 breaches log at error, P2 at warn, and signals that could not be computed
//! log separately from both — a check that is broken is a different problem from
//! a check that failed, and treating them alike is how the broken one stays
//! broken.

use std::time::Duration;

use tracing::{error, info, warn};

pub fn spawn(pg: sqlx::PgPool, ch_url: String) {
    // The monitor's data-family input. Started here because `main` already
    // starts this job, and before the first pipeline publishes a bar.
    spawn_feed_freshness(pg.clone());
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(3600));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tick.tick().await;
            run_once(&pg, &ch_url).await;
        }
    });
}

pub async fn run_once(pg: &sqlx::PgPool, ch_url: &str) {
    self_monitoring_pass(pg).await;
    silence_pass(pg, ch_url).await;
    fleet_pass(pg).await;
    drill_pass(pg).await;
    legacy_marker_pass(pg).await;
}

/// The `legacy_ungated` sweep (Set T T-1.4; ADR-P2-27): any running deployment
/// with neither a decided proposal nor a marker gets the marker. A fact shown
/// beside its numbers and never a permission, so it is safe to write without
/// anyone asking. Idempotent; hourly is more than enough, and once the proposal
/// requirement is armed there are no stragglers left to find.
async fn legacy_marker_pass(pg: &sqlx::PgPool) {
    match api::legacy_marker::mark_unmarked(pg).await {
        Ok(0) => {}
        Ok(n) => info!(marked = n, "running deployments marked legacy_ungated"),
        Err(e) => warn!(error = %e, "legacy_ungated sweep failed"),
    }
}

/// The capital-ramp review (Set T T-3.2, T-3.3; ADR-P2-27), hourly.
///
/// A separate loop from [`spawn`] because its effects land on the **live** kill
/// switch the order gate reads (`KillSwitch::scale_strategy`, and a trip for a
/// size of zero), and `main` only has that handle after the gate is built.
/// `main` calls this once, beside the gate; `run_once` cannot, and a ramp that
/// applied sizes to a switch nothing consults is the defect ADR-T3-03 closed on
/// the gate's side.
///
/// A deployment with no forward recorder is flagged each pass and **not** stepped
/// down (ADR-T3-04); one with a recorder and too few observed bars in a period is
/// `NotComputable` and halves, because a recorder that produced nothing for a
/// whole period is itself a failure.
pub fn spawn_ramp(pg: sqlx::PgPool, kill: std::sync::Arc<risk::KillSwitch>) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(3600));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tick.tick().await;
            ramp_pass(&pg, &kill).await;
        }
    });
}

/// One ramp pass. A policy that cannot be read stops the pass: with no row there
/// is no policy, and nothing is judged on a default nobody chose.
pub async fn ramp_pass(pg: &sqlx::PgPool, kill: &std::sync::Arc<risk::KillSwitch>) {
    let policy = match api::ramp_worker::RampPolicy::load(
        pg,
        &backtest::gates::profile::GateProfile::paper_v1(),
    )
    .await
    {
        Ok(p) => p,
        Err(e) => return error!(error = %e, "ramp review: no readable ramp_policy — nothing judged"),
    };
    let effects = api::ramp_worker::MonitorEffects {
        pg: pg.clone(),
        kill: std::sync::Arc::clone(kill),
    };
    match api::ramp_worker::run_pass(pg, chrono::Utc::now(), &policy, &effects).await {
        Ok(r) => {
            if !r.unmeasured.is_empty() {
                warn!(
                    unmeasured = r.unmeasured.len(),
                    deployments = ?r.unmeasured,
                    "running deployments with no forward recorder: flagged, not stepped down (ADR-T3-04)"
                );
            }
            for e in &r.errors {
                warn!(error = %e, "ramp review: a deployment could not be reviewed");
            }
            info!(
                opened = r.periods_opened,
                reviewed = r.reviewed.len(),
                applied = r.applied,
                "ramp review pass"
            );
        }
        Err(e) => error!(error = %e, "ramp review could not list the running deployments"),
    }
}

/// Fleet-level reconciliation (Set T T-2.2): every tenant's forward record over
/// the declared window, judged as a fleet, and a divergence common to the quorum
/// filed as a `monitor.platform_defect` without anyone asking. Hourly is ample:
/// the window is thirty days and the quorum needs thirty bars per deployment.
async fn fleet_pass(pg: &sqlx::PgPool) {
    let led = ledger::pg::PgTrialLedger::new(pg.clone());
    let tenants = match led.tenants_async().await {
        Ok(t) => t,
        Err(e) => return warn!(error = %e, "fleet reconciliation: could not list tenants"),
    };
    let store = reconciliation::monitor::store::MonitorStore::new(pg.clone());
    for tenant in tenants {
        match reconciliation::fleet_job::assess_tenant(pg, &store, &tenant, chrono::Utc::now(), true)
            .await
        {
            Ok(a) => info!(%tenant, verdict = ?a.report.verdict, filed = ?a.filed,
                "fleet reconciliation"),
            Err(e) => warn!(%tenant, error = %e, "fleet reconciliation failed"),
        }
    }
}

/// The fail-closed drills (Set T T-2.4), unattended, weekly. Checked hourly and
/// run when any drill is a week old or has never run; each records its outcome
/// and timing, and a failure files a `monitor.platform_defect` and logs at error.
async fn drill_pass(pg: &sqlx::PgPool) {
    let store = reconciliation::monitor::store::MonitorStore::new(pg.clone());
    let last = match store.last_drill_runs().await {
        Ok(l) => l,
        Err(e) => return error!(error = %e, "drill schedule: last runs unreadable — drills not run"),
    };
    if !reconciliation::drills::suite_is_due(&last, chrono::Utc::now()) {
        return;
    }
    match reconciliation::drills::run_all(&store, true, reconciliation::drills::SCHEDULE_ACTOR).await {
        Ok(runs) => {
            let failed = runs.iter().filter(|r| !r.passed).count();
            if failed == 0 {
                info!(drills = runs.len(), "scheduled fail-closed drills passed");
            } else {
                error!(failed, drills = runs.len(), "scheduled fail-closed drills FAILED — defects filed");
            }
        }
        Err(e) => error!(error = %e, "scheduled fail-closed drills could not run"),
    }
}

/// The per-tenant self-monitoring signals.
async fn self_monitoring_pass(pg: &sqlx::PgPool) {
    // Every tenant with a ledger. A tenant with no trials has nothing to report
    // on, and inventing a clean bill of health for one would be the exact failure
    // §16.2 is about.
    let led = ledger::pg::PgTrialLedger::new(pg.clone());
    let tenants = match led.tenants_async().await {
        Ok(t) => t,
        Err(e) => return warn!(error = %e, "platform health: could not list tenants"),
    };
    for tenant in tenants {
        match api::self_monitor::health(pg, &tenant).await {
            Ok(report) => {
                for s in report.alarms() {
                    match s.severity {
                        api::self_monitor::Severity::P1 => {
                            error!(%tenant, signal = s.id, spec = s.spec_ref, detail = %s.detail,
                                "P1: platform self-monitoring alarm");
                        }
                        api::self_monitor::Severity::P2 => {
                            warn!(%tenant, signal = s.id, spec = s.spec_ref, detail = %s.detail,
                                "platform self-monitoring alarm");
                        }
                    }
                }
                for s in report.unavailable() {
                    warn!(%tenant, signal = s.id, detail = %s.detail,
                        "platform self-monitoring: signal could not be computed");
                }
                if report.healthy() {
                    info!(summary = ?api::self_monitor::HealthSummary::from(&report),
                        "platform self-monitoring clean");
                }
            }
            Err(e) => warn!(%tenant, error = %e, "platform health pass failed"),
        }
    }
}

/// The silence alarm (Set P P-0.6): the caller
/// `storage::clickhouse::census`'s module docs name.
///
/// Not tenant-scoped — a collector that died is one fact about one process, not
/// one per tenant — and the hourly tick `spawn` already owns is the right cadence,
/// since the tightest deadline any registered series declares is three hours.
///
/// [`silence_report`] never returns `Err`: every failure, including a store it
/// could not read at all, comes back as a finding whose severity says so, and
/// `emit` logs alarms at error, unjudgeable feeds at warn and quiet-but-fine feeds
/// at info — the same three-way split [`self_monitoring_pass`] uses, for the same
/// reason.
///
/// Its own function, called after the tenant pass rather than appended inside it,
/// because that pass returns early when Postgres will not list tenants. Inside its
/// body, a database hiccup would silence the silence alarm — which is the one
/// failure this whole mechanism exists to make impossible.
///
/// [`silence_report`]: storage::clickhouse::census::silence_report
async fn silence_pass(pg: &sqlx::PgPool, ch_url: &str) {
    observability::silence::emit(
        &storage::clickhouse::census::silence_report(
            pg,
            ch_url,
            chrono::Utc::now(),
            std::num::NonZeroU32::new(
                storage::clickhouse::census::REGISTRY_DEFAULT_TOLERANCE_CADENCES,
            )
            .expect("the registry default is positive"),
        )
        .await,
    );
}

// ── The live-feed freshness producer (COMP-007 §3; Set S S-7.5) ─────────────
//
// `tbot-monitor` judges how old each live feed is, and it has no ClickHouse
// client and must not grow one. So this process — the one that *observes* every
// completed minute as it publishes it to the v3 runners (`crate::live_bars`) —
// writes `monitor.feed_freshness`, one gauge row per feed, through the monitor's
// own store. It measures rather than infers: a row is written when a bar is seen,
// never from a guess about when one should have been.
//
// Every thirty seconds it also affirms the rows it owns and says whether bars
// are expected now. A US equity feed is silent every night; without that word
// the monitor would halt every equity strategy at the close and a human would
// reset each one every morning — the habit-of-ignoring-it failure §4.3 exists to
// prevent. A producer that stops affirming makes its rows *unmeasured* on the
// monitor's side, not stale: the platform being down is the lease's business.

/// How often the producer affirms its rows and reloads the asset list.
const FEED_AFFIRM_SECS: u64 = 30;

/// The venue each asset class's live pipeline streams from — the same table as
/// `pipeline_manager::CollectorBinding::for_asset_class`, which is private to
/// that module. A guard below reads that function's source and fails if the two
/// ever disagree, because a gauge row filed under the wrong venue is a feed the
/// monitor judges and no strategy depends on.
fn live_feed_venue(asset_class: &str) -> Option<&'static str> {
    match asset_class {
        "crypto_spot_cex" => Some("kraken"),
        "equity" | "etf" => Some("alpaca"),
        _ => None,
    }
}

/// Whether bars are expected from this asset class's live feed at `now`.
///
/// Continuous markets always. Session markets on an exchange trading day (the
/// gap analyser's own calendar, holidays included) between 09:30 and 16:00 New
/// York time. During that session the New York date and the UTC date agree, so
/// the trading-day test on the UTC date is exact.
fn session_open(asset_class: &str, now: chrono::DateTime<chrono::Utc>) -> bool {
    use backtest::gaps::ScheduleKind;
    let schedule = ScheduleKind::for_asset_class(asset_class);
    if matches!(schedule, ScheduleKind::Continuous) {
        return true;
    }
    let day = now
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .expect("midnight exists")
        .and_utc();
    let trading_day = backtest::gaps::analyze(
        day,
        day + chrono::Duration::days(1),
        &std::collections::HashMap::new(),
        domain::payloads::bar::Timeframe::Minutes1,
        schedule,
    )
    .expected_bars
        > 0;
    if !trading_day {
        return false;
    }
    if matches!(schedule, ScheduleKind::Weekday24h) {
        return true;
    }
    let rth = domain::instrument::TradingSchedule {
        timezone: "America/New_York".into(),
        sessions: vec![domain::instrument::TradingSession {
            open: "09:30".into(),
            close: "16:00".into(),
        }],
        has_pre_market: false,
        has_post_market: false,
    };
    reconciliation::freshness::is_within_trading_hours(now, &rth)
}

/// The initialised assets and their classes, plus the BTC-USD feed `main` keeps
/// streaming before anything is initialised.
async fn asset_classes(pg: &sqlx::PgPool) -> std::collections::HashMap<String, String> {
    let mut out: std::collections::HashMap<String, String> =
        sqlx::query_as::<_, (String, String)>("SELECT symbol, asset_class FROM asset_lifecycle")
            .fetch_all(pg)
            .await
            .unwrap_or_else(|e| {
                warn!(error = %e, "feed producer: asset_lifecycle unreadable; keeping defaults");
                Vec::new()
            })
            .into_iter()
            .collect();
    out.entry("BTC-USD".to_owned())
        .or_insert_with(|| "crypto_spot_cex".to_owned());
    out
}

fn feed_key(symbol: &str, venue: &str) -> Option<reconciliation::monitor::feed::FeedKey> {
    let venue = reconciliation::monitor::feed::Venue::named(venue).ok()?;
    reconciliation::monitor::feed::FeedKey::new(symbol, venue).ok()
}

/// Spawn the producer. Subscribes before it returns, so no bar published after
/// `spawn` is missed.
pub fn spawn_feed_freshness(pg: sqlx::PgPool) {
    let mut bars = crate::live_bars::subscribe();
    tokio::spawn(async move {
        use tokio::sync::broadcast::error::RecvError;
        let store = reconciliation::monitor::store::MonitorStore::new(pg.clone());
        let mut classes = asset_classes(&pg).await;
        let mut tick = tokio::time::interval(Duration::from_secs(FEED_AFFIRM_SECS));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                bar = bars.recv() => match bar {
                    Ok(b) => {
                        let Some(class) = classes.get(&b.instrument_id).cloned() else {
                            // Initialised since the last reload: learn it now
                            // rather than drop its first bars.
                            classes = asset_classes(&pg).await;
                            continue;
                        };
                        let Some(venue) = live_feed_venue(&class) else { continue };
                        let Some(feed) = feed_key(&b.instrument_id, venue) else { continue };
                        let now = chrono::Utc::now();
                        if let Err(e) = store
                            .observe_bar(
                                &feed,
                                reconciliation::monitor::freshness::LIVE_FEED_TIMEFRAME_SECS,
                                b.bar.available_time,
                                session_open(&class, now),
                            )
                            .await
                        {
                            warn!(instrument_id = %b.instrument_id, error = %e,
                                "feed producer: bar observed but not recorded");
                        }
                    }
                    Err(RecvError::Lagged(n)) => {
                        // Missed bars are missed observations, not stale feeds:
                        // the next bar on each feed records the newest close.
                        warn!(missed = n, "feed producer lagged the live bar hub");
                    }
                    Err(RecvError::Closed) => {
                        error!("live bar hub closed — feed freshness is no longer produced");
                        return;
                    }
                },
                _ = tick.tick() => {
                    classes = asset_classes(&pg).await;
                    let now = chrono::Utc::now();
                    let sessions: Vec<_> = classes
                        .iter()
                        .filter_map(|(symbol, class)| {
                            let venue = live_feed_venue(class)?;
                            Some((feed_key(symbol, venue)?, session_open(class, now)))
                        })
                        .collect();
                    if let Err(e) = store.affirm_feeds(&sessions).await {
                        warn!(error = %e, "feed producer: could not affirm the feed gauge");
                    }
                }
            }
        }
    });
}

#[cfg(test)]
mod feed_tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn the_venue_table_agrees_with_the_pipeline_that_streams_the_bars() {
        let src = include_str!("pipeline_manager.rs");
        for class in ["crypto_spot_cex", "equity", "etf"] {
            let arm = format!("\"{class}\" => Some(Self {{");
            let at = src
                .find(&arm)
                .unwrap_or_else(|| panic!("pipeline_manager no longer binds {class}; update live_feed_venue"));
            let rest = &src[at..];
            let v = rest.find("venue_id: \"").expect("a venue_id in the arm") + "venue_id: \"".len();
            let venue = &rest[v..v + rest[v..].find('"').expect("closing quote")];
            assert_eq!(
                live_feed_venue(class),
                Some(venue),
                "the feed producer files {class} bars under a venue its pipeline does not stream from"
            );
        }
    }

    #[test]
    fn crypto_is_always_in_session_and_equities_are_not_at_night_or_on_weekends() {
        // Wednesday 2026-10-07: 15:00 UTC is 11:00 New York, 23:00 UTC is 19:00.
        let midday = chrono::Utc.with_ymd_and_hms(2026, 10, 7, 15, 0, 0).unwrap();
        let evening = chrono::Utc.with_ymd_and_hms(2026, 10, 7, 23, 0, 0).unwrap();
        let saturday = chrono::Utc.with_ymd_and_hms(2026, 10, 10, 15, 0, 0).unwrap();
        assert!(session_open("crypto_spot_cex", evening));
        assert!(session_open("crypto_spot_cex", saturday));
        assert!(session_open("equity", midday));
        assert!(!session_open("equity", evening));
        assert!(!session_open("etf", saturday));
    }

    #[test]
    fn the_producer_is_spawned_by_the_job_main_already_starts() {
        // `main` calls `health_jobs::spawn`; the producer has no other door.
        let src = include_str!("health_jobs.rs");
        let body = &src[src.find("pub fn spawn(").expect("spawn")..];
        let body = &body[..body.find("\n}\n").expect("end of spawn")];
        assert!(body.contains("spawn_feed_freshness("), "the feed producer is built and unreached");
        let main = include_str!("main.rs");
        assert!(main.contains("health_jobs::spawn("), "main no longer starts health_jobs");
    }

    #[test]
    fn the_fleet_and_drill_passes_run_from_the_loop_that_runs() {
        let src = include_str!("health_jobs.rs");
        let body = &src[src.find("pub async fn run_once(").expect("run_once")..];
        let body = &body[..body.find("
}
").expect("end of run_once")];
        assert!(body.contains("fleet_pass(pg)"), "fleet reconciliation is built and unreached");
        assert!(body.contains("drill_pass(pg)"), "the drill schedule is built and unreached");
        assert!(
            body.contains("legacy_marker_pass(pg)"),
            "the legacy_ungated sweep is built and unreached"
        );
    }

    /// The ramp review needs the live kill switch, so it has its own loop; `main`
    /// must start it. Both halves are asserted: a loop nobody starts, and a pass
    /// the loop does not run, are the same defect.
    #[test]
    fn the_ramp_review_is_started_by_main_and_run_by_its_loop() {
        let src = include_str!("health_jobs.rs");
        let body = &src[src.find("pub fn spawn_ramp(").expect("spawn_ramp")..];
        let body = &body[..body.find("\n}\n").expect("end of spawn_ramp")];
        assert!(body.contains("ramp_pass(&pg, &kill)"), "the loop does not run the pass");
        let pass = &src[src.find("pub async fn ramp_pass(").expect("ramp_pass")..];
        assert!(pass.contains("api::ramp_worker::run_pass("), "the ramp worker is built and unreached");
        let main = include_str!("main.rs");
        assert!(
            main.contains("health_jobs::spawn_ramp("),
            "apps/platform/src/main.rs must call health_jobs::spawn_ramp(pg, kill_switch)"
        );
    }
}
