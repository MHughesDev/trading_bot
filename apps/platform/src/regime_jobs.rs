//! The daily regime labelling job and the separation test that says whether the
//! labels mean anything (SPEC §5.4, checklist 3.5, ADR-P3-02, ADR-P3-08,
//! ADR-P3-09).
//!
//! The rule tier, run on a timer: daily returns for each configured market scope
//! go in, volatility terciles come out, and `regime_causal.regime_state` holds
//! the result. That is the whole model for now, and it is deliberately the whole
//! model — a three-state filtered HMM replaces the internals later without
//! changing the table.
//!
//! Then, immediately after, the thing §5.4 says is the difference between a
//! regime model and a decoration: the out-of-sample regime-conditional
//! separation test. It runs in the same pass because the two questions share
//! their inputs, and because a labelling job whose output has never been
//! validated is exactly what §5.4 is warning about.
//!
//! ## Where the scopes come from
//!
//! `regime_causal.regime_scope`, not the config file. ADR-P3-07 refused a
//! default because "the market" is a different series for crypto, equities and
//! futures; a table with a NOT NULL `rationale` is how that refusal becomes a
//! mechanism rather than an empty list somebody eventually fills in with a guess
//! (ADR-P3-08). The config key is still read, and a symbol in it that no scope
//! covers is reported — loudly, and without being labelled, because a scope with
//! no recorded reasoning is what the ADR refused.
//!
//! The job writes **filtered** states only. There is no smoothed pass here and
//! no function in `api::knowledge` that would store one; the smoothed table
//! lives behind a schema grant this process's role does not hold (AT-42).

use std::time::Duration;

use api::knowledge::{RegimeScope, RegimeScopeStore, RegimeStore, RULE_MODEL_VERSION};
use api::regime_separation::SeparationJob;
use sqlx::PgPool;
use tracing::{error, info, warn};

/// How often the labels are recomputed.
///
/// Daily, because the input is a daily return series and the tercile boundaries
/// move only when a day is added. More often would relabel the same history with
/// the same answer; less often would leave the newest days unlabelled, and an
/// unlabelled day is one Gate 11 cannot count toward coverage.
const INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

/// The trailing window the realized volatility is measured over.
///
/// Twenty-one trading days — a month. Short enough to move when the market does,
/// long enough that a single day cannot flip the label, which matters because
/// the label is what Gate 11 counts regimes with.
const VOL_WINDOW: usize = 21;

/// How much history to label on each pass.
const LOOKBACK_DAYS: i64 = 400;

/// The tenant the separation test is run for.
///
/// Single-tenant today; when a second appears this becomes a loop over the
/// tenants that have trials, not a default that quietly means the first one.
const TENANT: &str = "default";

/// Start the daily loop.
///
/// `config_scopes` is the legacy `regime_scopes` config key. It is no longer the
/// source of truth — a scope needs an asset class, a series, an instrument list
/// and a recorded reason, and a list of bare symbols cannot carry any of that —
/// but a symbol in it that no configured scope covers is worth saying out loud,
/// because somebody wrote it there expecting labels.
pub fn spawn(pg: PgPool, clickhouse_url: String, config_scopes: Vec<String>) {
    tokio::spawn(async move {
        loop {
            run_once(&pg, &clickhouse_url, &config_scopes).await;
            tokio::time::sleep(INTERVAL).await;
        }
    });
}

async fn run_once(pg: &PgPool, clickhouse_url: &str, config_scopes: &[String]) {
    let scopes = match RegimeScopeStore::new(pg.clone()).enabled().await {
        Ok(s) => s,
        Err(e) => {
            error!(error = %e, "could not read the regime scope registry");
            return;
        }
    };
    if scopes.is_empty() {
        info!(
            "no market scopes configured in regime_causal.regime_scope; regime labelling is not \
             running, and Gates 6 and 11 have no market series to read (ADR-P3-07)"
        );
        return;
    }
    for symbol in config_scopes {
        if !scopes.iter().any(|s| {
            s.series_symbol.eq_ignore_ascii_case(symbol)
                || s.applies_to.iter().any(|a| a.eq_ignore_ascii_case(symbol))
        }) {
            warn!(
                symbol = %symbol,
                "config lists this instrument under regime_scopes but no configured scope covers \
                 it; it is not being labelled. A scope names a market and records why that series \
                 is its market — add one rather than widening an existing scope's claim"
            );
        }
    }

    for scope in &scopes {
        match daily_returns(clickhouse_url, scope).await {
            Ok(returns) if returns.len() > VOL_WINDOW => {
                let store = RegimeStore::new(pg.clone());
                match store
                    .write_rule_tier(&scope.market_scope, &returns, VOL_WINDOW)
                    .await
                {
                    Ok(n) => {
                        info!(scope = %scope.market_scope, days = n, "regime labels written");
                        separation(pg, scope).await;
                    }
                    Err(e) => {
                        error!(scope = %scope.market_scope, error = %e, "regime labels not written");
                    }
                }
            }
            Ok(returns) => warn!(
                scope = %scope.market_scope,
                days = returns.len(),
                window = VOL_WINDOW,
                "not enough history to label a regime; nothing written rather than a label from \
                 fewer days than the window"
            ),
            Err(e) => {
                error!(scope = %scope.market_scope, error = %e, "could not read the scope series");
            }
        }
    }
}

/// Run the separation test and record whatever it says.
///
/// A negative result is written exactly as readily as a positive one. That is
/// the point of the job: §5.4's standard is that an unvalidated regime model is
/// decorative, and a job that only records validations would make the platform
/// unable to discover that about itself.
async fn separation(pg: &PgPool, scope: &RegimeScope) {
    match SeparationJob::new(pg.clone())
        .run(TENANT, scope, RULE_MODEL_VERSION)
        .await
    {
        Ok(run) => info!(
            scope = %scope.market_scope,
            model_version = RULE_MODEL_VERSION,
            written = run.written,
            separates = ?run.result.separates,
            "regime separation: {}",
            run.result.verdict
        ),
        Err(e) => error!(
            scope = %scope.market_scope,
            error = %e,
            "the regime separation test did not run"
        ),
    }
}

/// The scope series' returns, through the single PIT reader.
///
/// The scope names the series; the series is read at its stored timeframe and
/// rolled up to the scope's period *through the same reader*, because the
/// platform stores minutes and a regime is a daily thing. A rollup built on the
/// PIT read can never see what a plain read cannot (`BarStore`, INV-01).
async fn daily_returns(
    clickhouse_url: &str,
    scope: &RegimeScope,
) -> anyhow::Result<Vec<(chrono::DateTime<chrono::Utc>, f64)>> {
    use backtest::BarStore;
    use domain::payloads::bar::Timeframe;

    let end = chrono::Utc::now();
    let start = end - chrono::Duration::days(LOOKBACK_DAYS);
    let base = match scope.series_base_period_secs {
        1 => Timeframe::Seconds1,
        60 => Timeframe::Minutes1,
        300 => Timeframe::Minutes5,
        900 => Timeframe::Minutes15,
        3_600 => Timeframe::Hours1,
        14_400 => Timeframe::Hours4,
        86_400 => Timeframe::Daily,
        other => anyhow::bail!(
            "scope {} reads a {other}s bar nobody stores",
            scope.market_scope
        ),
    };
    let store = BarStore::connect(clickhouse_url);
    let bucket = u32::try_from(scope.series_period_secs).unwrap_or(86_400);
    let bars = if scope.series_base_period_secs == scope.series_period_secs {
        store
            .load_bars(&scope.series_symbol, base, start, end)
            .await?
    } else {
        store
            .load_bars_bucketed(&scope.series_symbol, base, bucket, start, end)
            .await?
    };

    let mut out = Vec::with_capacity(bars.len().saturating_sub(1));
    for pair in bars.windows(2) {
        let (prev, next) = (&pair[0], &pair[1]);
        let (p0, p1) = (to_f64(prev.close), to_f64(next.close));
        if p0 > 0.0 && p1 > 0.0 {
            // The bar's *close*: the earliest instant a strategy acting on the
            // whole bar could have known its return.
            let ts = chrono::DateTime::from_timestamp_nanos(next.ts_ns);
            out.push((ts, (p1 / p0).ln()));
        }
    }
    Ok(out)
}

fn to_f64(d: rust_decimal::Decimal) -> f64 {
    use rust_decimal::prelude::ToPrimitive;
    d.to_f64().unwrap_or(0.0)
}
