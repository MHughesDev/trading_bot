//! The coverage census job (Set P, P-0.1).
//!
//! Walks every `(instrument, venue, timeframe)` the point-in-time reader can see,
//! computes [`dataplane::census`] over it, and appends one row per series to
//! `dataplane.data_coverage_census` (migration 0097). It writes nothing to the bars
//! themselves and it reads them only through [`BarStore`], so the census sees
//! exactly what a strategy could and no more.
//!
//! Deterministic: two runs over an unchanged database differ only in `as_of`. The
//! series are enumerated in a fixed order and every count is an integer.
//!
//! The census is a time series, not a gauge. Re-running appends; migration 0097's
//! immutability trigger refuses an overwrite. That is deliberate — watching depth
//! accrue is the only way a dead collector ever announces itself, and a table that
//! kept only the latest reading could not show it (Set P §8).

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use dataplane::calendar::{Calendar, CalendarVersion};
use dataplane::census::{census, CensusBar, CoverageCensus, FloorVerdict};
use tracing::error;

use crate::store::BarStore;

/// How a series accrues: backfillable history, or a forward-only clock whose holes
/// are permanent (`artifact.md` FM-11).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeriesKind {
    Historical,
    Forward,
}

impl SeriesKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Historical => "historical",
            Self::Forward => "forward",
        }
    }
}

/// One census row, ready to persist.
#[derive(Clone, Debug)]
pub struct CensusRecord {
    pub census: CoverageCensus,
    pub kind: SeriesKind,
    /// Age of the newest row at `as_of`, whole seconds. `None` for an empty series.
    pub last_row_age_secs: Option<i64>,
}

impl CensusRecord {
    #[must_use]
    fn new(c: CoverageCensus, kind: SeriesKind) -> Self {
        let last_row_age_secs = dataplane::census::staleness(c.last_bar_ns, c.period_secs, c.as_of)
            .map(|d| d.num_seconds());
        Self {
            census: c,
            kind,
            last_row_age_secs,
        }
    }
}

/// Take a census of every series the reader can see at `as_of`.
///
/// Series are enumerated from [`BarStore::list_coverage`] and processed in its
/// order, which is `(symbol, venue, period)` — fixed, so the output order is too.
///
/// `calendar_for` supplies the trading calendar per venue. Expected-bar counts come
/// from it, so a session venue's overnight close is not reported as a hole. A venue
/// it does not know falls back to 24/7, which *over*-states the expected count and
/// therefore under-states coverage — the safe direction for a number whose job is
/// to stop people believing the platform has more history than it does.
///
/// # Errors
/// Backend failures from the reader.
pub async fn take_census<F>(
    store: &BarStore,
    as_of: DateTime<Utc>,
    calendar_for: F,
) -> anyhow::Result<Vec<CensusRecord>>
where
    F: Fn(&str) -> Calendar,
{
    let series = store.list_coverage().await?;
    let mut out = Vec::with_capacity(series.len());
    for s in series {
        let period_secs = period_secs_of(&s.timeframe);
        let bars = store
            .census_bars(&s.instrument_id, &s.venue, period_secs)
            .await?;
        let cal = calendar_for(&s.venue);
        let c = census(
            &s.instrument_id,
            &s.venue,
            &s.timeframe,
            period_secs,
            &bars,
            &cal,
            as_of,
        );
        // A number on a page nobody reads is not a control. A series holding rows
        // that claim an observed knowledge time their source could not have
        // supported means something is still writing them, and it means this
        // series' observed count — the only count Gate 15 may draw on — is
        // overstated by exactly that much (INV-01, ADR-P2-24, D-P3).
        if let Some(alarm) = c.provenance_alarm() {
            error!(
                instrument = %alarm.instrument,
                venue = %alarm.venue,
                timeframe = %alarm.timeframe,
                unflagged_sentinel_bars = alarm.unflagged_sentinel_bars,
                observed_knowledge_bars = alarm.observed_knowledge_bars,
                defensibly_observed_bars = alarm.defensibly_observed_bars(),
                "{alarm}"
            );
        }
        out.push(CensusRecord::new(c, SeriesKind::Historical));
    }
    Ok(out)
}

/// Every provenance alarm this census raised.
///
/// Returned as data so a caller can fail a job, page an operator, or refuse to
/// promote a dataset built over the offending series — rather than having to
/// re-derive the condition from the persisted rows.
#[must_use]
pub fn provenance_alarms(records: &[CensusRecord]) -> Vec<dataplane::census::ProvenanceAlarm> {
    records
        .iter()
        .filter_map(|r| r.census.provenance_alarm())
        .collect()
}

/// Rows across every series that may honestly be offered to Gate 15 as forward
/// evidence (D-P3).
///
/// This is the platform's entire stock of admissible forward evidence, and it is a
/// single number on purpose: it is the one an operator should be able to ask for.
#[must_use]
pub fn defensibly_observed_bars(records: &[CensusRecord]) -> u64 {
    records
        .iter()
        .map(|r| r.census.defensibly_observed_bars())
        .sum()
}

/// Census rows for the forward-only series (P-0.6 item 3), so the coverage page
/// shows the forward clock beside the historical one.
///
/// A forward series has no calendar of "expected" rows in the bar sense — it has a
/// declared cadence and a last-seen row. It is handed here as its observation times
/// so the same gap arithmetic applies, and every one of its rows is observed by
/// construction: a forward-only series with a derived knowledge time would be a
/// contradiction, and is refused rather than written.
///
/// # Errors
/// A forward observation carrying a derived knowledge time.
pub fn forward_census(
    series: &str,
    venue: &str,
    cadence_secs: u32,
    observations: &[CensusBar],
    as_of: DateTime<Utc>,
) -> anyhow::Result<CensusRecord> {
    if let Some(bad) = observations.iter().find(|o| o.knowledge_derived) {
        anyhow::bail!(
            "forward series {series}@{venue} carries a derived knowledge_time at open_ns {}: a forward-only \
             series is observed by construction; a derived stamp means it was backfilled and is not forward evidence \
             (Set P D-P3)",
            bad.open_ns
        );
    }
    if let Some(bad) = observations.iter().find(|o| o.unflagged_sentinel) {
        anyhow::bail!(
            "forward series {series}@{venue} carries an observation at open_ns {} whose knowledge_time is \
             earlier than its source could have published it, with no BACKFILLED_KNOWLEDGE_TIME flag. \
             A forward series' whole value is that its receipts are real; an unsupportable one is refused \
             rather than written, because it would be counted as Gate 15 evidence (Set P D-P3, ADR-P2-24)",
            bad.open_ns
        );
    }
    let cal = Calendar::Continuous {
        version: CalendarVersion::crypto_24_7(),
    };
    let tf = cadence_key(cadence_secs);
    let c = census(series, venue, &tf, cadence_secs, observations, &cal, as_of);
    Ok(CensusRecord::new(c, SeriesKind::Forward))
}

/// Append census rows. Never updates; migration 0097 refuses that.
///
/// # Errors
/// Backend failures, or a constraint refusing a row that cannot state its
/// provenance.
pub async fn persist(pg: &sqlx::PgPool, records: &[CensusRecord]) -> anyhow::Result<u64> {
    let mut n = 0u64;
    for r in records {
        let c = &r.census;
        let (strict_ok, strict_short) = verdict_columns(c.strict_v1);
        let (paper_ok, paper_short) = verdict_columns(c.paper_v1);
        sqlx::query(
            "INSERT INTO dataplane.data_coverage_census
               (instrument, venue, timeframe, as_of, period_secs, series_kind,
                first_bar, last_bar, expected_bars, present_bars, missing_bars, missing_bps,
                longest_gap_periods, longest_continuous_bars, longest_continuous_secs,
                derived_knowledge_bars, observed_knowledge_bars,
                strict_v1_satisfied, strict_v1_shortfall_days,
                paper_v1_satisfied, paper_v1_shortfall_days, last_row_age_secs,
                unflagged_sentinel_bars)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21,$22,$23)
             ON CONFLICT (instrument, venue, timeframe, as_of) DO NOTHING",
        )
        .bind(&c.instrument)
        .bind(&c.venue)
        .bind(&c.timeframe)
        .bind(c.as_of)
        .bind(i32::try_from(c.period_secs).unwrap_or(i32::MAX))
        .bind(r.kind.as_str())
        .bind(c.first_bar_ns.map(DateTime::from_timestamp_nanos))
        .bind(c.last_bar_ns.map(DateTime::from_timestamp_nanos))
        .bind(i64::try_from(c.expected_bars).unwrap_or(i64::MAX))
        .bind(i64::try_from(c.present_bars).unwrap_or(i64::MAX))
        .bind(i64::try_from(c.missing_bars).unwrap_or(i64::MAX))
        .bind(i32::try_from(c.missing_bps).unwrap_or(10_000))
        .bind(i64::try_from(c.longest_gap_periods).unwrap_or(i64::MAX))
        .bind(i64::try_from(c.longest_continuous_bars).unwrap_or(i64::MAX))
        .bind(c.longest_continuous_secs)
        .bind(i64::try_from(c.derived_knowledge_bars).unwrap_or(i64::MAX))
        .bind(i64::try_from(c.observed_knowledge_bars).unwrap_or(i64::MAX))
        .bind(strict_ok)
        .bind(strict_short)
        .bind(paper_ok)
        .bind(paper_short)
        .bind(r.last_row_age_secs)
        // Measured by this reading, so never NULL here: NULL is reserved for rows
        // written before migration 0251 added the column (they did not measure it).
        .bind(i64::try_from(c.unflagged_sentinel_bars).unwrap_or(i64::MAX))
        .execute(pg)
        .await?;
        n += 1;
    }
    Ok(n)
}

fn verdict_columns(v: FloorVerdict) -> (bool, Option<i32>) {
    match v {
        FloorVerdict::Satisfied => (true, None),
        FloorVerdict::Short { shortfall_days, .. } => (
            false,
            Some(i32::try_from(shortfall_days).unwrap_or(i32::MAX)),
        ),
        FloorVerdict::NoData => (false, None),
    }
}

/// A compact summary of a census run, for a job result and for the report a human
/// reads. Sorted thinnest-first, because the thinnest series is the one that
/// decides what the platform can evaluate.
#[must_use]
pub fn summarise(records: &[CensusRecord]) -> BTreeMap<String, serde_json::Value> {
    let mut by_depth: Vec<&CensusRecord> = records.iter().collect();
    by_depth.sort_by_key(|r| {
        (
            r.census.longest_continuous_secs,
            r.census.instrument.clone(),
            r.census.venue.clone(),
        )
    });
    let deepest = by_depth.last().map_or(0, |r| r.census.continuous_days());
    let derived: u64 = records
        .iter()
        .map(|r| r.census.derived_knowledge_bars)
        .sum();
    let observed: u64 = records
        .iter()
        .map(|r| r.census.observed_knowledge_bars)
        .sum();
    let mut m = BTreeMap::new();
    m.insert("series".into(), serde_json::json!(records.len()));
    m.insert("deepest_continuous_days".into(), serde_json::json!(deepest));
    m.insert("derived_knowledge_bars".into(), serde_json::json!(derived));
    m.insert(
        "observed_knowledge_bars".into(),
        serde_json::json!(observed),
    );
    m.insert(
        "strict_v1_satisfied_series".into(),
        serde_json::json!(records
            .iter()
            .filter(|r| r.census.strict_v1.is_satisfied())
            .count()),
    );
    m.insert(
        "paper_v1_satisfied_series".into(),
        serde_json::json!(records
            .iter()
            .filter(|r| r.census.paper_v1.is_satisfied())
            .count()),
    );
    m.insert(
        "thinnest".into(),
        serde_json::json!(by_depth
            .iter()
            .take(5)
            .map(|r| serde_json::json!({
                "instrument": r.census.instrument,
                "venue": r.census.venue,
                "timeframe": r.census.timeframe,
                "continuous_days": r.census.continuous_days(),
            }))
            .collect::<Vec<_>>()),
    );
    m
}

fn period_secs_of(timeframe: &str) -> u32 {
    match timeframe {
        "1s" => 1,
        "1m" => 60,
        "5m" => 300,
        "15m" => 900,
        "1h" => 3600,
        "4h" => 14_400,
        "1d" => 86_400,
        other => other.trim_end_matches('s').parse().unwrap_or(60),
    }
}

fn cadence_key(secs: u32) -> String {
    match secs {
        60 => "1m".into(),
        300 => "5m".into(),
        3600 => "1h".into(),
        28_800 => "8h".into(),
        86_400 => "1d".into(),
        other => format!("{other}s"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOUR: i64 = 3600 * 1_000_000_000;

    fn at(n: i64) -> DateTime<Utc> {
        DateTime::from_timestamp_nanos(n)
    }

    #[test]
    fn timeframe_keys_map_to_periods() {
        assert_eq!(period_secs_of("1m"), 60);
        assert_eq!(period_secs_of("1h"), 3600);
        assert_eq!(period_secs_of("42s"), 42);
    }

    #[test]
    fn forward_census_marks_the_series_forward() {
        let obs: Vec<CensusBar> = (0..24).map(|i| CensusBar::observed(i * HOUR)).collect();
        let r = forward_census(
            "funding:PI_XBTUSD",
            "kraken_futures",
            3600,
            &obs,
            at(24 * HOUR),
        )
        .unwrap();
        assert_eq!(r.kind, SeriesKind::Forward);
        assert_eq!(r.kind.as_str(), "forward");
        assert_eq!(r.census.observed_knowledge_bars, 24);
        assert_eq!(r.census.derived_knowledge_bars, 0);
        // The last observation opened at 23h and covers an hour, so at 24h it is
        // exactly fresh.
        assert_eq!(r.last_row_age_secs, Some(0));
    }

    /// Adversarial: a forward-only series may not carry a derived knowledge time.
    /// If one appears, the row is refused rather than written — a backfilled row
    /// masquerading as forward evidence is the exact failure D-P3 exists to stop.
    #[test]
    fn a_derived_stamp_on_a_forward_series_is_refused() {
        let obs = vec![
            CensusBar::observed(0),
            CensusBar::derived(HOUR),
            CensusBar::observed(2 * HOUR),
        ];
        let err = forward_census(
            "funding:PI_XBTUSD",
            "kraken_futures",
            3600,
            &obs,
            at(3 * HOUR),
        )
        .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("derived knowledge_time"), "{msg}");
        assert!(
            msg.contains("D-P3"),
            "the refusal must name the decision it enforces: {msg}"
        );
    }

    #[test]
    fn staleness_grows_with_silence() {
        let obs: Vec<CensusBar> = (0..10).map(|i| CensusBar::observed(i * HOUR)).collect();
        let fresh = forward_census("f", "v", 3600, &obs, at(10 * HOUR)).unwrap();
        let stale = forward_census("f", "v", 3600, &obs, at(100 * HOUR)).unwrap();
        assert_eq!(fresh.last_row_age_secs, Some(0));
        assert_eq!(stale.last_row_age_secs, Some(90 * 3600));
    }

    #[test]
    fn summary_puts_the_thinnest_series_first() {
        let deep: Vec<CensusBar> = (0..24 * 400)
            .map(|i| CensusBar::observed(i * HOUR))
            .collect();
        let thin: Vec<CensusBar> = (0..5).map(|i| CensusBar::observed(i * HOUR)).collect();
        let cal = Calendar::Continuous {
            version: CalendarVersion::crypto_24_7(),
        };
        let records = vec![
            CensusRecord::new(
                census("DEEP", "v", "1h", 3600, &deep, &cal, at(0)),
                SeriesKind::Historical,
            ),
            CensusRecord::new(
                census("THIN", "v", "1h", 3600, &thin, &cal, at(0)),
                SeriesKind::Historical,
            ),
        ];
        let s = summarise(&records);
        let thinnest = s.get("thinnest").unwrap().as_array().unwrap();
        assert_eq!(thinnest[0]["instrument"], "THIN");
        assert_eq!(s.get("series").unwrap(), &serde_json::json!(2));
        assert_eq!(
            s.get("paper_v1_satisfied_series").unwrap(),
            &serde_json::json!(1)
        );
        assert_eq!(
            s.get("strict_v1_satisfied_series").unwrap(),
            &serde_json::json!(0)
        );
    }

    #[test]
    fn verdict_columns_never_state_a_shortfall_for_a_satisfied_floor() {
        assert_eq!(verdict_columns(FloorVerdict::Satisfied), (true, None));
        assert_eq!(verdict_columns(FloorVerdict::NoData), (false, None));
        assert_eq!(
            verdict_columns(FloorVerdict::Short {
                shortfall_days: 12,
                days_at_current_rate: 24
            }),
            (false, Some(12))
        );
    }
}
