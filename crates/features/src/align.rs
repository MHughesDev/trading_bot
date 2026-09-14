//! Master-clock densification (SPEC §2, INV-11, AT-11).
//!
//! Five asset classes have five clocks; the UTC grid is the only one they share.
//! Sessions, holidays and outages are *attributes* of an observation, never the
//! index — so the reader densifies a sparse bar series onto the grid and every
//! value it carries forward travels with the age and quality of the observation
//! it came from.
//!
//! This is the **one** implementation of that contract. The dataset builder and
//! the live serve both call it; there is no second densifier and no path that
//! forward-fills without exposing age.

use dataplane::align::{densify, grid, Aligned, Obs};
use dataplane::feature::{Feature, FeatureRow};
use dataplane::quality::QualityFlags;

const MINUTE_NS: i64 = 60_000_000_000;

/// One sparse bar, with the provenance alignment needs.
///
/// `knowledge_ns` is when the bar became knowable, not when it happened:
/// alignment keys on knowledge time, because that is the only ordering a live
/// reader could have observed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BarObs {
    pub ts_ns: i64,
    pub knowledge_ns: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
    pub quality: QualityFlags,
}

/// A bar series densified onto the UTC grid.
///
/// `rows` is what features are computed over; `age_minutes[i]` and `quality[i]`
/// describe the observation row `i` was densified from. An age of 0 is a real
/// observation at that tick; anything larger is carried forward and says so.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MasterClockFrame {
    pub rows: Vec<FeatureRow>,
    pub age_minutes: Vec<i64>,
    pub quality: Vec<QualityFlags>,
}

impl MasterClockFrame {
    #[must_use]
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// The knowledge time behind each grid row: the tick minus how far the value
    /// was carried. This is what a diff between a live serve and a later
    /// recomputation compares, so both sides must read it from here.
    #[must_use]
    pub fn knowledge_ns(&self) -> Vec<i64> {
        self.rows
            .iter()
            .zip(&self.age_minutes)
            .map(|(r, age)| r.ts_ns - age * MINUTE_NS)
            .collect()
    }

    /// The last grid row at or before `knowledge_ns` -- the row a reader deciding
    /// at that moment would have been on. `None` if that moment is before the
    /// first observation.
    #[must_use]
    pub fn decision_at(&self, knowledge_ns: i64) -> Option<usize> {
        self.rows
            .partition_point(|r| r.ts_ns <= knowledge_ns)
            .checked_sub(1)
    }

    /// The staleness and quality of the inputs a feature value at `decision` was
    /// computed from: the age of the newest input, and the union of quality
    /// flags across the feature's declared window.
    ///
    /// The window, not just the decision row, because a feature that averages
    /// over an outage is as affected as one that reads a single stale quote.
    #[must_use]
    pub fn provenance(&self, lookback_bars: u32, decision: usize) -> (i64, QualityFlags) {
        if decision >= self.rows.len() {
            return (0, QualityFlags::NONE);
        }
        let span = lookback_bars.max(1) as usize;
        let first = decision.saturating_sub(span - 1);
        let quality = self.quality[first..=decision]
            .iter()
            .fold(QualityFlags::NONE, |a, b| a.union(*b));
        (self.age_minutes[decision], quality)
    }

    /// `provenance` for a resolved feature, using its declared lookback.
    #[must_use]
    pub fn provenance_for(&self, feature: &dyn Feature, decision: usize) -> (i64, QualityFlags) {
        self.provenance(feature.def().lookback_bars, decision)
    }
}

/// Densify `obs` onto the UTC grid of `step_ns` ticks.
///
/// Ticks before the first observation carry nothing and are dropped: there is no
/// honest value for them. From the first observation onward every tick has a
/// value, and the age column is what distinguishes a fresh bar from one carried
/// across a weekend, a holiday or a venue outage.
///
/// No age bound is applied. Rule 1 of §2 is "emit staleness, never forward-fill
/// *silently*" — a bound would drop the value and leave the model with a hole
/// where it could instead see "this is 4300 minutes old" and decide for itself.
#[must_use]
pub fn densify_bars(obs: &[BarObs], step_ns: i64) -> MasterClockFrame {
    if obs.is_empty() || step_ns <= 0 {
        return MasterClockFrame::default();
    }
    let mut sorted: Vec<BarObs> = obs.to_vec();
    sorted.sort_by_key(|o| o.knowledge_ns);

    let start = sorted[0].knowledge_ns;
    let end = sorted[sorted.len() - 1].knowledge_ns + step_ns;
    let ticks = grid(start, end, step_ns);
    if ticks.is_empty() {
        return MasterClockFrame::default();
    }

    // One densification per field, all against the same grid and the same
    // backward-only as-of, so a row is always one whole observation.
    let field = |f: fn(&BarObs) -> f64| -> Vec<Aligned> {
        let points: Vec<Obs> = sorted
            .iter()
            .map(|o| Obs {
                knowledge_ns: o.knowledge_ns,
                value: f(o),
                quality: o.quality,
            })
            .collect();
        densify(&points, &ticks, None)
    };
    let open = field(|o| o.open);
    let high = field(|o| o.high);
    let low = field(|o| o.low);
    let close = field(|o| o.close);
    let volume = field(|o| o.volume);

    let mut frame = MasterClockFrame::default();
    for (i, &tick) in ticks.iter().enumerate() {
        let (Some(o), Some(h), Some(l), Some(c), Some(v)) =
            (open[i].value, high[i].value, low[i].value, close[i].value, volume[i].value)
        else {
            // Before the first observation.
            continue;
        };
        let age = close[i].age_minutes.unwrap_or(0);
        // The row's timestamp is the grid tick it occupies: the index is the
        // master clock, and the observation's own knowledge time is recoverable
        // from the tick minus the age (see `knowledge_ns`).
        frame.rows.push(FeatureRow {
            ts_ns: tick,
            open: o,
            high: h,
            low: l,
            close: c,
            volume: v,
        });
        frame.age_minutes.push(age);
        frame.quality.push(close[i].quality);
    }
    frame
}

/// Densify onto the minute grid — the master clock itself.
#[must_use]
pub fn densify_to_master_clock(obs: &[BarObs]) -> MasterClockFrame {
    densify_bars(obs, MINUTE_NS)
}

/// The three column names a feature emits. A value without its two companions
/// is not an aligned feature (INV-11).
#[must_use]
pub fn companion_columns(feature: &str) -> [String; 3] {
    dataplane::align::companion_columns(feature)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(x: i64) -> i64 {
        x * MINUTE_NS
    }

    fn bar(minute: i64, close: f64, quality: QualityFlags) -> BarObs {
        BarObs {
            ts_ns: m(minute),
            knowledge_ns: m(minute),
            open: close,
            high: close,
            low: close,
            close,
            volume: 1.0,
            quality,
        }
    }

    #[test]
    fn a_contiguous_series_densifies_to_itself_with_zero_age() {
        let obs: Vec<BarObs> = (0..5).map(|i| bar(i, 100.0 + i as f64, QualityFlags::NONE)).collect();
        let f = densify_to_master_clock(&obs);
        assert_eq!(f.len(), 5);
        assert!(f.age_minutes.iter().all(|a| *a == 0));
        assert_eq!(f.rows[4].close, 104.0);
    }

    /// AT-11: a gap — a venue outage, a holiday, a crypto weekend — is carried
    /// forward, and the age column says exactly how far.
    #[test]
    fn a_gap_is_carried_forward_with_its_age_exposed() {
        let obs = vec![
            bar(0, 100.0, QualityFlags::NONE),
            bar(5, 101.0, QualityFlags::NONE),
        ];
        let f = densify_to_master_clock(&obs);
        assert_eq!(f.len(), 6, "the gap is filled, not skipped");
        assert_eq!(f.age_minutes, vec![0, 1, 2, 3, 4, 0]);
        for i in 1..5 {
            assert_eq!(f.rows[i].close, 100.0, "carried forward");
            assert!(
                f.quality[i].contains(QualityFlags::INTERPOLATED),
                "a carried value says so"
            );
        }
        assert!(!f.quality[0].contains(QualityFlags::INTERPOLATED));
        assert!(!f.quality[5].contains(QualityFlags::INTERPOLATED));
    }

    #[test]
    fn nothing_is_emitted_before_the_first_observation() {
        let obs = vec![bar(10, 100.0, QualityFlags::NONE)];
        let f = densify_to_master_clock(&obs);
        assert_eq!(f.len(), 1);
        assert_eq!(f.rows[0].ts_ns, m(10));
    }

    #[test]
    fn source_quality_flags_survive_densification() {
        let obs = vec![bar(0, 100.0, QualityFlags::SUSPECT_VOLUME)];
        let f = densify_to_master_clock(&obs);
        assert!(f.quality[0].contains(QualityFlags::SUSPECT_VOLUME));
    }

    /// A feature that averages across an outage is as affected as one reading a
    /// single stale quote, so provenance unions quality over the whole window.
    #[test]
    fn provenance_unions_quality_over_the_declared_window() {
        let obs = vec![
            bar(0, 100.0, QualityFlags::SUSPECT_VOLUME),
            bar(1, 101.0, QualityFlags::NONE),
            bar(2, 102.0, QualityFlags::NONE),
        ];
        let f = densify_to_master_clock(&obs);
        let (age, q) = f.provenance(3, 2);
        assert_eq!(age, 0, "the newest input is fresh");
        assert!(q.contains(QualityFlags::SUSPECT_VOLUME), "an older input was not");

        let (_, narrow) = f.provenance(1, 2);
        assert!(!narrow.contains(QualityFlags::SUSPECT_VOLUME), "outside the window");
    }

    #[test]
    fn provenance_past_the_end_is_empty_not_a_panic() {
        let f = densify_to_master_clock(&[bar(0, 1.0, QualityFlags::NONE)]);
        assert_eq!(f.provenance(5, 99), (0, QualityFlags::NONE));
    }

    #[test]
    fn an_empty_series_densifies_to_nothing() {
        assert!(densify_to_master_clock(&[]).is_empty());
    }

    #[test]
    fn coarser_bars_densify_on_their_own_period() {
        let obs = vec![bar(0, 100.0, QualityFlags::NONE), bar(15, 101.0, QualityFlags::NONE)];
        let f = densify_bars(&obs, 5 * MINUTE_NS);
        assert_eq!(f.len(), 4);
        assert_eq!(f.age_minutes, vec![0, 5, 10, 0]);
    }

    #[test]
    fn knowledge_times_survive_the_carry() {
        let obs = vec![bar(0, 100.0, QualityFlags::NONE), bar(3, 101.0, QualityFlags::NONE)];
        let f = densify_to_master_clock(&obs);
        assert_eq!(f.knowledge_ns(), vec![m(0), m(0), m(0), m(3)]);
    }

    #[test]
    fn decision_at_finds_the_row_a_reader_would_have_been_on() {
        let obs = vec![bar(0, 100.0, QualityFlags::NONE), bar(3, 101.0, QualityFlags::NONE)];
        let f = densify_to_master_clock(&obs);
        assert_eq!(f.decision_at(m(2)), Some(2));
        assert_eq!(f.decision_at(m(3)), Some(3));
        assert_eq!(f.decision_at(m(99)), Some(3));
        assert_eq!(f.decision_at(m(0) - 1), None);
    }
}
