//! Loading the rows a plan's external indicators read (Set P P-3.6).
//!
//! A `macro_<series>` or `fundamental_<cik>_<tag>` indicator resolves against
//! vintaged rows the environment hands the engine (`Event::External`). This is
//! the one loader every environment calls — the backtest runner for its whole
//! window, a paper runner at start and on refresh — so a backtest and paper read
//! the same rows the same way. Every row knowable by `as_of` is loaded; which of
//! them a decision may see is the engine's choice at each bar
//! (`ExternalInputs::resolve`), so loading a later vintage can never leak it
//! into an earlier decision.

use chrono::{DateTime, Duration, Utc};
use features::external_inputs::ExternalSeries;
use strategy_engine::ExternalData;

/// How far before the first bar a macro series is read, so the observation
/// knowable at the first decision is in hand even for a quarterly series
/// published a month late.
pub const MACRO_LOOKBACK_DAYS: i64 = 200;

/// The rows for `indicators` (`Plan::external_indicators`) over a window whose
/// first and last bars close at `first_bar` and `last_bar`, as they stood at
/// `as_of`. Empty when the plan reads none.
///
/// # Errors
/// An indicator that does not resolve, the macro store or the fundamentals
/// table. A run must not proceed on rows it could not read: the indicator
/// would read missing for a reason nobody chose.
pub async fn load(
    pg: &sqlx::PgPool,
    clickhouse_url: &str,
    indicators: &[(String, u32)],
    first_bar: DateTime<Utc>,
    last_bar: DateTime<Utc>,
    as_of: DateTime<Utc>,
) -> anyhow::Result<ExternalData> {
    let mut out = ExternalData::default();
    let mut seen = std::collections::BTreeSet::new();
    for (name, bar_secs) in indicators {
        let feature = features::external_features::resolve_external(name, *bar_secs)
            .map_err(|e| anyhow::anyhow!("{name}: {e}"))?
            .ok_or_else(|| anyhow::anyhow!("{name} is not an external indicator"))?;
        for input in feature.inputs() {
            match &input.series {
                ExternalSeries::Macro { series_id } => {
                    if !seen.insert(format!("m:{series_id}")) {
                        continue;
                    }
                    let from = (first_bar - Duration::days(MACRO_LOOKBACK_DAYS)).date_naive();
                    out.macro_rows.extend(
                        storage::clickhouse::alt::macro_vintages(
                            clickhouse_url,
                            series_id,
                            from,
                            last_bar.date_naive(),
                            as_of,
                        )
                        .await?,
                    );
                }
                ExternalSeries::Fundamental { cik, tag } => {
                    if !seen.insert(format!("f:{cik}:{tag}")) {
                        continue;
                    }
                    out.facts.extend(
                        storage::postgres::fundamentals::facts(pg, *cik, tag, as_of).await?,
                    );
                }
            }
        }
    }
    Ok(out)
}
