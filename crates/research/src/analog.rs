//! Analog search over instrument profiles (Set R-b RB-1.7, KM-08; AGENT-003 §7).
//!
//! "What has this looked like before?" is answered by a nearest-neighbour search over
//! an **explicit, standardised vector** (named statistics of a return window), never
//! an embedding: every coordinate means something a person can check, and a distance
//! between two vectors is a sum of differences in named quantities.
//!
//! # The time-exclusion zone
//!
//! The nearest neighbour of any window is the window next to it: overlapping windows
//! share most of their bars, so their statistics are nearly equal. Without a guard,
//! the "analog" of last month is last month shifted by a day, which tells nobody
//! anything. So an episode on the **same instrument** is excluded when it comes
//! within `horizon` of the query's own window, on either side, where `horizon` is
//! the length of the window being asked about: whatever it was, it is not a second
//! observation. Episodes on other instruments are not excluded by time: a different
//! market at the same moment is a legitimate analog (ADR-RB-07).
//!
//! The query's own episode is excluded by id even when it lies outside the zone.

use serde::{Deserialize, Serialize};

/// What each coordinate of [`profile_vector`] is, in order.
pub const PROFILE_FIELDS: [&str; 6] = [
    "volatility",
    "mean_return",
    "skew",
    "excess_kurtosis",
    "lag1_autocorrelation",
    "max_drawdown",
];

/// The explicit profile vector of a window of simple returns. Empty for fewer than
/// eight returns: six moments from three points would be numbers, not a profile.
#[must_use]
pub fn profile_vector(returns: &[f64]) -> Vec<f64> {
    let n = returns.len();
    if n < 8 {
        return Vec::new();
    }
    let nf = n as f64;
    let mean = returns.iter().sum::<f64>() / nf;
    let var = returns.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / nf;
    let sd = var.sqrt();
    let (skew, kurt) = if sd > 0.0 {
        (
            returns
                .iter()
                .map(|r| ((r - mean) / sd).powi(3))
                .sum::<f64>()
                / nf,
            returns
                .iter()
                .map(|r| ((r - mean) / sd).powi(4))
                .sum::<f64>()
                / nf
                - 3.0,
        )
    } else {
        (0.0, 0.0)
    };
    let ac1 = if var > 0.0 {
        returns
            .windows(2)
            .map(|w| (w[0] - mean) * (w[1] - mean))
            .sum::<f64>()
            / (nf * var)
    } else {
        0.0
    };
    let (mut equity, mut peak, mut dd) = (1.0_f64, 1.0_f64, 0.0_f64);
    for r in returns {
        equity *= 1.0 + r;
        peak = peak.max(equity);
        dd = dd.max(1.0 - equity / peak);
    }
    vec![sd, mean, skew, kurt, ac1, dd]
}

/// One stored window.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Episode {
    pub id: String,
    pub instrument: String,
    /// Inclusive bounds, in any monotone integer unit (epoch days, bar index).
    pub start: i64,
    pub end: i64,
    pub vector: Vec<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Query {
    /// The episode being asked about, when it is itself stored.
    pub id: Option<String>,
    pub instrument: String,
    pub start: i64,
    pub end: i64,
    pub vector: Vec<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Analog {
    pub id: String,
    pub instrument: String,
    pub start: i64,
    pub end: i64,
    pub distance: f64,
}

/// Per-coordinate mean and standard deviation of a pool, for standardising.
#[derive(Clone, Debug, PartialEq)]
pub struct Standardiser {
    mean: Vec<f64>,
    sd: Vec<f64>,
}

impl Standardiser {
    /// Fit over every vector of one length (the first one's). A coordinate with no
    /// spread keeps scale 1 so it contributes its raw difference, not NaN.
    #[must_use]
    pub fn fit(pool: &[Episode]) -> Self {
        let dim = pool.first().map_or(0, |e| e.vector.len());
        let rows: Vec<&Vec<f64>> = pool
            .iter()
            .map(|e| &e.vector)
            .filter(|v| v.len() == dim)
            .collect();
        let n = rows.len().max(1) as f64;
        let mean: Vec<f64> = (0..dim)
            .map(|d| rows.iter().map(|r| r[d]).sum::<f64>() / n)
            .collect();
        let sd = (0..dim)
            .map(|d| {
                let v = rows.iter().map(|r| (r[d] - mean[d]).powi(2)).sum::<f64>() / n;
                if v.sqrt() > 1e-12 {
                    v.sqrt()
                } else {
                    1.0
                }
            })
            .collect();
        Self { mean, sd }
    }

    #[must_use]
    pub fn apply(&self, v: &[f64]) -> Vec<f64> {
        v.iter()
            .enumerate()
            .map(|(i, x)| {
                (x - self.mean.get(i).copied().unwrap_or(0.0))
                    / self.sd.get(i).copied().unwrap_or(1.0)
            })
            .collect()
    }
}

/// Whether `e` is the same episode as the query by the exclusion rule.
#[must_use]
pub fn excluded(q: &Query, e: &Episode, horizon: i64) -> bool {
    if q.id.as_deref() == Some(e.id.as_str()) {
        return true;
    }
    e.instrument == q.instrument && e.start <= q.end + horizon && e.end >= q.start - horizon
}

/// The `k` nearest analogs of `q` in `pool`, nearest first, excluding the query's own
/// episode and anything on the same instrument within `horizon` of it.
///
/// Distances are Euclidean over vectors standardised by the pool. An episode whose
/// vector length differs from the query's is skipped, not padded.
#[must_use]
pub fn analogs(q: &Query, pool: &[Episode], k: usize, horizon: i64) -> Vec<Analog> {
    let st = Standardiser::fit(pool);
    let qv = st.apply(&q.vector);
    let mut out: Vec<Analog> = pool
        .iter()
        .filter(|e| e.vector.len() == q.vector.len() && !excluded(q, e, horizon))
        .map(|e| {
            let ev = st.apply(&e.vector);
            let d = qv
                .iter()
                .zip(&ev)
                .map(|(a, b)| (a - b).powi(2))
                .sum::<f64>()
                .sqrt();
            Analog {
                id: e.id.clone(),
                instrument: e.instrument.clone(),
                start: e.start,
                end: e.end,
                distance: d,
            }
        })
        .collect();
    out.sort_by(|a, b| {
        a.distance
            .total_cmp(&b.distance)
            .then_with(|| a.id.cmp(&b.id))
    });
    out.truncate(k);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ep(id: &str, inst: &str, s: i64, e: i64, v: &[f64]) -> Episode {
        Episode {
            id: id.into(),
            instrument: inst.into(),
            start: s,
            end: e,
            vector: v.to_vec(),
        }
    }

    fn q(s: i64, e: i64) -> Query {
        Query {
            id: Some("self".into()),
            instrument: "A".into(),
            start: s,
            end: e,
            vector: vec![1.0, 1.0],
        }
    }

    #[test]
    fn the_profile_vector_is_explicit_and_named() {
        let r: Vec<f64> = (0..40)
            .map(|i| if i % 2 == 0 { 0.01 } else { -0.008 })
            .collect();
        let v = profile_vector(&r);
        assert_eq!(v.len(), PROFILE_FIELDS.len());
        assert!(v[0] > 0.0, "volatility");
        assert!(
            v[4] < 0.0,
            "an alternating series has negative lag-1 autocorrelation"
        );
        assert!(v[5] >= 0.0);
        assert!(profile_vector(&r[..5]).is_empty());
    }

    #[test]
    fn the_query_window_and_its_neighbours_are_never_returned() {
        // The query is days 100..130. An identical-vector episode overlapping it, one
        // starting just after it, and one just before: all on the same instrument.
        let pool = vec![
            ep("self", "A", 100, 130, &[1.0, 1.0]),
            ep("overlap", "A", 120, 150, &[1.0, 1.0]),
            ep("next_door", "A", 131, 161, &[1.0, 1.0]),
            ep("before", "A", 60, 99, &[1.0, 1.0]),
            ep("far", "A", 400, 430, &[3.0, 3.0]),
            ep("other_market_same_time", "B", 100, 130, &[1.1, 1.1]),
        ];
        let out = analogs(&q(100, 130), &pool, 10, 30);
        let ids: Vec<&str> = out.iter().map(|a| a.id.as_str()).collect();
        assert!(!ids.contains(&"self"));
        assert!(!ids.contains(&"overlap"));
        assert!(
            !ids.contains(&"next_door"),
            "inside the horizon after the window"
        );
        assert!(
            !ids.contains(&"before"),
            "inside the horizon before the window"
        );
        assert_eq!(ids, vec!["other_market_same_time", "far"]);
    }

    #[test]
    fn mutation_without_the_zone_the_nearest_analog_is_the_same_episode() {
        let pool = vec![
            ep("overlap", "A", 120, 150, &[1.0, 1.0]),
            ep("far", "A", 400, 430, &[3.0, 3.0]),
        ];
        // horizon 0 still excludes overlap; a *negative* horizon shrinks the zone below
        // the window and is how the guard would be removed: the overlap comes back first.
        let guarded = analogs(&q(100, 130), &pool, 1, 30);
        assert_eq!(guarded[0].id, "far");
        let unguarded = analogs(
            &Query {
                id: None,
                ..q(100, 130)
            },
            &pool,
            1,
            -1000,
        );
        assert_eq!(unguarded[0].id, "overlap");
    }

    #[test]
    fn distances_are_standardised_so_a_wide_coordinate_does_not_dominate() {
        let pool = vec![
            ep("a", "B", 0, 1, &[0.0, 1000.0]),
            ep("b", "B", 2, 3, &[1.0, 0.0]),
            ep("c", "B", 4, 5, &[0.0, -1000.0]),
        ];
        let q2 = Query {
            id: None,
            instrument: "A".into(),
            start: 10,
            end: 11,
            vector: vec![0.9, 10.0],
        };
        let out = analogs(&q2, &pool, 3, 5);
        // On raw units "a" and "c" are ~1000 away and "b" ~10; after standardising, "b" still wins
        // and the order is by the standardised distance.
        assert_eq!(out[0].id, "b");
    }

    #[test]
    fn a_vector_of_another_length_is_skipped() {
        let pool = vec![
            ep("short", "B", 0, 1, &[1.0]),
            ep("ok", "B", 2, 3, &[1.0, 1.0]),
        ];
        let out = analogs(&q(100, 130), &pool, 5, 30);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].id, "ok");
    }
}
