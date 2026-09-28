//! Read-time resampling of a stored series to a coarser timeframe (Set P P-4.1;
//! DA-10; DATA-005 §3, §7).
//!
//! Any stored timeframe becomes any coarser one *once*, here, rather than in
//! every caller that wants hourly bars out of a minute table. The reason to
//! centralise it is not tidiness — it is that resampling has three ways of going
//! quietly wrong, and each of them is a decision this module makes on purpose.
//!
//! 1. **A derived series is never cleaner than what it came from.** The grade of
//!    a resampled series is the **worst** grade of its inputs and its declared
//!    lag is the **largest**. That arithmetic has to be that way round or
//!    resampling becomes a laundering step: aggregate a D-graded minute series
//!    into hours and the holes stop being visible in the numbers while the letter
//!    on the series says the data is fine. [`ResampledSeries::grade`] is computed
//!    with [`Grade::min`] and the lag with `max`, and a source whose grade was
//!    never declared is **refused** rather than assumed good
//!    ([`ResampleError::UndeclaredSource`]).
//!
//! 2. **Bar timestamps are the OPEN, and the grids are epoch-anchored.** This is
//!    the canonical rule (DATA-005 §3, `bar::validate`), and resampling is where
//!    an off-by-one-period becomes invisible: a series shifted by one bar still
//!    validates, still has no gaps, and is wrong everywhere. A bucket's open is
//!    the floor of its constituents' opens onto the coarse grid; its close is
//!    that open plus the coarse period. The acceptance test for this module is
//!    that a 1m→1h resample agrees bar-for-bar with the venue's own 1h series.
//!
//! 3. **A partial bucket is not a bar.** The last bucket of a window is usually
//!    incomplete — the hour is still running. Emitting it as an hour bar hands a
//!    study a bar whose high and low are still moving, which is the same defect
//!    as reading a bar before it closes. The default
//!    ([`PartialPolicy::Drop`]) drops a bucket the input does not cover to its
//!    end; [`PartialPolicy::Flag`] keeps it, marked, for a caller that wants to
//!    show a running period. An **interior** short bucket — a venue gap mid-hour
//!    — is always kept and always flagged, because dropping it would turn a
//!    small hole into a large one.
//!
//! What this module deliberately does not do is interpolate. Resampling to a
//! *finer* timeframe is [`ResampleError::Finer`], not a spline: a minute bar
//! invented from an hour bar is fabricated data wearing a real timestamp.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::bar::{CanonicalBar, PRICE_SCALE};
use crate::identity::{InstrumentKey, VenueKey};
use crate::qc::Grade;
use crate::quality::QualityFlags;

// ── what a source declares about itself ──────────────────────────────────────

/// What a contributing source declares, so a derived series can inherit the
/// worst of it.
///
/// Both fields are required. There is no `Default` and no `Option`: a resample
/// over a source whose grade or lag was never resolved is refused, because the
/// alternative is a derived series that reports grade A and a zero lag for
/// inputs nobody graded.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceProvenance {
    pub source_id: i32,
    /// The QC grade of the *input* series from this source.
    pub grade: Grade,
    /// The source's declared vendor lag (`dataplane.source`, ADR-P2-24).
    #[serde(with = "secs")]
    pub declared_lag: Duration,
}

/// What to do with a bucket the input window does not cover to its end.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PartialPolicy {
    /// Drop it. The default, and what makes a resampled series comparable with
    /// the venue's own: a venue does not publish the hour it is still in.
    #[default]
    Drop,
    /// Keep it, with `complete: false` and `LOW_UPDATE_COUNT`, for a caller that
    /// is deliberately showing a running period.
    Flag,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResampleOptions {
    pub partial: PartialPolicy,
}

// ── the output ───────────────────────────────────────────────────────────────

/// One resampled bar. Not a [`CanonicalBar`], on purpose: this row is computed at
/// read time and never stored, so it has no `source_id`, no `ingest_time` and no
/// `revision_seq` to be honest about. Handing back a `CanonicalBar` would invite
/// a caller to insert one.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResampledBar {
    /// The bucket OPEN, on the coarse grid (DATA-005 §3).
    pub event_time: DateTime<Utc>,
    pub period_secs: u32,
    pub open: Decimal,
    pub high: Decimal,
    pub low: Decimal,
    pub close: Decimal,
    pub volume: Decimal,
    /// Volume-weighted from the inputs' own VWAPs. `None` when any constituent
    /// did not report one, or when the bucket traded nothing — a VWAP invented
    /// from a subset of the bucket is not the bucket's VWAP.
    pub vwap: Option<Decimal>,
    /// `None` when any constituent did not report a count. A partial sum would
    /// read as a real count that happens to be low.
    pub trade_count: Option<u64>,
    /// The latest knowledge time of any constituent, and never earlier than the
    /// bucket's own close.
    pub knowledge_time: DateTime<Utc>,
    /// The union of every constituent's flags, plus `LOW_UPDATE_COUNT` when the
    /// bucket holds fewer bars than its grid expects.
    pub quality_flags: QualityFlags,
    /// Input bars that landed in this bucket.
    pub constituents: u32,
    /// Input bars the grid expects in a bucket of this size.
    pub expected_constituents: u32,
    /// Whether the input covered this bucket to its end *and* filled its grid.
    pub complete: bool,
}

impl ResampledBar {
    pub fn close_time(&self) -> DateTime<Utc> {
        self.event_time + chrono::Duration::seconds(i64::from(self.period_secs))
    }
}

/// A resampled series and the provenance it inherited.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResampledSeries {
    pub instrument_id: InstrumentKey,
    pub venue_id: VenueKey,
    pub from_period_secs: u32,
    pub period_secs: u32,
    pub bars: Vec<ResampledBar>,
    /// The **worst** grade of every source that contributed.
    pub grade: Grade,
    /// The **largest** declared lag of every source that contributed.
    #[serde(with = "secs")]
    pub declared_lag: Duration,
    /// Every source that contributed a bar, sorted. A derived series that cannot
    /// name its sources cannot be audited.
    pub source_ids: Vec<i32>,
    /// Buckets dropped because the input did not cover them to their end.
    pub dropped_partial_buckets: u32,
}

impl ResampledSeries {
    /// The figure a dataset carries into `split::EmbargoInputs`.
    pub fn declared_lag_ms(&self) -> u64 {
        u64::try_from(self.declared_lag.as_millis()).unwrap_or(u64::MAX)
    }
}

/// What a derived series inherited from its inputs.
///
/// Shared by [`resample`] and by the information-driven generators
/// (`bars::information_driven`), because "a derived series is never cleaner than
/// its inputs" has to be *one* function. Two copies of this arithmetic is two
/// chances for one of them to be the wrong way round.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InheritedProvenance {
    /// The worst grade of the sources that actually contributed.
    pub grade: Grade,
    /// The largest declared lag of the sources that actually contributed.
    #[serde(with = "secs")]
    pub declared_lag: Duration,
    /// Those sources, sorted.
    pub source_ids: Vec<i32>,
}

/// The shape every derivation in this crate requires of its input series.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InputShape {
    pub instrument_id: InstrumentKey,
    pub venue_id: VenueKey,
    pub period_secs: u32,
    /// The latest bar close in the input: the high-water mark that distinguishes
    /// a bucket the window never reached from one the venue was missing rows in.
    pub covered_to: DateTime<Utc>,
}

/// Check that `inputs` is one instrument, one venue, one period, strictly
/// ordered, and open-aligned.
///
/// # Errors
/// The first violation found. Nothing here is repaired: a derivation over rows
/// it had to guess about produces a series nobody can reproduce.
pub fn validate_input_series(inputs: &[CanonicalBar]) -> Result<InputShape, ResampleError> {
    let first = inputs.first().ok_or(ResampleError::NoInput)?;
    let period_secs = first.period_secs;
    if period_secs == 0 {
        return Err(ResampleError::ZeroPeriod);
    }
    let mut previous: Option<DateTime<Utc>> = None;
    let mut covered_to = first.close_time();
    for b in inputs {
        if b.period_secs != period_secs {
            return Err(ResampleError::MixedPeriods {
                expected: period_secs,
                found: b.period_secs,
            });
        }
        if b.instrument_id != first.instrument_id || b.venue_id != first.venue_id {
            return Err(ResampleError::MixedInstruments);
        }
        if let Some(prev) = previous {
            if b.event_time <= prev {
                return Err(ResampleError::Unordered {
                    previous: prev,
                    found: b.event_time,
                });
            }
        }
        previous = Some(b.event_time);
        if floor_to_grid(b.event_time, period_secs) != Some(b.event_time) {
            return Err(ResampleError::Unaligned {
                event_time: b.event_time,
                period_secs,
            });
        }
        covered_to = covered_to.max(b.close_time());
    }
    Ok(InputShape {
        instrument_id: first.instrument_id,
        venue_id: first.venue_id,
        period_secs,
        covered_to,
    })
}

/// Resolve the worst grade and largest declared lag over the sources that
/// contributed to `inputs`.
///
/// # Errors
/// [`ResampleError::UndeclaredSource`] when a source contributed bars but
/// declared neither. The tempting default — grade A, zero lag — is the most
/// flattering possible answer and would be silent.
pub fn inherit_provenance(
    inputs: &[CanonicalBar],
    provenance: &[SourceProvenance],
) -> Result<InheritedProvenance, ResampleError> {
    let by_source: BTreeMap<i32, SourceProvenance> =
        provenance.iter().map(|p| (p.source_id, *p)).collect();
    let contributing: BTreeSet<i32> = inputs.iter().map(|b| b.source_id).collect();
    if contributing.is_empty() {
        return Err(ResampleError::NoInput);
    }
    let mut grade = Grade::A;
    let mut declared_lag = Duration::ZERO;
    for id in &contributing {
        let p = by_source
            .get(id)
            .ok_or(ResampleError::UndeclaredSource { source_id: *id })?;
        grade = grade.min(p.grade);
        declared_lag = declared_lag.max(p.declared_lag);
    }
    Ok(InheritedProvenance {
        grade,
        declared_lag,
        source_ids: contributing.into_iter().collect(),
    })
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ResampleError {
    #[error("nothing to resample: an empty input series has no grade and no declared lag to inherit, and returning an empty one would present it as clean")]
    NoInput,
    #[error("refusing to resample {from_secs}s bars to {to_secs}s: a finer timeframe can only be interpolated, and interpolated bars are fabricated data wearing a real timestamp")]
    Finer { from_secs: u32, to_secs: u32 },
    #[error("{to_secs}s is not a whole multiple of the stored {from_secs}s period, so its buckets would not align with the stored grid")]
    NotAMultiple { from_secs: u32, to_secs: u32 },
    #[error("period must not be zero")]
    ZeroPeriod,
    #[error("the input mixes bar periods: {expected}s and {found}s")]
    MixedPeriods { expected: u32, found: u32 },
    #[error("the input mixes instruments or venues; resample one series at a time")]
    MixedInstruments,
    #[error("input bar at {found} is out of order or duplicated (previous {previous}); resampling needs a point-in-time-resolved series, one row per bar")]
    Unordered {
        previous: DateTime<Utc>,
        found: DateTime<Utc>,
    },
    #[error("input bar {event_time} is not aligned to a {period_secs}s bar open; bar timestamps are the OPEN (DATA-005 §3)")]
    Unaligned {
        event_time: DateTime<Utc>,
        period_secs: u32,
    },
    #[error("source {source_id} contributed bars but declared no grade or lag; refusing to grade a derived series from an undeclared input rather than assuming it is clean")]
    UndeclaredSource { source_id: i32 },
}

/// Resample `inputs` onto the `to_period_secs` grid.
///
/// `inputs` must be one instrument, one venue, one period, sorted, with one row
/// per bar (point-in-time resolved upstream). Every distinct `source_id` in
/// `inputs` must appear in `provenance`.
///
/// # Errors
/// Any [`ResampleError`]; nothing is aggregated on a partial failure, because a
/// silently shortened derived series is indistinguishable from a real gap.
pub fn resample(
    inputs: &[CanonicalBar],
    to_period_secs: u32,
    provenance: &[SourceProvenance],
    opts: ResampleOptions,
) -> Result<ResampledSeries, ResampleError> {
    if to_period_secs == 0 {
        return Err(ResampleError::ZeroPeriod);
    }
    let shape = validate_input_series(inputs)?;
    let from = shape.period_secs;
    if to_period_secs < from {
        return Err(ResampleError::Finer {
            from_secs: from,
            to_secs: to_period_secs,
        });
    }
    if to_period_secs % from != 0 {
        return Err(ResampleError::NotAMultiple {
            from_secs: from,
            to_secs: to_period_secs,
        });
    }
    let expected = to_period_secs / from;
    let inherited = inherit_provenance(inputs, provenance)?;
    // The high-water mark of the input window, which is what decides whether a
    // bucket is merely short or genuinely still running.
    let input_covered_to = shape.covered_to;

    let mut buckets: BTreeMap<DateTime<Utc>, Vec<&CanonicalBar>> = BTreeMap::new();
    for b in inputs {
        let open = floor_to_grid(b.event_time, to_period_secs).ok_or(ResampleError::Unaligned {
            event_time: b.event_time,
            period_secs: to_period_secs,
        })?;
        buckets.entry(open).or_default().push(b);
    }

    let mut bars = Vec::with_capacity(buckets.len());
    let mut dropped_partial_buckets = 0_u32;
    for (open, members) in buckets {
        let bucket_end = open + chrono::Duration::seconds(i64::from(to_period_secs));
        let covers_end = bucket_end <= input_covered_to;
        let filled = u32::try_from(members.len()).unwrap_or(u32::MAX) >= expected;
        if !covers_end && opts.partial == PartialPolicy::Drop {
            dropped_partial_buckets += 1;
            continue;
        }
        bars.push(aggregate(
            open,
            to_period_secs,
            expected,
            covers_end && filled,
            &members,
        ));
    }

    Ok(ResampledSeries {
        instrument_id: shape.instrument_id,
        venue_id: shape.venue_id,
        from_period_secs: from,
        period_secs: to_period_secs,
        bars,
        grade: inherited.grade,
        declared_lag: inherited.declared_lag,
        source_ids: inherited.source_ids,
        dropped_partial_buckets,
    })
}

/// One bucket. `members` is non-empty and ordered by `event_time`.
fn aggregate(
    open: DateTime<Utc>,
    period_secs: u32,
    expected: u32,
    complete: bool,
    members: &[&CanonicalBar],
) -> ResampledBar {
    let bucket_end = open + chrono::Duration::seconds(i64::from(period_secs));
    let first = members[0];
    let last = members[members.len() - 1];

    let mut high = first.high;
    let mut low = first.low;
    let mut volume = Decimal::ZERO;
    let mut flags = QualityFlags::NONE;
    let mut knowledge_time = first.knowledge_time;
    let mut trade_count: Option<u64> = Some(0);
    // Numerator of the volume-weighted VWAP, kept exact until the single
    // division at the end.
    let mut vwap_numer = Some(Decimal::ZERO);

    for b in members {
        high = high.max(b.high);
        low = low.min(b.low);
        volume += b.volume;
        flags |= b.quality_flags;
        knowledge_time = knowledge_time.max(b.knowledge_time);
        trade_count = match (trade_count, b.trade_count) {
            (Some(acc), Some(n)) => Some(acc.saturating_add(u64::from(n))),
            _ => None,
        };
        vwap_numer = match (vwap_numer, b.vwap) {
            (Some(acc), Some(v)) => acc.checked_add(v * b.volume),
            _ => None,
        };
    }

    let constituents = u32::try_from(members.len()).unwrap_or(u32::MAX);
    if constituents < expected {
        // The closest flag the platform has to "this bar was assembled from
        // fewer updates than its grid expects". Recorded rather than inferred
        // from `constituents`, so a reader filtering on flags sees it too.
        flags |= QualityFlags::LOW_UPDATE_COUNT;
    }

    let vwap = vwap_numer.and_then(|numer| {
        if volume.is_zero() {
            return None;
        }
        // The one division in this module. Rounded to the canonical price scale
        // and normalized at the point of computation, so two buckets holding the
        // same weighted mean stringify identically (SC-1).
        numer
            .checked_div(volume)
            .map(|v| v.round_dp(PRICE_SCALE).normalize())
    });

    ResampledBar {
        event_time: open,
        period_secs,
        open: first.open,
        high,
        low,
        close: last.close,
        volume: volume.normalize(),
        vwap,
        trade_count,
        // A bar labelled as closing at `bucket_end` cannot be knowable before
        // `bucket_end`, whatever its constituents say.
        knowledge_time: knowledge_time.max(bucket_end),
        quality_flags: flags,
        constituents,
        expected_constituents: expected,
        complete,
    }
}

/// Floor an instant onto an epoch-anchored `period_secs` grid.
///
/// Epoch-anchored is the platform's rule (`bar::validate` checks the same
/// `rem_euclid`), which is why a 1m grid and a 1h grid always nest. A
/// week-anchored or session-anchored grid would not, and this function returning
/// `None` for an unrepresentable instant is what keeps that a refusal rather
/// than a silent shift.
fn floor_to_grid(t: DateTime<Utc>, period_secs: u32) -> Option<DateTime<Utc>> {
    let ns = t.timestamp_nanos_opt()?;
    let period_ns = i64::from(period_secs).checked_mul(1_000_000_000)?;
    if period_ns == 0 {
        return None;
    }
    Some(DateTime::from_timestamp_nanos(
        ns - ns.rem_euclid(period_ns),
    ))
}

/// `Duration` as whole seconds, matching `source.rs`'s wire form.
mod secs {
    use serde::{Deserialize, Deserializer, Serializer};
    use std::time::Duration;

    pub fn serialize<S: Serializer>(d: &Duration, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u64(d.as_secs())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Duration, D::Error> {
        Ok(Duration::from_secs(u64::deserialize(d)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use rust_decimal_macros::dec;

    const COINBASE: i32 = 2;
    const ALPACA: i32 = 5;

    fn prov(source_id: i32, grade: Grade, lag_secs: u64) -> SourceProvenance {
        SourceProvenance {
            source_id,
            grade,
            declared_lag: Duration::from_secs(lag_secs),
        }
    }

    fn t(h: u32, m: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 3, 2, h, m, 0).unwrap()
    }

    /// One 1m bar, opening at `open`, with OHLC given as whole-unit decimals.
    fn bar(
        open_time: DateTime<Utc>,
        period_secs: u32,
        ohlc: (Decimal, Decimal, Decimal, Decimal),
        volume: Decimal,
    ) -> CanonicalBar {
        let (open, high, low, close) = ohlc;
        let close_time = open_time + chrono::Duration::seconds(i64::from(period_secs));
        CanonicalBar {
            instrument_id: InstrumentKey(1),
            venue_id: VenueKey(2),
            period_secs,
            event_time: open_time,
            venue_ts: None,
            ingest_time: close_time,
            knowledge_time: close_time + chrono::Duration::seconds(60),
            open,
            high,
            low,
            close,
            volume,
            trade_count: Some(3),
            vwap: None,
            bid_close: None,
            ask_close: None,
            quality_flags: QualityFlags::NONE,
            revision_seq: 0,
            source_id: COINBASE,
        }
    }

    /// A deterministic 1m series over `minutes` minutes from `start`, whose
    /// prices wander enough that a wrong high/low or a one-bar shift shows up.
    fn minute_series(start: DateTime<Utc>, minutes: i64) -> Vec<CanonicalBar> {
        (0..minutes)
            .map(|i| {
                let base = Decimal::from(70_000 + (i * 7) % 113);
                bar(
                    start + chrono::Duration::minutes(i),
                    60,
                    (base, base + dec!(5), base - dec!(3), base + dec!(1)),
                    Decimal::from(i + 1) / dec!(100),
                )
            })
            .collect()
    }

    /// The venue's own 1h bar over the same minutes, assembled the way a venue
    /// does: first open, max high, min low, last close, summed volume.
    struct VenueHour {
        open: Decimal,
        high: Decimal,
        low: Decimal,
        close: Decimal,
        volume: Decimal,
    }

    fn venue_hour(minutes: &[CanonicalBar]) -> VenueHour {
        VenueHour {
            open: minutes[0].open,
            high: minutes.iter().map(|b| b.high).max().unwrap(),
            low: minutes.iter().map(|b| b.low).min().unwrap(),
            close: minutes[minutes.len() - 1].close,
            volume: minutes.iter().map(|b| b.volume).sum::<Decimal>(),
        }
    }

    // ── the acceptance clause: 1m→1h agrees with the venue's own 1h ──────────

    /// P-4.1 clause 1. Three whole hours of minutes, resampled, compared
    /// bar-for-bar against the hour bars a venue would publish for the same
    /// minutes. A one-period shift — the failure this module is most likely to
    /// have — moves every open and close by a bar and is caught here.
    #[test]
    fn a_1m_to_1h_resample_agrees_bar_for_bar_with_the_venues_own_1h_series() {
        let start = t(9, 0);
        let minutes = minute_series(start, 180);
        let out = resample(
            &minutes,
            3_600,
            &[prov(COINBASE, Grade::A, 60)],
            ResampleOptions::default(),
        )
        .unwrap();

        assert_eq!(out.bars.len(), 3);
        for (i, got) in out.bars.iter().enumerate() {
            let slice = &minutes[i * 60..(i + 1) * 60];
            let want = venue_hour(slice);
            assert_eq!(
                got.event_time,
                start + chrono::Duration::hours(i as i64),
                "bucket {i} opens on the hour grid"
            );
            assert_eq!(
                got.close_time(),
                got.event_time + chrono::Duration::hours(1)
            );
            assert_eq!(got.open, want.open, "bucket {i} open");
            assert_eq!(got.high, want.high, "bucket {i} high");
            assert_eq!(got.low, want.low, "bucket {i} low");
            assert_eq!(got.close, want.close, "bucket {i} close");
            assert_eq!(got.volume, want.volume.normalize(), "bucket {i} volume");
            assert_eq!(got.constituents, 60);
            assert_eq!(got.expected_constituents, 60);
            assert!(got.complete);
            assert_eq!(got.trade_count, Some(180));
        }
    }

    /// The same window at two coarsenesses must tell the same story: an hour
    /// resample and a day resample over the same minutes agree on the day's
    /// extremes. This is what would break if a bucket's open were computed from
    /// its own first member's *close* rather than floored onto the grid.
    #[test]
    fn nested_grids_agree_on_the_extremes_they_share() {
        let minutes = minute_series(Utc.with_ymd_and_hms(2026, 3, 2, 0, 0, 0).unwrap(), 1_440);
        let hourly = resample(
            &minutes,
            3_600,
            &[prov(COINBASE, Grade::A, 60)],
            ResampleOptions::default(),
        )
        .unwrap();
        let daily = resample(
            &minutes,
            86_400,
            &[prov(COINBASE, Grade::A, 60)],
            ResampleOptions::default(),
        )
        .unwrap();
        assert_eq!(hourly.bars.len(), 24);
        assert_eq!(daily.bars.len(), 1);
        let day = &daily.bars[0];
        assert_eq!(day.high, hourly.bars.iter().map(|b| b.high).max().unwrap());
        assert_eq!(day.low, hourly.bars.iter().map(|b| b.low).min().unwrap());
        assert_eq!(day.open, hourly.bars[0].open);
        assert_eq!(day.close, hourly.bars[23].close);
        assert_eq!(
            day.volume,
            hourly
                .bars
                .iter()
                .map(|b| b.volume)
                .sum::<Decimal>()
                .normalize()
        );
    }

    // ── the acceptance clause: worst grade, largest lag ──────────────────────

    /// P-4.1 clause 2, and the direction of the arithmetic is the whole point: a
    /// series mixing an A-graded 60 s source with a C-graded 900 s source comes
    /// out **C** and **900 s**. Taking the best grade or the smallest lag would
    /// make resampling a laundering step.
    #[test]
    fn a_derived_series_inherits_the_worst_grade_and_the_largest_lag_of_its_inputs() {
        let mut bars = minute_series(t(9, 0), 120);
        for b in bars.iter_mut().skip(60) {
            b.source_id = ALPACA;
        }
        let out = resample(
            &bars,
            3_600,
            &[prov(COINBASE, Grade::A, 60), prov(ALPACA, Grade::C, 900)],
            ResampleOptions::default(),
        )
        .unwrap();
        assert_eq!(out.grade, Grade::C, "the worst input grade, not the best");
        assert_eq!(out.declared_lag, Duration::from_secs(900));
        assert_eq!(out.declared_lag_ms(), 900_000);
        assert_eq!(out.source_ids, vec![COINBASE, ALPACA]);

        // And the clean single-source case is not dragged down by a declared
        // source that contributed nothing.
        let clean = resample(
            &minute_series(t(9, 0), 60),
            3_600,
            &[prov(COINBASE, Grade::A, 60), prov(ALPACA, Grade::D, 3_600)],
            ResampleOptions::default(),
        )
        .unwrap();
        assert_eq!(clean.grade, Grade::A);
        assert_eq!(clean.declared_lag, Duration::from_secs(60));
        assert_eq!(clean.source_ids, vec![COINBASE]);
    }

    /// A D grade survives into the derived series. Aggregation hides the holes
    /// that produced the D — the hourly series looks continuous — so the letter
    /// is the only thing left carrying the warning.
    #[test]
    fn aggregating_a_d_graded_series_does_not_wash_the_d_out() {
        let out = resample(
            &minute_series(t(9, 0), 60),
            3_600,
            &[prov(COINBASE, Grade::D, 60)],
            ResampleOptions::default(),
        )
        .unwrap();
        assert_eq!(out.grade, Grade::D);
        assert!(!out.grade.admissible_without_waiver());
        assert!(!out.grade.waivable());
    }

    /// Adversarial: a source that contributed bars but declared nothing is
    /// refused. The tempting default — treat it as A with a zero lag — is the
    /// most flattering possible answer and would be silent.
    #[test]
    fn a_source_that_declared_no_grade_is_refused_rather_than_assumed_clean() {
        let bars = minute_series(t(9, 0), 60);
        let err = resample(&bars, 3_600, &[], ResampleOptions::default()).unwrap_err();
        assert_eq!(
            err,
            ResampleError::UndeclaredSource {
                source_id: COINBASE
            }
        );
        assert!(err.to_string().contains("assuming it is clean"));
    }

    // ── the acceptance clause: a finer timeframe is refused ──────────────────

    /// P-4.1 clause 3. Nothing interpolates here.
    #[test]
    fn resampling_to_a_finer_timeframe_is_refused_rather_than_interpolated() {
        let hourly: Vec<CanonicalBar> = minute_series(t(9, 0), 3)
            .into_iter()
            .enumerate()
            .map(|(i, mut b)| {
                b.period_secs = 3_600;
                b.event_time = t(9, 0) + chrono::Duration::hours(i as i64);
                b.knowledge_time = b.close_time() + chrono::Duration::seconds(60);
                b
            })
            .collect();
        let err = resample(
            &hourly,
            60,
            &[prov(COINBASE, Grade::A, 60)],
            ResampleOptions::default(),
        )
        .unwrap_err();
        assert_eq!(
            err,
            ResampleError::Finer {
                from_secs: 3_600,
                to_secs: 60
            }
        );
        assert!(err.to_string().contains("fabricated"));
    }

    /// A target that is coarser but does not nest is refused too: 90 s buckets
    /// over 60 s bars would each straddle a boundary, so every bar would be
    /// assigned to one bucket and silently dropped from another.
    #[test]
    fn a_coarser_period_that_does_not_nest_is_refused() {
        let bars = minute_series(t(9, 0), 10);
        assert_eq!(
            resample(
                &bars,
                90,
                &[prov(COINBASE, Grade::A, 60)],
                ResampleOptions::default()
            )
            .unwrap_err(),
            ResampleError::NotAMultiple {
                from_secs: 60,
                to_secs: 90
            }
        );
    }

    /// Resampling to the same period is the identity, not an error, and it must
    /// not change a single field that a caller would compare against the stored
    /// series.
    #[test]
    fn resampling_to_the_same_period_is_the_identity() {
        let bars = minute_series(t(9, 0), 5);
        let out = resample(
            &bars,
            60,
            &[prov(COINBASE, Grade::A, 60)],
            ResampleOptions::default(),
        )
        .unwrap();
        assert_eq!(out.bars.len(), 5);
        for (got, src) in out.bars.iter().zip(&bars) {
            assert_eq!(got.event_time, src.event_time);
            assert_eq!(got.open, src.open);
            assert_eq!(got.high, src.high);
            assert_eq!(got.low, src.low);
            assert_eq!(got.close, src.close);
            assert_eq!(got.constituents, 1);
            assert!(got.complete);
        }
    }

    // ── partial and short buckets ────────────────────────────────────────────

    /// The default drops the hour that is still running. A venue does not
    /// publish it either, which is exactly why the comparison above works.
    #[test]
    fn the_hour_still_running_is_dropped_by_default_and_flagged_on_request() {
        let bars = minute_series(t(9, 0), 80); // one whole hour plus 20 minutes
        let dropped = resample(
            &bars,
            3_600,
            &[prov(COINBASE, Grade::A, 60)],
            ResampleOptions::default(),
        )
        .unwrap();
        assert_eq!(dropped.bars.len(), 1, "only the hour that finished");
        assert_eq!(dropped.dropped_partial_buckets, 1);

        let flagged = resample(
            &bars,
            3_600,
            &[prov(COINBASE, Grade::A, 60)],
            ResampleOptions {
                partial: PartialPolicy::Flag,
            },
        )
        .unwrap();
        assert_eq!(flagged.bars.len(), 2);
        assert_eq!(flagged.dropped_partial_buckets, 0);
        let running = &flagged.bars[1];
        assert!(!running.complete);
        assert_eq!(running.constituents, 20);
        assert_eq!(running.expected_constituents, 60);
        assert!(running
            .quality_flags
            .contains(QualityFlags::LOW_UPDATE_COUNT));
        // And it does not claim to have been knowable before the hour it names
        // had even ended.
        assert_eq!(running.knowledge_time, running.close_time());
    }

    /// An **interior** gap is kept and flagged. Dropping the hour because the
    /// venue missed four minutes of it would turn a four-minute hole into a
    /// sixty-minute one, which is the opposite of what a coverage census wants.
    #[test]
    fn an_interior_venue_gap_is_flagged_rather_than_dropping_the_bucket() {
        let mut bars = minute_series(t(9, 0), 120);
        bars.drain(20..24); // four minutes the venue did not have
        let out = resample(
            &bars,
            3_600,
            &[prov(COINBASE, Grade::A, 60)],
            ResampleOptions::default(),
        )
        .unwrap();
        assert_eq!(out.bars.len(), 2, "both hours are still reported");
        let short = &out.bars[0];
        assert_eq!(short.constituents, 56);
        assert!(!short.complete, "56 minutes is not an hour of minutes");
        assert!(short.quality_flags.contains(QualityFlags::LOW_UPDATE_COUNT));
        assert_eq!(out.dropped_partial_buckets, 0);
        assert!(out.bars[1].complete);
    }

    /// Flags are unioned, not recomputed: a derived bar carries every warning
    /// its constituents carried. A single suspect minute must not become a clean
    /// hour.
    #[test]
    fn one_flagged_minute_makes_the_hour_carry_that_flag() {
        let mut bars = minute_series(t(9, 0), 60);
        bars[30].quality_flags = QualityFlags::SUSPECT_VOLUME;
        bars[31].quality_flags = QualityFlags::BACKFILLED_KNOWLEDGE_TIME;
        let out = resample(
            &bars,
            3_600,
            &[prov(COINBASE, Grade::A, 60)],
            ResampleOptions::default(),
        )
        .unwrap();
        let hour = &out.bars[0];
        assert!(hour.quality_flags.contains(QualityFlags::SUSPECT_VOLUME));
        assert!(hour
            .quality_flags
            .contains(QualityFlags::BACKFILLED_KNOWLEDGE_TIME));
        assert!(hour.complete, "60 minutes, all present");
    }

    // ── knowledge time ──────────────────────────────────────────────────────

    /// The derived bar is knowable when its *last* constituent was, not its
    /// first. Taking the minimum, or the first bar's stamp, would hand a study
    /// an hour bar forty minutes before its hour ended.
    #[test]
    fn a_resampled_bar_is_knowable_only_when_its_last_constituent_was() {
        let mut bars = minute_series(t(9, 0), 60);
        let late = bars[59].close_time() + chrono::Duration::minutes(15);
        bars[59].knowledge_time = late;
        let out = resample(
            &bars,
            3_600,
            &[prov(COINBASE, Grade::A, 60)],
            ResampleOptions::default(),
        )
        .unwrap();
        assert_eq!(out.bars[0].knowledge_time, late);
        assert!(out.bars[0].knowledge_time >= out.bars[0].close_time());
    }

    // ── VWAP and counts ─────────────────────────────────────────────────────

    /// A VWAP is volume-weighted, and the weighting is exact until one division.
    /// Averaging the inputs' VWAPs unweighted is the common shortcut and it is
    /// wrong whenever volume is uneven — which is always.
    #[test]
    fn a_resampled_vwap_is_volume_weighted_not_averaged() {
        let mut bars = minute_series(t(9, 0), 2);
        bars[0].volume = dec!(1);
        bars[0].vwap = Some(dec!(100));
        bars[1].volume = dec!(3);
        bars[1].vwap = Some(dec!(200));
        let out = resample(
            &bars,
            120,
            &[prov(COINBASE, Grade::A, 60)],
            ResampleOptions::default(),
        )
        .unwrap();
        // (100*1 + 200*3) / 4 = 175, not the unweighted mean of 150.
        assert_eq!(out.bars[0].vwap, Some(dec!(175)));
        assert_eq!(out.bars[0].volume, dec!(4));
    }

    /// Adversarial: a bucket where only some constituents reported a VWAP or a
    /// trade count reports neither, rather than a figure computed over part of
    /// the bucket and presented as the whole.
    #[test]
    fn a_partly_reported_vwap_or_count_becomes_none_rather_than_a_partial_sum() {
        let mut bars = minute_series(t(9, 0), 2);
        bars[0].vwap = Some(dec!(100));
        bars[1].vwap = None;
        bars[1].trade_count = None;
        let out = resample(
            &bars,
            120,
            &[prov(COINBASE, Grade::A, 60)],
            ResampleOptions::default(),
        )
        .unwrap();
        assert_eq!(out.bars[0].vwap, None);
        assert_eq!(out.bars[0].trade_count, None);
    }

    /// Two buckets whose weighted mean is the same value must stringify the same
    /// way, or the same number looks like two numbers downstream.
    #[test]
    fn the_same_weighted_mean_stringifies_the_same_way_however_it_arose() {
        let mk = |v0: Decimal, v1: Decimal| {
            let mut bars = minute_series(t(9, 0), 2);
            bars[0].volume = v0;
            bars[0].vwap = Some(dec!(50));
            bars[1].volume = v1;
            bars[1].vwap = Some(dec!(50));
            resample(
                &bars,
                120,
                &[prov(COINBASE, Grade::A, 60)],
                ResampleOptions::default(),
            )
            .unwrap()
            .bars[0]
                .vwap
                .unwrap()
                .to_string()
        };
        assert_eq!(mk(dec!(1), dec!(3)), mk(dec!(2.50), dec!(7.5000)));
        assert_eq!(mk(dec!(1), dec!(3)), "50");
    }

    /// A bucket that traded nothing has no VWAP. Dividing by zero volume would
    /// either panic or produce a price out of thin air.
    #[test]
    fn a_bucket_that_traded_nothing_has_no_vwap() {
        let mut bars = minute_series(t(9, 0), 2);
        for b in &mut bars {
            b.volume = Decimal::ZERO;
            b.vwap = Some(dec!(100));
        }
        let out = resample(
            &bars,
            120,
            &[prov(COINBASE, Grade::A, 60)],
            ResampleOptions::default(),
        )
        .unwrap();
        assert_eq!(out.bars[0].vwap, None);
        assert_eq!(out.bars[0].volume, Decimal::ZERO);
    }

    // ── input hygiene ───────────────────────────────────────────────────────

    #[test]
    fn an_empty_input_is_refused_rather_than_returning_a_clean_empty_series() {
        assert_eq!(
            resample(&[], 3_600, &[], ResampleOptions::default()).unwrap_err(),
            ResampleError::NoInput
        );
    }

    #[test]
    fn a_duplicated_or_out_of_order_input_bar_is_refused() {
        let mut bars = minute_series(t(9, 0), 3);
        bars.push(bars[1].clone());
        assert!(matches!(
            resample(
                &bars,
                3_600,
                &[prov(COINBASE, Grade::A, 60)],
                ResampleOptions::default()
            ),
            Err(ResampleError::Unordered { .. })
        ));
    }

    #[test]
    fn a_mixed_period_or_mixed_instrument_input_is_refused() {
        let mut mixed = minute_series(t(9, 0), 3);
        mixed[2].period_secs = 300;
        assert_eq!(
            resample(
                &mixed,
                3_600,
                &[prov(COINBASE, Grade::A, 60)],
                ResampleOptions::default()
            )
            .unwrap_err(),
            ResampleError::MixedPeriods {
                expected: 60,
                found: 300
            }
        );

        let mut two = minute_series(t(9, 0), 3);
        two[1].instrument_id = InstrumentKey(99);
        assert_eq!(
            resample(
                &two,
                3_600,
                &[prov(COINBASE, Grade::A, 60)],
                ResampleOptions::default()
            )
            .unwrap_err(),
            ResampleError::MixedInstruments
        );
    }

    /// A bar whose open is not on the stored grid cannot be bucketed without
    /// choosing a side, so it is refused. This is the same `rem_euclid` rule
    /// `bar::validate` enforces at ingest, restated at read time because a
    /// resample can be handed rows from anywhere.
    #[test]
    fn an_input_bar_that_is_not_open_aligned_is_refused() {
        let mut bars = minute_series(t(9, 0), 3);
        bars[1].event_time += chrono::Duration::seconds(30);
        assert!(matches!(
            resample(
                &bars,
                3_600,
                &[prov(COINBASE, Grade::A, 60)],
                ResampleOptions::default()
            ),
            Err(ResampleError::Unaligned { .. })
        ));
    }

    #[test]
    fn a_zero_period_is_refused_on_either_side() {
        let mut bars = minute_series(t(9, 0), 2);
        bars[0].period_secs = 0;
        bars[1].period_secs = 0;
        assert_eq!(
            resample(
                &bars,
                3_600,
                &[prov(COINBASE, Grade::A, 60)],
                ResampleOptions::default()
            )
            .unwrap_err(),
            ResampleError::ZeroPeriod
        );
        assert_eq!(
            resample(
                &minute_series(t(9, 0), 2),
                0,
                &[prov(COINBASE, Grade::A, 60)],
                ResampleOptions::default()
            )
            .unwrap_err(),
            ResampleError::ZeroPeriod
        );
    }

    /// The grids are epoch-anchored, and 1970-01-01 is on every one of them.
    /// Flooring across the epoch must not round towards zero.
    #[test]
    fn flooring_is_towards_the_past_on_both_sides_of_the_epoch() {
        let before = Utc.with_ymd_and_hms(1969, 12, 31, 23, 30, 0).unwrap();
        assert_eq!(
            floor_to_grid(before, 3_600),
            Some(Utc.with_ymd_and_hms(1969, 12, 31, 23, 0, 0).unwrap())
        );
        assert_eq!(
            floor_to_grid(Utc.with_ymd_and_hms(1970, 1, 1, 0, 0, 0).unwrap(), 86_400),
            Some(Utc.with_ymd_and_hms(1970, 1, 1, 0, 0, 0).unwrap())
        );
    }

    /// The series round-trips through JSON, because this is what a read endpoint
    /// serves and a lost `grade` or `declared_lag` in transit is a derived series
    /// that looks clean on arrival.
    #[test]
    fn a_resampled_series_round_trips_through_json_with_its_provenance() {
        let out = resample(
            &minute_series(t(9, 0), 60),
            3_600,
            &[prov(COINBASE, Grade::C, 900)],
            ResampleOptions::default(),
        )
        .unwrap();
        let json = serde_json::to_string(&out).unwrap();
        assert!(json.contains("\"grade\":\"C\""), "{json}");
        assert!(json.contains("\"declared_lag\":900"), "{json}");
        assert_eq!(serde_json::from_str::<ResampledSeries>(&json).unwrap(), out);
    }
}
