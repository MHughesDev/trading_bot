//! UTC minute-grid densification with exposed staleness (SPEC §2, INV-11).
//!
//! Every aligned value travels with `age_minutes` and `quality`. Forward-filling
//! without exposing age is how a model learns to trade a stale quote.

use serde::{Deserialize, Serialize};

use crate::asof::asof_backward;
use crate::quality::QualityFlags;

const MINUTE_NS: i64 = 60_000_000_000;

/// A sparse observation on one clock.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Obs {
    /// Knowledge time in ns — alignment keys on when a value was knowable.
    pub knowledge_ns: i64,
    pub value: f64,
    pub quality: QualityFlags,
}

/// The `(value, age, quality)` triple emitted per grid minute.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Aligned {
    pub value: Option<f64>,
    pub age_minutes: Option<i64>,
    pub quality: QualityFlags,
}

/// Column names emitted for a feature: the value plus its two mandatory companions.
#[must_use]
pub fn companion_columns(feature: &str) -> [String; 3] {
    [
        feature.to_string(),
        format!("{feature}_age_minutes"),
        format!("{feature}_quality"),
    ]
}

/// The UTC minute grid covering `[start_ns, end_ns)`.
#[must_use]
pub fn minute_grid(start_ns: i64, end_ns: i64) -> Vec<i64> {
    grid(start_ns, end_ns, MINUTE_NS)
}

/// The UTC grid of `step_ns` ticks covering `[start_ns, end_ns)`, anchored to the
/// epoch so the same span always yields the same ticks regardless of where the
/// data happens to start.
///
/// The master clock is the minute grid (SPEC §2); coarser bar periods share its
/// anchor, which is what makes a 5m series and a 1m series line up exactly.
#[must_use]
pub fn grid(start_ns: i64, end_ns: i64, step_ns: i64) -> Vec<i64> {
    if step_ns <= 0 {
        return Vec::new();
    }
    let first = start_ns.div_euclid(step_ns) * step_ns;
    let first = if first < start_ns { first + step_ns } else { first };
    if end_ns <= first {
        return Vec::new();
    }
    let ticks = (end_ns - first + step_ns - 1) / step_ns;
    (0..ticks).map(|i| first + i * step_ns).collect()
}

/// Densify one sparse series onto `grid`. Values older than `max_age_minutes` are
/// emitted as missing (with their age still reported), never silently carried.
#[must_use]
pub fn densify(obs: &[Obs], grid: &[i64], max_age_minutes: Option<i64>) -> Vec<Aligned> {
    let mut sorted: Vec<Obs> = obs.to_vec();
    sorted.sort_by_key(|o| o.knowledge_ns);
    let keys: Vec<i64> = sorted.iter().map(|o| o.knowledge_ns).collect();
    asof_backward(grid, &keys, None)
        .into_iter()
        .zip(grid)
        .map(|(idx, &g)| match idx {
            None => Aligned { value: None, age_minutes: None, quality: QualityFlags::NONE },
            Some(i) => {
                let o = sorted[i];
                let age = (g - o.knowledge_ns).div_euclid(MINUTE_NS);
                let mut quality = o.quality;
                if age > 0 {
                    quality |= QualityFlags::INTERPOLATED;
                }
                let stale = max_age_minutes.is_some_and(|m| age > m);
                if stale {
                    quality |= QualityFlags::STALE_QUOTE;
                }
                Aligned {
                    value: (!stale).then_some(o.value),
                    age_minutes: Some(age),
                    quality,
                }
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(x: i64) -> i64 {
        x * MINUTE_NS
    }

    /// AT-11 (unit half): staleness is exposed across a gap such as a weekend.
    #[test]
    fn age_and_quality_are_emitted_across_a_gap() {
        let obs = vec![
            Obs { knowledge_ns: m(0), value: 1.0, quality: QualityFlags::NONE },
            Obs { knowledge_ns: m(3), value: 2.0, quality: QualityFlags::NONE },
        ];
        let grid = minute_grid(m(0), m(10));
        let out = densify(&obs, &grid, Some(4));
        assert_eq!(out[0], Aligned { value: Some(1.0), age_minutes: Some(0), quality: QualityFlags::NONE });
        assert_eq!(out[2].age_minutes, Some(2));
        assert!(out[2].quality.contains(QualityFlags::INTERPOLATED));
        assert_eq!(out[7].value, Some(2.0));
        assert_eq!(out[8].value, None, "age 5 exceeds the 4-minute bound");
        assert_eq!(out[8].age_minutes, Some(5), "age is still reported");
        assert!(out[8].quality.contains(QualityFlags::STALE_QUOTE));
    }

    #[test]
    fn nothing_before_first_observation() {
        let obs = vec![Obs { knowledge_ns: m(5), value: 1.0, quality: QualityFlags::NONE }];
        let out = densify(&obs, &minute_grid(m(0), m(6)), None);
        assert!(out[..5].iter().all(|a| a.value.is_none() && a.age_minutes.is_none()));
    }

    #[test]
    fn companions_named() {
        assert_eq!(companion_columns("rv_5m"), ["rv_5m".to_string(), "rv_5m_age_minutes".into(), "rv_5m_quality".into()]);
    }

    #[test]
    fn grid_is_utc_minute_aligned() {
        assert_eq!(minute_grid(m(1) + 5, m(3)), vec![m(2)]);
    }

    #[test]
    fn coarser_grids_share_the_minute_grid_anchor() {
        let five = grid(m(0), m(20), 5 * MINUTE_NS);
        assert_eq!(five, vec![m(0), m(5), m(10), m(15)]);
        // Every 5m tick is also a minute-grid tick: the clocks line up.
        let minutes = minute_grid(m(0), m(20));
        assert!(five.iter().all(|t| minutes.contains(t)));
    }

    #[test]
    fn a_non_positive_step_yields_no_grid() {
        assert!(grid(m(0), m(10), 0).is_empty());
    }
}
