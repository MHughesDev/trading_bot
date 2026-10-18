//! Event sampling: CUSUM and strategy events (N-1.7, MD-01).
//!
//! Both return **bar indices** into the series the caller passed, ascending, and both are
//! point-in-time: an event at bar `i` is decided from bars `<= i` (CUSUM) or from an entry
//! timestamp at or before bar `i` (strategy events). Neither can name a later bar.

use serde::{Deserialize, Serialize};

/// How rows are chosen from the bar series. Part of the label spec, so it is part of the hash.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SamplingSpec {
    /// A symmetric CUSUM filter with threshold `h_sigma` multiples of the causal sigma.
    Cusum { h_sigma: f64 },
    /// Only the bars at which a strategy version entered.
    StrategyEvents { strategy_version: String },
}

/// The symmetric CUSUM filter (AFML 2.5.2.1) over 1-bar returns `ret`
/// (`ret[i]` is the return into bar `i`), with threshold `h_sigma * sigma[i - 1]`: the sigma
/// known *before* the bar, so a jump cannot widen its own threshold.
///
/// A bar whose threshold is not finite and positive neither accumulates nor fires: it cannot be
/// measured. After a fire the triggering sum restarts from zero.
#[must_use]
pub fn cusum_events(ret: &[f64], sigma: &[f64], h_sigma: f64) -> Vec<usize> {
    let mut events = Vec::new();
    if !(h_sigma.is_finite() && h_sigma > 0.0) {
        return events;
    }
    let (mut s_pos, mut s_neg) = (0.0_f64, 0.0_f64);
    for i in 1..ret.len().min(sigma.len()) {
        let (r, s) = (ret[i], sigma[i - 1]);
        if !(r.is_finite() && s.is_finite() && s > 0.0) {
            continue;
        }
        let h = h_sigma * s;
        s_pos = (s_pos + r).max(0.0);
        s_neg = (s_neg + r).min(0.0);
        if s_neg < -h {
            s_neg = 0.0;
            events.push(i);
        } else if s_pos > h {
            s_pos = 0.0;
            events.push(i);
        }
    }
    events
}

/// The bar index each strategy entry maps to: the **last bar at or before** the entry
/// timestamp (never a later one: a later bar carries information the entry did not have).
/// `bar_ts` must be ascending. An entry before the first bar maps to nothing. The result is
/// ascending and de-duplicated.
#[must_use]
pub fn strategy_event_indices(bar_ts: &[i64], entry_ts: &[i64]) -> Vec<usize> {
    let mut out: Vec<usize> = entry_ts
        .iter()
        .filter_map(|&t| bar_ts.partition_point(|&b| b <= t).checked_sub(1))
        .collect();
    out.sort_unstable();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::barrier::causal_sigma;

    /// Quiet noise with two planted jumps (up at 80, down at 150).
    fn planted() -> (Vec<f64>, Vec<usize>) {
        let mut close = vec![100.0];
        for i in 1..220 {
            let wiggle = if i % 2 == 0 { 0.0005 } else { -0.0005 };
            let r = match i {
                80 => 0.05,
                150 => -0.05,
                _ => wiggle,
            };
            close.push(close[i - 1] * (1.0 + r));
        }
        (close, vec![80, 150])
    }

    fn returns(close: &[f64]) -> Vec<f64> {
        let mut r = vec![f64::NAN];
        r.extend((1..close.len()).map(|i| close[i] / close[i - 1] - 1.0));
        r
    }

    #[test]
    fn cusum_events_land_exactly_on_the_planted_jump_bars() {
        let (close, jumps) = planted();
        // sigma frozen at the quiet level so the jump does not widen its own threshold
        let sigma = vec![0.0005; close.len()];
        let ev = cusum_events(&returns(&close), &sigma, 5.0);
        assert_eq!(ev, jumps);
    }

    #[test]
    fn a_later_regime_cannot_erase_an_earlier_event() {
        let (close, jumps) = planted();
        let ev = cusum_events(&returns(&close), &causal_sigma(&close, 20), 5.0);
        assert!(jumps.iter().all(|j| ev.contains(j)), "{ev:?}");
        // appending a wild tail changes only later sigma and later events
        let mut longer = close.clone();
        for i in 0..60 {
            let last = *longer.last().unwrap();
            longer.push(last * if i % 2 == 0 { 1.04 } else { 0.96 });
        }
        let ev2 = cusum_events(&returns(&longer), &causal_sigma(&longer, 20), 5.0);
        let head: Vec<usize> = ev2.iter().copied().filter(|&i| i < close.len()).collect();
        assert_eq!(head, ev, "events before the cut must not move");
    }

    #[test]
    fn an_unmeasurable_sigma_neither_fires_nor_accumulates() {
        let ret = vec![0.5, 0.5, 0.5];
        assert!(cusum_events(&ret, &[f64::NAN, 0.0, -1.0], 1.0).is_empty());
        assert!(cusum_events(&ret, &[0.01; 3], 0.0).is_empty());
    }

    #[test]
    fn strategy_events_map_to_the_last_bar_at_or_before_the_entry() {
        let bars = [100, 200, 300, 400];
        assert_eq!(strategy_event_indices(&bars, &[200]), vec![1]);
        assert_eq!(
            strategy_event_indices(&bars, &[250]),
            vec![1],
            "never the later bar 300"
        );
        assert_eq!(strategy_event_indices(&bars, &[999]), vec![3]);
        assert_eq!(strategy_event_indices(&bars, &[50]), Vec::<usize>::new());
        assert_eq!(
            strategy_event_indices(&bars, &[310, 301, 100, 100]),
            vec![0, 2]
        );
    }

    #[test]
    fn strategy_event_sampling_uses_only_entry_timestamps() {
        let bars: Vec<i64> = (0..50).map(|i| i * 60).collect();
        assert_eq!(strategy_event_indices(&bars, &[600, 1799]), vec![10, 29]);
    }

    #[test]
    fn the_spec_refuses_an_unknown_field() {
        assert!(
            serde_json::from_str::<SamplingSpec>(r#"{"kind":"cusum","h_sigma":2,"x":1}"#).is_err()
        );
        let s: SamplingSpec = serde_json::from_str(r#"{"kind":"cusum","h_sigma":2.0}"#).unwrap();
        assert_eq!(s, SamplingSpec::Cusum { h_sigma: 2.0 });
    }
}
