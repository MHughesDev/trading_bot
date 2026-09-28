//! Volume, dollar and tick bars (Set P P-4.2; DA-10; 06_DATA §2).
//!
//! Clock bars sample the market on a schedule the market does not keep. A
//! midnight-to-01:00 bar in a dead hour holds the same weight as the minute a
//! Fed print lands, so a clock series oversamples the quiet and undersamples the
//! part that matters — which is why returns over clock bars are so far from
//! i.i.d. that most of the statistics applied to them do not hold. Sampling on
//! *information* — a fixed quantity of volume, of notional, or of trades per bar
//! — is the standard repair.
//!
//! Three things here are decisions rather than details:
//!
//! 1. **The threshold is recorded with the series, and it is in the hash.** A
//!    volume bar set means nothing without the quantity that closed each bar, so
//!    [`InformationSeries::series_hash`] covers the spec as well as the bars and
//!    [`InformationSeries::spec`] travels with them. Regenerating with the same
//!    threshold over the same inputs is byte-identical; regenerating with a
//!    different one is a different series with a different hash, not an update of
//!    this one.
//!
//! 2. **Buckets close on a bar boundary, and the overshoot is recorded, not
//!    hidden.** These generators read *stored bars*, which is the only input the
//!    platform has for most of its history. A bar cannot be split, so a bucket
//!    closes on the first input bar that takes it to or past its threshold and
//!    carries [`InformationBar::overshoot`] saying by how much. The alternative —
//!    apportioning part of a bar into each bucket — would fabricate intra-bar
//!    prices for the split, which is exactly the thing this crate refuses to do
//!    anywhere else. [`InformationSeries::sampled_from_period_secs`] states the
//!    granularity limit so a reader knows the boundaries are as precise as the
//!    input and no more.
//!
//! 3. **A bucket that never reached its threshold is not a bar.** The tail of any
//!    window is a partial accumulation. Emitting it would present an unfinished
//!    bucket as a closed one, so it is reported as [`ResidualTail`] instead of
//!    appended. The accumulator resets to zero on emission rather than carrying
//!    the overshoot forward: a carried remainder makes every later boundary
//!    depend on a quantity none of its own constituents contained, which is
//!    reproducible but not explicable.
//!
//! **Absent inputs are a stated reason, never an empty series.** Tick bars over a
//! source that does not report trade counts would silently accumulate nothing for
//! ever and return zero bars, which reads identically to "this instrument did not
//! trade". Coinbase's candle endpoint is exactly such a source — its adapter
//! reports `trade_count: 0` because zero is the honest value for "trades we were
//! told about" — so [`InformationBarError::InputsCarryNoInformation`] names the
//! metric, the source and what would supply it.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use chrono::{DateTime, Utc};

use crate::bar::CanonicalBar;
use crate::hash::content_hash;
use crate::identity::{InstrumentKey, VenueKey};
use crate::qc::Grade;
use crate::quality::QualityFlags;
use crate::resample::{inherit_provenance, validate_input_series, ResampleError, SourceProvenance};

// ── the spec ─────────────────────────────────────────────────────────────────

/// What quantity closes a bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InformationBarKind {
    /// A fixed quantity of base-asset volume.
    Volume,
    /// A fixed quantity of notional (`close × volume`). The one to prefer over a
    /// long window: a fixed *volume* threshold samples a market whose price rose
    /// tenfold far less often at the end than at the start, so the bar count per
    /// unit of economic activity drifts.
    Dollar,
    /// A fixed number of trades.
    Tick,
}

impl InformationBarKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Volume => "volume",
            Self::Dollar => "dollar",
            Self::Tick => "tick",
        }
    }

    /// What supplies this metric, named in the error when it is absent.
    const fn input_field(self) -> &'static str {
        match self {
            Self::Volume => "bar volume",
            Self::Dollar => "bar close × volume",
            Self::Tick => "bar trade_count",
        }
    }
}

/// A generator, fully specified. Two of these that compare equal produce
/// byte-identical series over the same inputs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InformationBarSpec {
    pub kind: InformationBarKind,
    /// The quantity that closes a bar. For [`InformationBarKind::Tick`] this is a
    /// whole number of trades.
    pub threshold: Decimal,
}

impl InformationBarSpec {
    pub const fn new(kind: InformationBarKind, threshold: Decimal) -> Self {
        Self { kind, threshold }
    }

    /// # Errors
    /// A threshold that is not strictly positive, or a fractional tick count.
    pub fn validate(&self) -> Result<(), InformationBarError> {
        if self.threshold <= Decimal::ZERO {
            return Err(InformationBarError::ThresholdNotPositive {
                kind: self.kind,
                threshold: self.threshold.to_string(),
            });
        }
        if self.kind == InformationBarKind::Tick && self.threshold.fract() != Decimal::ZERO {
            return Err(InformationBarError::FractionalTickThreshold {
                threshold: self.threshold.to_string(),
            });
        }
        Ok(())
    }
}

// ── the output ───────────────────────────────────────────────────────────────

/// One information-driven bar.
///
/// Not a [`CanonicalBar`]: these are never stored, and their boundaries are not
/// on any clock grid, so they have no `event_time` in the canonical sense. The
/// span is stated as both ends explicitly.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InformationBar {
    /// The open of the first constituent bar.
    pub open_time: DateTime<Utc>,
    /// The close of the constituent that crossed the threshold.
    pub close_time: DateTime<Utc>,
    pub open: Decimal,
    pub high: Decimal,
    pub low: Decimal,
    pub close: Decimal,
    pub volume: Decimal,
    /// `Σ close × volume` over the constituents.
    pub dollar_volume: Decimal,
    /// `Σ trade_count`. `None` when any constituent did not report one.
    pub ticks: Option<u64>,
    /// The metric this bar accumulated, which is at or above the threshold.
    pub accumulated: Decimal,
    /// `accumulated − threshold`: how far past its threshold the crossing bar
    /// carried this bucket. Zero when the boundary landed exactly.
    pub overshoot: Decimal,
    /// Input bars in this bucket.
    pub constituents: u32,
    /// The latest knowledge time of any constituent.
    pub knowledge_time: DateTime<Utc>,
    /// The union of every constituent's flags.
    pub quality_flags: QualityFlags,
}

/// Accumulation left over at the end of the window: real activity that did not
/// amount to a bar.
///
/// Reported rather than emitted. A reader that wants to know whether the series
/// ends mid-bucket asks this; a reader that wants closed bars ignores it. What it
/// must never be is an extra bar.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResidualTail {
    pub bars: u32,
    pub accumulated: Decimal,
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
}

/// A generated series, its spec, and the provenance it inherited.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InformationSeries {
    pub spec: InformationBarSpec,
    pub instrument_id: InstrumentKey,
    pub venue_id: VenueKey,
    /// The clock period of the input bars: the precision limit on every boundary
    /// in this series.
    pub sampled_from_period_secs: u32,
    pub bars: Vec<InformationBar>,
    /// `Some` when the window ended mid-bucket.
    pub residual: Option<ResidualTail>,
    /// The **worst** grade of the sources that contributed.
    pub grade: Grade,
    /// The **largest** declared lag of the sources that contributed, in
    /// milliseconds — the unit `split::EmbargoInputs` wants.
    pub declared_lag_ms: u64,
    pub source_ids: Vec<i32>,
}

impl InformationSeries {
    /// The content hash over the spec and every bar.
    ///
    /// This is what "reproducible" means for an information-driven series: the
    /// threshold is inside the hash, so a series cannot be quoted without the
    /// quantity that produced it.
    ///
    /// # Panics
    /// Never in practice: every field is JSON-representable.
    pub fn series_hash(&self) -> String {
        content_hash(self).expect("information series serializes")
    }
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum InformationBarError {
    #[error("{kind} bars need a threshold above zero; {threshold} would close a bar on every input row or on none")]
    ThresholdNotPositive {
        kind: InformationBarKind,
        threshold: String,
    },
    #[error("tick bars count trades, so the threshold must be a whole number; {threshold} is not")]
    FractionalTickThreshold { threshold: String },
    #[error("no stored bars were supplied for {kind} bars over the requested window, so there is nothing to sample; this is a missing input, not an instrument that did not trade")]
    NoInput { kind: InformationBarKind },
    #[error("{kind} bars cannot be generated here: {input_field} is absent or zero across all {bars} input bars, so no bucket would ever reach its threshold and the generator would return an empty series that reads as 'this instrument did not trade'. {remedy}")]
    InputsCarryNoInformation {
        kind: InformationBarKind,
        input_field: &'static str,
        bars: u32,
        remedy: String,
    },
    #[error("input bar at {at} reports no trade_count, so tick bars over this window would silently undercount; a source that does not report trade counts cannot produce tick bars")]
    TickCountAbsent { at: DateTime<Utc> },
    #[error(transparent)]
    Input(#[from] ResampleError),
}

/// Generate an information-driven series from stored clock bars.
///
/// `inputs` must be one instrument, one venue, one period, sorted, one row per
/// bar — the same precondition [`crate::resample::resample`] has, checked by the
/// same function. Every `source_id` present must appear in `provenance`.
///
/// # Errors
/// Any [`InformationBarError`]. Absent inputs are an error with a stated reason
/// and never an empty series.
pub fn generate(
    spec: InformationBarSpec,
    inputs: &[CanonicalBar],
    provenance: &[SourceProvenance],
) -> Result<InformationSeries, InformationBarError> {
    spec.validate()?;
    if inputs.is_empty() {
        return Err(InformationBarError::NoInput { kind: spec.kind });
    }
    let shape = validate_input_series(inputs)?;
    let inherited = inherit_provenance(inputs, provenance)?;

    // The metric, per input bar, resolved up front so an absent one is an error
    // about the input rather than a bucket that never closes.
    let mut metrics = Vec::with_capacity(inputs.len());
    let mut total = Decimal::ZERO;
    for b in inputs {
        let m = match spec.kind {
            InformationBarKind::Volume => b.volume,
            InformationBarKind::Dollar => b.close * b.volume,
            InformationBarKind::Tick => match b.trade_count {
                Some(n) => Decimal::from(n),
                None => {
                    return Err(InformationBarError::TickCountAbsent { at: b.event_time });
                }
            },
        };
        total += m;
        metrics.push(m);
    }
    if total <= Decimal::ZERO {
        let bars = u32::try_from(inputs.len()).unwrap_or(u32::MAX);
        return Err(InformationBarError::InputsCarryNoInformation {
            kind: spec.kind,
            input_field: spec.kind.input_field(),
            bars,
            remedy: remedy_for(spec.kind),
        });
    }

    let mut bars: Vec<InformationBar> = Vec::new();
    let mut bucket: Vec<usize> = Vec::new();
    let mut accumulated = Decimal::ZERO;
    for (i, b) in inputs.iter().enumerate() {
        bucket.push(i);
        accumulated += metrics[i];
        if accumulated >= spec.threshold {
            bars.push(close_bucket(inputs, &bucket, accumulated, spec.threshold));
            bucket.clear();
            // Reset rather than carry: see the module docs.
            accumulated = Decimal::ZERO;
        }
        debug_assert!(b.period_secs == shape.period_secs);
    }

    let residual = (!bucket.is_empty()).then(|| ResidualTail {
        bars: u32::try_from(bucket.len()).unwrap_or(u32::MAX),
        accumulated: accumulated.normalize(),
        from: inputs[bucket[0]].event_time,
        to: inputs[bucket[bucket.len() - 1]].close_time(),
    });

    Ok(InformationSeries {
        spec,
        instrument_id: shape.instrument_id,
        venue_id: shape.venue_id,
        sampled_from_period_secs: shape.period_secs,
        bars,
        residual,
        grade: inherited.grade,
        declared_lag_ms: u64::try_from(inherited.declared_lag.as_millis()).unwrap_or(u64::MAX),
        source_ids: inherited.source_ids,
    })
}

/// What would supply a metric this input cannot.
fn remedy_for(kind: InformationBarKind) -> String {
    match kind {
        InformationBarKind::Tick => "Trade counts come from a trade-level feed or from a bar source that reports them (Kraken OHLC and Alpaca bars do; the Coinbase Exchange candle endpoint does not, and its adapter records 0 rather than inventing a figure).".to_string(),
        InformationBarKind::Volume | InformationBarKind::Dollar => "Every bar in the window reports zero volume, which means either the instrument genuinely did not trade or the source does not report volume; the coverage census (P-0.1) distinguishes the two.".to_string(),
    }
}

fn close_bucket(
    inputs: &[CanonicalBar],
    bucket: &[usize],
    accumulated: Decimal,
    threshold: Decimal,
) -> InformationBar {
    let first = &inputs[bucket[0]];
    let last = &inputs[bucket[bucket.len() - 1]];
    let mut high = first.high;
    let mut low = first.low;
    let mut volume = Decimal::ZERO;
    let mut dollar_volume = Decimal::ZERO;
    let mut ticks: Option<u64> = Some(0);
    let mut flags = QualityFlags::NONE;
    let mut knowledge_time = first.knowledge_time;
    for i in bucket {
        let b = &inputs[*i];
        high = high.max(b.high);
        low = low.min(b.low);
        volume += b.volume;
        dollar_volume += b.close * b.volume;
        ticks = match (ticks, b.trade_count) {
            (Some(acc), Some(n)) => Some(acc.saturating_add(u64::from(n))),
            _ => None,
        };
        flags |= b.quality_flags;
        knowledge_time = knowledge_time.max(b.knowledge_time);
    }
    InformationBar {
        open_time: first.event_time,
        close_time: last.close_time(),
        open: first.open,
        high,
        low,
        close: last.close,
        volume: volume.normalize(),
        dollar_volume: dollar_volume.normalize(),
        ticks,
        accumulated: accumulated.normalize(),
        overshoot: (accumulated - threshold).normalize(),
        constituents: u32::try_from(bucket.len()).unwrap_or(u32::MAX),
        knowledge_time,
        quality_flags: flags,
    }
}

impl std::fmt::Display for InformationBarKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
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
            declared_lag: std::time::Duration::from_secs(lag_secs),
        }
    }

    fn start() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 3, 2, 9, 0, 0).unwrap()
    }

    /// `minutes` 1m bars whose volume is `vols[i]` and whose close is
    /// `100 + i`, so a dollar threshold and a volume threshold do not coincide.
    fn series(vols: &[Decimal]) -> Vec<CanonicalBar> {
        vols.iter()
            .enumerate()
            .map(|(i, v)| {
                let open = start() + chrono::Duration::minutes(i as i64);
                let close = open + chrono::Duration::minutes(1);
                let px = Decimal::from(100 + i as i64);
                CanonicalBar {
                    instrument_id: InstrumentKey(1),
                    venue_id: VenueKey(2),
                    period_secs: 60,
                    event_time: open,
                    venue_ts: None,
                    ingest_time: close,
                    knowledge_time: close + chrono::Duration::seconds(60),
                    open: px,
                    high: px + dec!(2),
                    low: px - dec!(1),
                    close: px,
                    volume: *v,
                    trade_count: Some(2),
                    vwap: None,
                    bid_close: None,
                    ask_close: None,
                    quality_flags: QualityFlags::NONE,
                    revision_seq: 0,
                    source_id: COINBASE,
                }
            })
            .collect()
    }

    fn only_coinbase() -> Vec<SourceProvenance> {
        vec![prov(COINBASE, Grade::A, 60)]
    }

    // ── the acceptance clause: all three kinds, threshold recorded ───────────

    /// P-4.2 clause 1, volume. Four minutes of 3 units each with a threshold of
    /// 6 gives two bars of two minutes, and each bar carries the threshold that
    /// closed it.
    #[test]
    fn volume_bars_close_on_their_threshold_and_record_it() {
        let bars = series(&[dec!(3), dec!(3), dec!(3), dec!(3)]);
        let out = generate(
            InformationBarSpec::new(InformationBarKind::Volume, dec!(6)),
            &bars,
            &only_coinbase(),
        )
        .unwrap();
        assert_eq!(out.bars.len(), 2);
        assert_eq!(out.spec.threshold, dec!(6));
        assert_eq!(out.spec.kind, InformationBarKind::Volume);
        assert_eq!(out.sampled_from_period_secs, 60);
        assert!(out.residual.is_none());
        for (i, b) in out.bars.iter().enumerate() {
            assert_eq!(b.volume, dec!(6), "bar {i}");
            assert_eq!(b.accumulated, dec!(6));
            assert_eq!(b.overshoot, Decimal::ZERO);
            assert_eq!(b.constituents, 2);
            assert_eq!(b.ticks, Some(4));
        }
        assert_eq!(out.bars[0].open_time, start());
        assert_eq!(
            out.bars[0].close_time,
            start() + chrono::Duration::minutes(2)
        );
        assert_eq!(out.bars[0].open, dec!(100));
        assert_eq!(out.bars[0].close, dec!(101));
        assert_eq!(out.bars[1].open, dec!(102));
    }

    /// P-4.2 clause 1, dollar. The same volumes at a rising price close dollar
    /// bars at different places than volume bars — which is the whole reason to
    /// prefer them over a long window.
    #[test]
    fn dollar_bars_sample_notional_not_quantity() {
        let bars = series(&[dec!(1), dec!(1), dec!(1), dec!(1)]);
        let vol = generate(
            InformationBarSpec::new(InformationBarKind::Volume, dec!(2)),
            &bars,
            &only_coinbase(),
        )
        .unwrap();
        let dollar = generate(
            // Closes 100+101 = 201 at the second minute, 102+103 = 205 at the
            // fourth: the same boundaries here…
            InformationBarSpec::new(InformationBarKind::Dollar, dec!(201)),
            &bars,
            &only_coinbase(),
        )
        .unwrap();
        assert_eq!(vol.bars.len(), 2);
        assert_eq!(dollar.bars.len(), 2);
        assert_eq!(dollar.bars[0].dollar_volume, dec!(201));
        // …and different ones at a threshold quantity cannot express.
        let uneven = generate(
            InformationBarSpec::new(InformationBarKind::Dollar, dec!(100)),
            &bars,
            &only_coinbase(),
        )
        .unwrap();
        assert_eq!(uneven.bars.len(), 4, "every minute clears 100 of notional");
        assert_eq!(uneven.bars[3].overshoot, dec!(3), "103 − 100");
    }

    /// P-4.2 clause 1, tick.
    #[test]
    fn tick_bars_count_trades() {
        let bars = series(&[dec!(1); 6]);
        let out = generate(
            InformationBarSpec::new(InformationBarKind::Tick, dec!(4)),
            &bars,
            &only_coinbase(),
        )
        .unwrap();
        // Two trades per minute, four per bar: three bars.
        assert_eq!(out.bars.len(), 3);
        assert!(out.bars.iter().all(|b| b.ticks == Some(4)));
        assert!(out.bars.iter().all(|b| b.constituents == 2));
    }

    // ── the acceptance clause: regeneration is byte-identical ────────────────

    /// P-4.2 clause 2. The same threshold over the same inputs produces the same
    /// bytes, and a different threshold produces a different series rather than a
    /// revision of this one.
    #[test]
    fn regenerating_with_the_same_threshold_is_byte_identical() {
        let bars = series(&[dec!(3), dec!(5), dec!(2), dec!(9), dec!(1)]);
        let spec = InformationBarSpec::new(InformationBarKind::Volume, dec!(7));
        let a = generate(spec, &bars, &only_coinbase()).unwrap();
        let b = generate(spec, &bars, &only_coinbase()).unwrap();
        assert_eq!(
            crate::hash::canonical_json(&a).unwrap(),
            crate::hash::canonical_json(&b).unwrap(),
            "byte-identical, not merely equal"
        );
        assert_eq!(a.series_hash(), b.series_hash());

        let other = generate(
            InformationBarSpec::new(InformationBarKind::Volume, dec!(8)),
            &bars,
            &only_coinbase(),
        )
        .unwrap();
        assert_ne!(
            a.series_hash(),
            other.series_hash(),
            "the threshold is inside the hash, so a different one is a different series"
        );
    }

    /// The hash covers the *kind* too. A volume series and a tick series that
    /// happen to produce identical bars must still be distinguishable, or one
    /// could be quoted as the other.
    #[test]
    fn the_hash_distinguishes_two_kinds_that_produced_the_same_bars() {
        let bars = series(&[dec!(2), dec!(2), dec!(2), dec!(2)]);
        let vol = generate(
            InformationBarSpec::new(InformationBarKind::Volume, dec!(4)),
            &bars,
            &only_coinbase(),
        )
        .unwrap();
        let tick = generate(
            InformationBarSpec::new(InformationBarKind::Tick, dec!(4)),
            &bars,
            &only_coinbase(),
        )
        .unwrap();
        assert_eq!(vol.bars.len(), tick.bars.len());
        assert_eq!(vol.bars[0].volume, tick.bars[0].volume);
        assert_ne!(vol.series_hash(), tick.series_hash());
    }

    // ── the acceptance clause: absent inputs state a reason ──────────────────

    /// P-4.2 clause 3, and the case that actually happens: Coinbase's candle
    /// endpoint reports no trade count, so its adapter writes 0. Tick bars over
    /// that window would accumulate nothing for ever and return zero bars, which
    /// reads exactly like an instrument that did not trade.
    #[test]
    fn tick_bars_over_a_source_with_no_trade_counts_state_the_reason() {
        let mut bars = series(&[dec!(1); 10]);
        for b in &mut bars {
            b.trade_count = Some(0);
        }
        let err = generate(
            InformationBarSpec::new(InformationBarKind::Tick, dec!(5)),
            &bars,
            &only_coinbase(),
        )
        .unwrap_err();
        match &err {
            InformationBarError::InputsCarryNoInformation {
                kind,
                input_field,
                bars: n,
                ..
            } => {
                assert_eq!(*kind, InformationBarKind::Tick);
                assert_eq!(*input_field, "bar trade_count");
                assert_eq!(*n, 10);
            }
            other => panic!("{other:?}"),
        }
        let text = err.to_string();
        assert!(text.contains("did not trade"), "{text}");
        assert!(
            text.contains("Coinbase"),
            "the reason should name the source that cannot supply it: {text}"
        );
    }

    /// A window with no volume at all is a stated reason, not an empty series —
    /// and the reason says which of the two possible causes the census settles.
    #[test]
    fn a_window_with_no_volume_is_a_stated_reason_not_an_empty_series() {
        let bars = series(&[Decimal::ZERO; 4]);
        let err = generate(
            InformationBarSpec::new(InformationBarKind::Volume, dec!(1)),
            &bars,
            &only_coinbase(),
        )
        .unwrap_err();
        assert!(matches!(
            err,
            InformationBarError::InputsCarryNoInformation { .. }
        ));
        assert!(err.to_string().contains("coverage census"));
    }

    /// No rows at all names the generator that was asked for, so the caller can
    /// tell "I have no data for this window" from "this window had no activity".
    #[test]
    fn an_absent_input_window_is_refused_by_kind() {
        let err = generate(
            InformationBarSpec::new(InformationBarKind::Dollar, dec!(1)),
            &[],
            &only_coinbase(),
        )
        .unwrap_err();
        assert_eq!(
            err,
            InformationBarError::NoInput {
                kind: InformationBarKind::Dollar
            }
        );
        assert!(err
            .to_string()
            .contains("not an instrument that did not trade"));
    }

    /// A single bar missing its count is not the same as a source that never
    /// reports one, and it is not silently treated as zero either.
    #[test]
    fn one_input_bar_with_no_trade_count_is_named_rather_than_counted_as_zero() {
        let mut bars = series(&[dec!(1); 4]);
        bars[2].trade_count = None;
        let err = generate(
            InformationBarSpec::new(InformationBarKind::Tick, dec!(4)),
            &bars,
            &only_coinbase(),
        )
        .unwrap_err();
        assert_eq!(
            err,
            InformationBarError::TickCountAbsent {
                at: bars[2].event_time
            }
        );
    }

    // ── boundaries, overshoot and the tail ──────────────────────────────────

    /// A bar cannot be split, so a bucket closes past its threshold and says by
    /// how much. Silently apportioning part of the crossing bar into the next
    /// bucket would require inventing prices for the split.
    #[test]
    fn a_bucket_closes_past_its_threshold_and_records_the_overshoot() {
        let bars = series(&[dec!(1), dec!(50)]);
        let out = generate(
            InformationBarSpec::new(InformationBarKind::Volume, dec!(10)),
            &bars,
            &only_coinbase(),
        )
        .unwrap();
        assert_eq!(out.bars.len(), 1);
        assert_eq!(out.bars[0].accumulated, dec!(51));
        assert_eq!(out.bars[0].overshoot, dec!(41));
        assert_eq!(out.bars[0].constituents, 2);
        assert!(out.residual.is_none());
    }

    /// The accumulator resets rather than carrying, so a huge bar does not
    /// pre-close the next several buckets from volume they never contained.
    #[test]
    fn the_overshoot_is_not_carried_into_the_next_bucket() {
        let bars = series(&[dec!(25), dec!(5), dec!(4)]);
        let out = generate(
            InformationBarSpec::new(InformationBarKind::Volume, dec!(10)),
            &bars,
            &only_coinbase(),
        )
        .unwrap();
        assert_eq!(
            out.bars.len(),
            1,
            "the 25 closed one bar; had its 15 of overshoot been carried, the next 5 would have \
             closed a second bar out of volume it did not contain"
        );
        assert_eq!(out.bars[0].overshoot, dec!(15));
        let residual = out.residual.unwrap();
        assert_eq!(residual.bars, 2);
        assert_eq!(
            residual.accumulated,
            dec!(9),
            "the 5 and the 4 accumulate from zero, not from the carried 15"
        );
    }

    /// The tail is reported, never emitted. A bucket that has not reached its
    /// threshold is not a bar, and appending it would present an unfinished
    /// accumulation as a closed one.
    #[test]
    fn an_unfinished_tail_is_reported_rather_than_emitted_as_a_bar() {
        let bars = series(&[dec!(4), dec!(4), dec!(4)]);
        let out = generate(
            InformationBarSpec::new(InformationBarKind::Volume, dec!(8)),
            &bars,
            &only_coinbase(),
        )
        .unwrap();
        assert_eq!(out.bars.len(), 1);
        let residual = out.residual.unwrap();
        assert_eq!(residual.bars, 1);
        assert_eq!(residual.accumulated, dec!(4));
        assert_eq!(residual.from, start() + chrono::Duration::minutes(2));
        assert_eq!(residual.to, start() + chrono::Duration::minutes(3));
    }

    /// Prices in, prices out: the OHLC of a bucket is the venue's own extremes
    /// over its constituents, and a bar assembled with `open` taken from the last
    /// constituent still validates while being wrong everywhere.
    #[test]
    fn a_bucket_takes_the_first_open_and_the_last_close() {
        let mut bars = series(&[dec!(5), dec!(5)]);
        bars[0].high = dec!(999);
        bars[1].low = dec!(1);
        let out = generate(
            InformationBarSpec::new(InformationBarKind::Volume, dec!(10)),
            &bars,
            &only_coinbase(),
        )
        .unwrap();
        let b = &out.bars[0];
        assert_eq!(b.open, bars[0].open);
        assert_eq!(b.close, bars[1].close);
        assert_eq!(b.high, dec!(999));
        assert_eq!(b.low, dec!(1));
    }

    // ── provenance and hygiene ──────────────────────────────────────────────

    /// The same rule as resampling, through the same function: a derived series
    /// inherits the worst grade and the largest lag.
    #[test]
    fn an_information_series_inherits_the_worst_grade_and_largest_lag() {
        let mut bars = series(&[dec!(5); 4]);
        bars[3].source_id = ALPACA;
        let out = generate(
            InformationBarSpec::new(InformationBarKind::Volume, dec!(10)),
            &bars,
            &[prov(COINBASE, Grade::A, 60), prov(ALPACA, Grade::C, 900)],
        )
        .unwrap();
        assert_eq!(out.grade, Grade::C);
        assert_eq!(out.declared_lag_ms, 900_000);
        assert_eq!(out.source_ids, vec![COINBASE, ALPACA]);
    }

    #[test]
    fn a_source_that_declared_nothing_is_refused_here_too() {
        let bars = series(&[dec!(5); 2]);
        let err = generate(
            InformationBarSpec::new(InformationBarKind::Volume, dec!(10)),
            &bars,
            &[],
        )
        .unwrap_err();
        assert!(matches!(
            err,
            InformationBarError::Input(ResampleError::UndeclaredSource { .. })
        ));
    }

    /// A flagged minute makes every bucket it lands in carry that flag: an
    /// information bar is no cleaner than the clock bars behind it.
    #[test]
    fn a_flagged_constituent_flags_the_information_bar() {
        let mut bars = series(&[dec!(5); 4]);
        bars[1].quality_flags = QualityFlags::BACKFILLED_KNOWLEDGE_TIME;
        let out = generate(
            InformationBarSpec::new(InformationBarKind::Volume, dec!(10)),
            &bars,
            &only_coinbase(),
        )
        .unwrap();
        assert!(out.bars[0]
            .quality_flags
            .contains(QualityFlags::BACKFILLED_KNOWLEDGE_TIME));
        assert_eq!(out.bars[1].quality_flags, QualityFlags::NONE);
    }

    /// A bucket is knowable when its last constituent was, and never before its
    /// own close.
    #[test]
    fn a_bucket_is_knowable_only_when_its_last_constituent_was() {
        let bars = series(&[dec!(5); 2]);
        let out = generate(
            InformationBarSpec::new(InformationBarKind::Volume, dec!(10)),
            &bars,
            &only_coinbase(),
        )
        .unwrap();
        assert_eq!(out.bars[0].knowledge_time, bars[1].knowledge_time);
        assert!(out.bars[0].knowledge_time >= out.bars[0].close_time);
    }

    #[test]
    fn a_threshold_of_zero_or_less_is_refused() {
        let bars = series(&[dec!(5); 2]);
        for t in [Decimal::ZERO, dec!(-1)] {
            let err = generate(
                InformationBarSpec::new(InformationBarKind::Volume, t),
                &bars,
                &only_coinbase(),
            )
            .unwrap_err();
            assert!(matches!(
                err,
                InformationBarError::ThresholdNotPositive { .. }
            ));
        }
    }

    #[test]
    fn a_fractional_tick_threshold_is_refused() {
        let bars = series(&[dec!(5); 2]);
        let err = generate(
            InformationBarSpec::new(InformationBarKind::Tick, dec!(2.5)),
            &bars,
            &only_coinbase(),
        )
        .unwrap_err();
        assert!(matches!(
            err,
            InformationBarError::FractionalTickThreshold { .. }
        ));
        // …but the same fractional threshold is legitimate for volume.
        assert!(generate(
            InformationBarSpec::new(InformationBarKind::Volume, dec!(2.5)),
            &bars,
            &only_coinbase()
        )
        .is_ok());
    }

    /// The input preconditions are the resampler's, checked by the resampler's
    /// function, so the two derivations cannot disagree about what a valid input
    /// series is.
    #[test]
    fn an_out_of_order_input_is_refused_by_the_shared_validator() {
        let mut bars = series(&[dec!(5); 3]);
        bars.swap(0, 2);
        let err = generate(
            InformationBarSpec::new(InformationBarKind::Volume, dec!(5)),
            &bars,
            &only_coinbase(),
        )
        .unwrap_err();
        assert!(matches!(
            err,
            InformationBarError::Input(ResampleError::Unordered { .. })
        ));
    }

    #[test]
    fn a_series_round_trips_through_json_with_its_spec() {
        let bars = series(&[dec!(5); 4]);
        let out = generate(
            InformationBarSpec::new(InformationBarKind::Dollar, dec!(500)),
            &bars,
            &only_coinbase(),
        )
        .unwrap();
        let json = serde_json::to_string(&out).unwrap();
        assert!(json.contains("\"kind\":\"dollar\""), "{json}");
        let back: InformationSeries = serde_json::from_str(&json).unwrap();
        assert_eq!(back, out);
        assert_eq!(back.series_hash(), out.series_hash());
    }
}
