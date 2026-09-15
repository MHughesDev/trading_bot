//! The out-of-sample regime-conditional separation test, run against the
//! platform's own data (SPEC §5.4).
//!
//! §5.4: *"Validation is out-of-sample regime-conditional performance
//! separation, or the regime model is decorative."* This is the harness that
//! answers it on this installation rather than on a fixture, and prints the
//! numbers so they can be written down whatever they say.
//!
//! ## What it uses, and what it does not
//!
//! **The labels are the platform's own.** They are written by
//! `api::knowledge::RegimeStore::write_rule_tier` — the same call
//! `apps/platform::regime_jobs` makes — from the scope configured in
//! `regime_causal.regime_scope`, read through `BarStore`, the single
//! point-in-time reader. No second labeller.
//!
//! **The features are the platform's own.** EMA and RSI come from
//! `features::runtime`, which is the one windowed implementation per feature
//! (INV-14). This harness contains no indicator arithmetic; it contains a
//! *rule* over features, which is a different thing.
//!
//! **The return series are reconstructions, and that is a limitation, not a
//! detail.** `mlops.trial_return_series` is empty on this installation, so there
//! are no recorded out-of-sample series to split. The honest substitute is the
//! strategy families this platform's simulator can express, evaluated on the
//! platform's own bars, with the parameter chosen on the first half of the
//! sample and performance measured only on the second. The separation job is
//! given these through `run_on`, the same path the ledger series take, and the
//! answer is recorded under its own digest like any other.
//!
//! Run it with:
//! ```text
//! DATABASE_URL=postgres://trading:trading@localhost:5432/trading \
//! CLICKHOUSE_URL=http://trading:trading@localhost:8123/trading \
//! cargo test -p api --test regime_separation_live -- --ignored --nocapture --test-threads=1
//! ```

use std::collections::BTreeMap;

use api::knowledge::{RegimeScope, RegimeScopeStore, RegimeStore, RULE_MODEL_VERSION};
use api::regime_separation::SeparationJob;
use chrono::{DateTime, Duration, Utc};
use features::FeatureRow;
use sqlx::PgPool;

/// The labeller's window, and the harness must not use a different one.
const VOL_WINDOW: usize = 21;

/// How much history to ask for. More than exists, deliberately: the point is to
/// use everything the platform has.
const LOOKBACK_DAYS: i64 = 1_200;

fn pg_url() -> String {
    std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://trading:trading@localhost:5432/trading".into())
}

fn ch_url() -> String {
    std::env::var("CLICKHOUSE_URL")
        .unwrap_or_else(|_| "http://trading:trading@localhost:8123/trading".into())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires live Postgres and ClickHouse"]
async fn the_regime_model_is_validated_or_it_is_decorative() {
    let pg = PgPool::connect(&pg_url()).await.expect("postgres reachable");

    let scopes = RegimeScopeStore::new(pg.clone())
        .enabled()
        .await
        .expect("the scope registry is readable");
    assert!(
        !scopes.is_empty(),
        "no market scope is configured, so there is nothing to validate. That is ADR-P3-07's \
         state, not a test failure — configure a scope in regime_causal.regime_scope first."
    );

    for scope in &scopes {
        println!("\n=== scope {} ({}) ===", scope.market_scope, scope.series_symbol);
        println!("rationale: {}", scope.rationale);

        let bars = daily_bars(&ch_url(), scope).await;
        println!("bars: {} daily observations", bars.len());
        if bars.len() <= VOL_WINDOW + 2 {
            println!(
                "SKIPPED: {} bars is not more than the {VOL_WINDOW}-day window; the labeller \
                 writes nothing and so does this",
                bars.len()
            );
            continue;
        }

        // 1. Label, exactly as the platform's own job does.
        let returns = log_returns(&bars);
        let store = RegimeStore::new(pg.clone());
        let written = store
            .write_rule_tier(&scope.market_scope, &returns, VOL_WINDOW)
            .await
            .expect("labels written");
        println!("labels written: {written}");

        let first = returns.first().map(|(t, _)| *t).unwrap_or_else(Utc::now);
        let last = returns.last().map(|(t, _)| *t).unwrap_or_else(Utc::now);
        let history = store
            .history(&scope.market_scope, RULE_MODEL_VERSION, first, last)
            .await
            .expect("history readable");
        let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
        for o in &history {
            *counts.entry(o.label.as_str()).or_default() += 1;
        }
        println!("label distribution: {counts:?}");

        // 2. The strategies. Parameter chosen in-sample, measured out-of-sample.
        let split = bars.len() / 2;
        let series = candidate_series(&bars, split);
        for (name, points) in &series {
            let (mean, sharpe) = summary(points);
            println!(
                "  {name}: {} OOS days, mean {mean:+.5}, Sharpe {sharpe:+.2}",
                points.len()
            );
        }

        // 3. The test, through the job that records it.
        let run = SeparationJob::new(pg.clone())
            .run_on("default", scope, RULE_MODEL_VERSION, &series)
            .await
            .expect("the separation test runs");

        println!("\n--- SEPARATION ---");
        println!("recorded: {} (digest {})", run.written, run.input_digest);
        println!(
            "series {}, observations {}, labelled {}, regimes {}",
            run.result.series_count,
            run.result.observation_count,
            run.result.labelled_count,
            run.result.regimes_present
        );
        for s in &run.result.per_regime {
            println!(
                "  {:<9} n={:<5} mean={:+.5} sd={:.5} sharpe={:+.3}",
                s.label, s.n, s.mean, s.sd, s.sharpe
            );
        }
        println!("spread_mean    : {:?}", run.result.spread_mean);
        println!("spread_sharpe  : {:?}", run.result.spread_sharpe);
        println!("kruskal H / p  : {:?} / {:?}", run.result.kruskal_h, run.result.kruskal_p);
        println!("rotation p     : {:?}", run.result.permutation_p);
        println!("alpha          : {}", run.result.alpha);
        println!("separates      : {:?}", run.result.separates);
        println!("verdict        : {}", run.result.verdict);
        println!("--- END ---\n");

        // The assertion is deliberately *not* "it separates". A separation test
        // that fails the build when the regime model turns out to be decorative
        // is a test nobody will ever be allowed to believe. What must hold is
        // that the answer was computed and recorded — whatever it is.
        assert!(
            run.result.separates.is_some() || run.result.verdict.contains("no comparison"),
            "the test neither answered nor said why: {}",
            run.result.verdict
        );
    }
    pg.close().await;
}

/// The scope's series as daily bars, through the single PIT reader.
async fn daily_bars(ch: &str, scope: &RegimeScope) -> Vec<FeatureRow> {
    use backtest::BarStore;
    use domain::payloads::bar::Timeframe;
    use rust_decimal::prelude::ToPrimitive;

    // The canonical table and the identity dims are what `BarStore` reads. On a
    // box where the platform has not booted since they were added they do not
    // exist, so the harness applies the same migration the platform applies —
    // it does not read around it.
    storage::clickhouse::migrate::run_migrations(ch)
        .await
        .expect("clickhouse migrations apply");
    let pg = PgPool::connect(&pg_url()).await.expect("postgres reachable");
    let identity = storage::identity::PgIdentityService::new(pg.clone(), ch.to_string());
    identity.sync_dims().await.expect("identity dims sync");
    storage::identity::install(identity);
    let report = storage::clickhouse::canonical::migrate_to_canonical(ch, &pg)
        .await
        .expect("canonical migration");
    if report.rows_copied > 0 {
        println!(
            "canonical migration copied {} rows in {} chunks",
            report.rows_copied, report.chunks_copied
        );
    }
    pg.close().await;

    let end = Utc::now();
    let start = end - Duration::days(LOOKBACK_DAYS);
    let base = match scope.series_base_period_secs {
        60 => Timeframe::Minutes1,
        300 => Timeframe::Minutes5,
        900 => Timeframe::Minutes15,
        3_600 => Timeframe::Hours1,
        14_400 => Timeframe::Hours4,
        86_400 => Timeframe::Daily,
        other => panic!("scope reads a {other}s bar nobody stores"),
    };
    let store = BarStore::connect(ch);
    let bucket = u32::try_from(scope.series_period_secs).unwrap_or(86_400);
    let bars = if scope.series_base_period_secs == scope.series_period_secs {
        store.load_bars(&scope.series_symbol, base, start, end).await
    } else {
        store.load_bars_bucketed(&scope.series_symbol, base, bucket, start, end).await
    }
    .expect("bars read");

    bars.into_iter()
        .map(|b| FeatureRow {
            ts_ns: b.ts_ns,
            open: b.open.to_f64().unwrap_or(0.0),
            high: b.high.to_f64().unwrap_or(0.0),
            low: b.low.to_f64().unwrap_or(0.0),
            close: b.close.to_f64().unwrap_or(0.0),
            volume: b.volume.to_f64().unwrap_or(0.0),
        })
        .filter(|r| r.close > 0.0)
        .collect()
}

fn log_returns(bars: &[FeatureRow]) -> Vec<(DateTime<Utc>, f64)> {
    bars.windows(2)
        .filter(|w| w[0].close > 0.0 && w[1].close > 0.0)
        .map(|w| {
            (
                DateTime::from_timestamp_nanos(w[1].ts_ns),
                (w[1].close / w[0].close).ln(),
            )
        })
        .collect()
}

/// The candidate strategies' out-of-sample return series.
///
/// Two families, because two is what this platform's simulator expresses: an
/// EMA crossover and an RSI mean reversion. For each, the parameter is chosen on
/// the first half of the sample by in-sample mean return, and the series handed
/// to the separation test is the second half only. Buy-and-hold is included
/// unparameterised, as the thing every other result has to beat.
///
/// The indicators come from `features::runtime` — the one implementation
/// (INV-14). What lives here is the position rule.
fn candidate_series(
    bars: &[FeatureRow],
    split: usize,
) -> Vec<(String, Vec<(DateTime<Utc>, f64)>)> {
    let mut out = Vec::new();

    let ema_params = [(5_u32, 20_u32), (10, 50), (20, 100)];
    if let Some((fast, slow)) = best(&ema_params, |(f, s)| {
        mean(&ema_cross(bars, *f, *s)[..split.min(bars.len())])
    }) {
        let full = ema_cross(bars, *fast, *slow);
        out.push((format!("ema_cross_{fast}_{slow}"), oos(bars, &full, split)));
    }

    let rsi_params = [7_u32, 14, 21];
    if let Some(p) = best(&rsi_params, |p| mean(&rsi_reversion(bars, **p)[..split.min(bars.len())]))
    {
        let full = rsi_reversion(bars, **p);
        out.push((format!("rsi_reversion_{p}"), oos(bars, &full, split)));
    }

    let hold: Vec<f64> = std::iter::once(0.0)
        .chain(bars.windows(2).map(|w| w[1].close / w[0].close - 1.0))
        .collect();
    out.push(("buy_and_hold".to_string(), oos(bars, &hold, split)));

    out
}

/// One position-weighted return per bar: the signal is read at bar `i` and the
/// return it earns is bar `i + 1`'s. A signal that earned bar `i`'s own return
/// would be the whole lookahead problem in one line.
fn ema_cross(bars: &[FeatureRow], fast: u32, slow: u32) -> Vec<f64> {
    let f = column(bars, &format!("ema_{fast}"));
    let s = column(bars, &format!("ema_{slow}"));
    let mut out = vec![0.0; bars.len()];
    for i in 1..bars.len() {
        let pos = match (f[i - 1], s[i - 1]) {
            (Some(a), Some(b)) if a > b => 1.0,
            (Some(a), Some(b)) if a < b => -1.0,
            _ => 0.0,
        };
        out[i] = pos * (bars[i].close / bars[i - 1].close - 1.0);
    }
    out
}

fn rsi_reversion(bars: &[FeatureRow], period: u32) -> Vec<f64> {
    let r = column(bars, &format!("rsi_{period}"));
    let mut out = vec![0.0; bars.len()];
    for i in 1..bars.len() {
        let pos = match r[i - 1] {
            Some(v) if v < 30.0 => 1.0,
            Some(v) if v > 70.0 => -1.0,
            _ => 0.0,
        };
        out[i] = pos * (bars[i].close / bars[i - 1].close - 1.0);
    }
    out
}

fn column(bars: &[FeatureRow], name: &str) -> Vec<Option<f64>> {
    let f = features::runtime::feature(name).expect("a registered feature");
    features::runtime::backfill_column(f.as_ref(), bars)
}

fn oos(bars: &[FeatureRow], full: &[f64], split: usize) -> Vec<(DateTime<Utc>, f64)> {
    bars.iter()
        .zip(full)
        .skip(split)
        .map(|(b, r)| (DateTime::from_timestamp_nanos(b.ts_ns), *r))
        .collect()
}

fn best<'a, T>(items: &'a [T], score: impl Fn(&&'a T) -> f64) -> Option<&'a T> {
    items
        .iter()
        .max_by(|a, b| score(a).partial_cmp(&score(b)).unwrap_or(std::cmp::Ordering::Equal))
}

fn mean(v: &[f64]) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.iter().sum::<f64>() / v.len() as f64
}

fn summary(points: &[(DateTime<Utc>, f64)]) -> (f64, f64) {
    let v: Vec<f64> = points.iter().map(|(_, r)| *r).collect();
    let m = mean(&v);
    if v.len() < 2 {
        return (m, 0.0);
    }
    let sd = (v.iter().map(|r| (r - m).powi(2)).sum::<f64>() / (v.len() as f64 - 1.0)).sqrt();
    (m, if sd > 0.0 { m / sd * 365.25_f64.sqrt() } else { 0.0 })
}
