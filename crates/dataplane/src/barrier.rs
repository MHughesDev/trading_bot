//! Triple-barrier labels on the intrabar high/low path (N-1.4, MD-03).
//!
//! The close-only path misses a bar that wicks through a barrier and closes back inside.
//! Here a barrier is touched when the bar's high (target) or low (stop) crosses it, and when
//! one bar's range spans both barriers the **stop is taken** (the pessimistic reading, as in
//! FEAT-005 section 3.4: the order inside a bar is unknowable).
//!
//! Point-in-time: the barrier width at event `i` is `k * sigma[i]` where `sigma[i]` is
//! [`causal_sigma`] over bars `<= i` only, so appending later bars never changes an earlier
//! event's barriers. The *label* looks forward by definition; its span `[i, end]` is what
//! purge and embargo are derived from.

use serde::{Deserialize, Serialize};

/// Where a barrier looks for a touch.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BarrierPath {
    /// The bar's high and low. A bar spanning both barriers labels as the stop.
    #[default]
    HighLow,
    /// The close only, for reproducing an older label.
    Close,
}

/// One bar's prices; `high`/`low` are ignored on the close path.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PriceBar {
    pub high: f64,
    pub low: f64,
    pub close: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BarrierConfig {
    /// Target distance in multiples of sigma.
    pub pt_sigma: f64,
    /// Stop distance in multiples of sigma.
    pub sl_sigma: f64,
    /// Vertical barrier: the label is decided within this many bars.
    pub max_bars: usize,
    pub path: BarrierPath,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Target,
    Stop,
    Timeout,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BarrierLabel {
    pub event: usize,
    /// The bar that decided the label (touch bar, or `event + max_bars` on a timeout).
    pub end: usize,
    pub outcome: Outcome,
    /// `+1.0` target, `-1.0` stop, `0.0` timeout.
    pub label: f64,
    /// The close-to-close return from the event to `end`.
    pub ret: f64,
}

/// Rolling standard deviation of 1-bar simple returns over the last `window` returns ending
/// at each bar. `NaN` until `window` returns exist. Element `i` reads bars `<= i` only.
#[must_use]
pub fn causal_sigma(close: &[f64], window: usize) -> Vec<f64> {
    let n = close.len();
    let mut out = vec![f64::NAN; n];
    if window < 2 {
        return out;
    }
    // ret[k - 1] is the return into bar k.
    let ret: Vec<f64> = (1..n)
        .map(|k| {
            if close[k - 1] == 0.0 {
                f64::NAN
            } else {
                close[k] / close[k - 1] - 1.0
            }
        })
        .collect();
    // The window ending at bar i is the `window` returns into bars i-window+1..=i.
    for i in window..n {
        let w = &ret[i - window..i];
        if w.iter().any(|x| !x.is_finite()) {
            continue;
        }
        let mean = w.iter().sum::<f64>() / window as f64;
        let var = w.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (window as f64 - 1.0);
        out[i] = var.sqrt();
    }
    out
}

/// Label the event at bar `i`. `None` when the barrier width is unusable (`sigma` not finite
/// and positive) or the vertical barrier is not yet complete (`i + max_bars` is past the end):
/// an unfinished label is not a label.
#[must_use]
pub fn label_event(
    bars: &[PriceBar],
    i: usize,
    sigma: f64,
    cfg: &BarrierConfig,
) -> Option<BarrierLabel> {
    if !(sigma.is_finite() && sigma > 0.0) || cfg.max_bars == 0 || i >= bars.len() {
        return None;
    }
    let last = i.checked_add(cfg.max_bars)?;
    if last >= bars.len() {
        return None;
    }
    let entry = bars[i].close;
    if !(entry.is_finite() && entry > 0.0) {
        return None;
    }
    let upper = entry * (1.0 + cfg.pt_sigma * sigma);
    let lower = entry * (1.0 - cfg.sl_sigma * sigma);
    for (j, b) in bars.iter().enumerate().take(last + 1).skip(i + 1) {
        let (hit_up, hit_dn) = match cfg.path {
            BarrierPath::HighLow => (b.high >= upper, b.low <= lower),
            BarrierPath::Close => (b.close >= upper, b.close <= lower),
        };
        let outcome = match (hit_up, hit_dn) {
            (_, true) => Some(Outcome::Stop), // both in one bar: the stop is taken
            (true, false) => Some(Outcome::Target),
            (false, false) => None,
        };
        if let Some(outcome) = outcome {
            return Some(done(i, j, outcome, entry, bars[j].close));
        }
    }
    Some(done(i, last, Outcome::Timeout, entry, bars[last].close))
}

fn done(event: usize, end: usize, outcome: Outcome, entry: f64, exit: f64) -> BarrierLabel {
    BarrierLabel {
        event,
        end,
        outcome,
        label: match outcome {
            Outcome::Target => 1.0,
            Outcome::Stop => -1.0,
            Outcome::Timeout => 0.0,
        },
        ret: exit / entry - 1.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(n: usize) -> Vec<PriceBar> {
        vec![
            PriceBar {
                high: 100.0,
                low: 100.0,
                close: 100.0
            };
            n
        ]
    }

    fn cfg(path: BarrierPath) -> BarrierConfig {
        BarrierConfig {
            pt_sigma: 2.0,
            sl_sigma: 1.0,
            max_bars: 5,
            path,
        }
    }

    // sigma 1% -> target 102, stop 99.

    #[test]
    fn a_bar_spanning_both_barriers_labels_as_the_stop() {
        let mut b = flat(10);
        b[3] = PriceBar {
            high: 103.0,
            low: 98.0,
            close: 100.0,
        };
        let l = label_event(&b, 0, 0.01, &cfg(BarrierPath::HighLow)).unwrap();
        assert_eq!((l.outcome, l.end, l.label), (Outcome::Stop, 3, -1.0));
    }

    #[test]
    fn a_wick_only_through_the_target_labels_as_the_target() {
        let mut b = flat(10);
        b[2] = PriceBar {
            high: 102.5,
            low: 100.0,
            close: 100.2,
        };
        let l = label_event(&b, 0, 0.01, &cfg(BarrierPath::HighLow)).unwrap();
        assert_eq!((l.outcome, l.end, l.label), (Outcome::Target, 2, 1.0));
        // The close path never sees the wick.
        let c = label_event(&b, 0, 0.01, &cfg(BarrierPath::Close)).unwrap();
        assert_eq!(c.outcome, Outcome::Timeout);
    }

    #[test]
    fn the_close_path_equals_the_high_low_path_on_a_series_with_no_wicks() {
        let close = [
            100.0, 100.5, 101.0, 102.5, 103.0, 99.0, 98.0, 98.5, 97.0, 99.0,
        ];
        let b: Vec<PriceBar> = close
            .iter()
            .map(|&c| PriceBar {
                high: c,
                low: c,
                close: c,
            })
            .collect();
        for i in 0..4 {
            assert_eq!(
                label_event(&b, i, 0.01, &cfg(BarrierPath::HighLow)),
                label_event(&b, i, 0.01, &cfg(BarrierPath::Close)),
                "event {i}"
            );
        }
    }

    #[test]
    fn a_timeout_and_an_unfinished_label_are_distinct() {
        let b = flat(10);
        let t = label_event(&b, 2, 0.01, &cfg(BarrierPath::HighLow)).unwrap();
        assert_eq!((t.outcome, t.end), (Outcome::Timeout, 7));
        // Event 6 + 5 bars runs past the end: no label, not a timeout.
        assert!(label_event(&b, 6, 0.01, &cfg(BarrierPath::HighLow)).is_none());
        assert!(label_event(&b, 2, f64::NAN, &cfg(BarrierPath::HighLow)).is_none());
        assert!(label_event(&b, 2, 0.0, &cfg(BarrierPath::HighLow)).is_none());
    }

    #[test]
    fn a_label_ignores_bars_past_its_vertical_barrier() {
        let mut b = flat(12);
        let before = label_event(&b, 0, 0.01, &cfg(BarrierPath::HighLow)).unwrap();
        b[9] = PriceBar {
            high: 150.0,
            low: 50.0,
            close: 100.0,
        };
        assert_eq!(
            label_event(&b, 0, 0.01, &cfg(BarrierPath::HighLow)),
            Some(before)
        );
    }

    #[test]
    fn sigma_at_a_bar_never_reads_a_later_bar() {
        let close: Vec<f64> = (0..80)
            .map(|i| 100.0 + f64::from(i % 7) + f64::from(i) * 0.1)
            .collect();
        let full = causal_sigma(&close, 20);
        for cut in [30usize, 45, 79] {
            let part = causal_sigma(&close[..=cut], 20);
            for i in 0..=cut {
                assert!(
                    (part[i].is_nan() && full[i].is_nan()) || (part[i] - full[i]).abs() < 1e-15,
                    "sigma[{i}] changed when bars after {cut} were appended"
                );
            }
        }
        assert!(full[19].is_nan() && full[20].is_finite());
    }
}
