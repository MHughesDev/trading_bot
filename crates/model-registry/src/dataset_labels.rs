//! The two label-spec options the dataset build now honours natively (N-1.4, N-1.7):
//! the high/low triple barrier and CUSUM event sampling.
//!
//! [`apply`] runs on the frame [`features::build_aligned_training_frame`] produced, before it
//! is accumulated: it (1) keeps only the rows at CUSUM event bars when the spec samples, (2)
//! replaces the forward-return label with the triple-barrier label when the spec names a
//! barrier path, dropping rows whose barrier is unusable or unfinished, and (3) recomputes
//! uniqueness weights over the surviving label spans. A spec with neither option is returned
//! untouched, so every existing dataset id and frame digest is unchanged.
//!
//! Point-in-time: a CUSUM event at bar `i` and the barrier width at `i` read bars `<= i`
//! (`dataplane::barrier::causal_sigma`); only the label itself looks forward.

use std::collections::{HashMap, HashSet};

use dataplane::barrier::{causal_sigma, label_event, BarrierConfig, PriceBar};
use dataplane::label::{uniqueness_weights, LabelInterval, LabelSpec, SampleWeightMethod};
use dataplane::sampling::{cusum_events, SamplingSpec};
use features::align::{densify_bars, BarObs};
use features::TrainingFrame;

/// Look-back, in bars, of the causal sigma that sizes the barriers and the CUSUM threshold.
pub const SIGMA_WINDOW: usize = 50;

/// Why a spec could not be applied.
#[derive(Debug, PartialEq, Eq)]
pub enum LabelError {
    /// `strategy_events` needs the strategy's entry timestamps, which the dataset build does
    /// not have; the trainer samples those (ADR-N-PY2-04).
    StrategyEventsNeedEntries,
    /// A barrier path without the two multiples.
    BarrierMultiples,
}

impl std::fmt::Display for LabelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::StrategyEventsNeedEntries => f.write_str(
                "strategy_events sampling needs the strategy's entry timestamps; the dataset build cannot resolve them",
            ),
            Self::BarrierMultiples => {
                f.write_str("a barrier path needs two pt/sl multiples (target, stop)")
            }
        }
    }
}

impl std::error::Error for LabelError {}

/// Apply the spec's barrier path and sampling to `frame`.
///
/// # Errors
/// [`LabelError`] for a spec the build cannot honour; never a silent fallback.
pub fn apply(
    frame: TrainingFrame,
    obs: &[BarObs],
    step_ns: i64,
    spec: &LabelSpec,
) -> Result<TrainingFrame, LabelError> {
    if spec.barrier_path.is_none() && spec.sampling.is_none() {
        return Ok(frame);
    }
    if matches!(spec.sampling, Some(SamplingSpec::StrategyEvents { .. })) {
        return Err(LabelError::StrategyEventsNeedEntries);
    }
    let barrier = match spec.barrier_path {
        Some(path) => {
            let [pt, sl] = spec.pt_sl_multiples[..] else {
                return Err(LabelError::BarrierMultiples);
            };
            Some(BarrierConfig {
                pt_sigma: pt,
                sl_sigma: sl,
                max_bars: spec.horizon_bars as usize,
                path,
            })
        }
        None => None,
    };

    let clock = densify_bars(obs, step_ns);
    let n = clock.rows.len();
    let close: Vec<f64> = clock.rows.iter().map(|r| r.close).collect();
    let bars: Vec<PriceBar> = clock
        .rows
        .iter()
        .map(|r| PriceBar {
            high: r.high,
            low: r.low,
            close: r.close,
        })
        .collect();
    let by_ts: HashMap<i64, usize> = clock
        .rows
        .iter()
        .enumerate()
        .map(|(i, r)| (r.ts_ns, i))
        .collect();
    let sigma = causal_sigma(&close, SIGMA_WINDOW);

    let events: Option<HashSet<usize>> = match &spec.sampling {
        Some(SamplingSpec::Cusum { h_sigma }) => {
            let ret: Vec<f64> = (0..n)
                .map(|i| {
                    if i == 0 || close[i - 1] == 0.0 {
                        f64::NAN
                    } else {
                        close[i] / close[i - 1] - 1.0
                    }
                })
                .collect();
            Some(cusum_events(&ret, &sigma, *h_sigma).into_iter().collect())
        }
        _ => None,
    };

    let rows = frame.row_count();
    let mut keep: Vec<usize> = Vec::with_capacity(rows);
    let mut label: Vec<f64> = Vec::with_capacity(rows);
    let mut span: Vec<LabelInterval> = Vec::with_capacity(rows);
    for r in 0..rows {
        let Some(&i) = by_ts.get(&frame.ts_ns[r]) else {
            continue;
        };
        if events.as_ref().is_some_and(|e| !e.contains(&i)) {
            continue;
        }
        if let Some(cfg) = &barrier {
            let Some(l) = label_event(&bars, i, sigma[i], cfg) else {
                continue;
            };
            label.push(l.label);
            span.push(LabelInterval { t0: i, t1: l.end });
        } else {
            label.push(frame.label[r]);
            span.push(LabelInterval {
                t0: i,
                t1: (i + spec.horizon_bars as usize).min(n.saturating_sub(1)),
            });
        }
        keep.push(r);
    }

    let pick = |v: &[f64]| keep.iter().map(|&r| v[r]).collect::<Vec<f64>>();
    let sample_weight = if spec.sample_weight_method == SampleWeightMethod::Uniqueness && n > 0 {
        uniqueness_weights(&span, n)
    } else if frame.sample_weight.len() == rows {
        pick(&frame.sample_weight)
    } else {
        Vec::new()
    };
    Ok(TrainingFrame {
        feature_names: frame.feature_names.clone(),
        ts_ns: keep.iter().map(|&r| frame.ts_ns[r]).collect(),
        columns: frame.columns.iter().map(|c| pick(c)).collect(),
        age_minutes: frame
            .age_minutes
            .iter()
            .map(|c| keep.iter().map(|&r| c[r]).collect())
            .collect(),
        quality: frame
            .quality
            .iter()
            .map(|c| keep.iter().map(|&r| c[r]).collect())
            .collect(),
        label,
        sample_weight,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use dataplane::barrier::BarrierPath;
    use dataplane::quality::QualityFlags;

    const STEP: i64 = 60_000_000_000;

    /// 300 one-minute bars: a quiet zigzag with a +5% jump at bar 120 whose bar also has a wick.
    fn bars() -> Vec<BarObs> {
        let mut c = 100.0_f64;
        (0..300)
            .map(|i| {
                let r = match i {
                    120 => 0.05,
                    _ if i % 2 == 0 => 0.0005,
                    _ => -0.0005,
                };
                let open = c;
                c *= 1.0 + r;
                let (high, low) = if i == 200 {
                    // a wick far through both sides of any barrier, closing back at the open
                    (c * 1.2, c * 0.8)
                } else {
                    (c.max(open), c.min(open))
                };
                BarObs {
                    ts_ns: i64::from(i) * STEP,
                    knowledge_ns: i64::from(i) * STEP,
                    open,
                    high,
                    low,
                    close: c,
                    volume: 1.0,
                    quality: QualityFlags::NONE,
                }
            })
            .collect()
    }

    fn frame_of(obs: &[BarObs], horizon: u64) -> TrainingFrame {
        features::build_aligned_training_frame(obs, &["rsi_14".to_string()], horizon, STEP)
    }

    fn spec(path: Option<BarrierPath>, sampling: Option<SamplingSpec>) -> LabelSpec {
        LabelSpec {
            label_spec_id: String::new(),
            kind: dataplane::label::LabelKind::TripleBarrier,
            horizon_bars: 10,
            pt_sl_multiples: vec![2.0, 2.0],
            vol_estimator: None,
            min_return_threshold: None,
            sample_weight_method: SampleWeightMethod::Uniqueness,
            code_hash: "t".into(),
            barrier_path: path,
            sampling,
        }
    }

    #[test]
    fn a_spec_with_neither_option_returns_the_frame_untouched() {
        let obs = bars();
        let f = frame_of(&obs, 10);
        let out = apply(f.clone(), &obs, STEP, &spec(None, None)).unwrap();
        assert_eq!(out.label, f.label);
        assert_eq!(out.ts_ns, f.ts_ns);
    }

    #[test]
    fn cusum_keeps_only_event_rows_and_they_include_the_planted_jump() {
        let obs = bars();
        let f = frame_of(&obs, 10);
        let out = apply(
            f.clone(),
            &obs,
            STEP,
            &spec(None, Some(SamplingSpec::Cusum { h_sigma: 5.0 })),
        )
        .unwrap();
        assert!(out.row_count() > 0 && out.row_count() < f.row_count() / 4);
        assert!(
            out.ts_ns.contains(&(120 * STEP)),
            "the jump bar is an event"
        );
        assert_eq!(out.sample_weight.len(), out.row_count());
        assert!(out.columns.iter().all(|c| c.len() == out.row_count()));
        // rows are a subset, in order, with the original labels
        let orig: HashMap<i64, f64> = f
            .ts_ns
            .iter()
            .copied()
            .zip(f.label.iter().copied())
            .collect();
        assert!(out
            .ts_ns
            .iter()
            .zip(&out.label)
            .all(|(t, l)| orig[t].to_bits() == l.to_bits()));
    }

    #[test]
    fn the_high_low_path_sees_a_wick_the_close_path_does_not() {
        let obs = bars();
        let f = frame_of(&obs, 10);
        let hl = apply(
            f.clone(),
            &obs,
            STEP,
            &spec(Some(BarrierPath::HighLow), None),
        )
        .unwrap();
        let cl = apply(f, &obs, STEP, &spec(Some(BarrierPath::Close), None)).unwrap();
        let at = |fr: &TrainingFrame, bar: i64| {
            fr.ts_ns
                .iter()
                .position(|t| *t == bar * STEP)
                .map(|i| fr.label[i])
        };
        // Bar 195 looks forward across bar 200, whose range spans both barriers: the stop.
        assert_eq!(at(&hl, 195), Some(-1.0));
        assert_eq!(
            at(&cl, 195),
            Some(0.0),
            "the close path never sees the wick"
        );
    }

    #[test]
    fn rows_without_a_finished_barrier_are_dropped_not_labelled() {
        let obs = bars();
        let f = frame_of(&obs, 10);
        let hl = apply(f, &obs, STEP, &spec(Some(BarrierPath::HighLow), None)).unwrap();
        // warm-up (sigma window) rows and the last `max_bars` rows have no usable barrier
        assert!(hl.ts_ns.iter().all(|t| *t >= SIGMA_WINDOW as i64 * STEP));
        assert!(hl.ts_ns.iter().all(|t| *t <= (299 - 10) * STEP));
        assert!(hl.label.iter().all(|l| [-1.0, 0.0, 1.0].contains(l)));
    }

    #[test]
    fn strategy_events_and_a_missing_multiple_are_refused_by_name() {
        let obs = bars();
        let f = frame_of(&obs, 10);
        let s = spec(
            None,
            Some(SamplingSpec::StrategyEvents {
                strategy_version: "v".into(),
            }),
        );
        assert_eq!(
            apply(f.clone(), &obs, STEP, &s).unwrap_err(),
            LabelError::StrategyEventsNeedEntries
        );
        let mut b = spec(Some(BarrierPath::HighLow), None);
        b.pt_sl_multiples = vec![2.0];
        assert_eq!(
            apply(f, &obs, STEP, &b).unwrap_err(),
            LabelError::BarrierMultiples
        );
    }

    #[test]
    fn a_label_spec_without_the_new_fields_keeps_its_id() {
        let s = spec(None, None).content_keyed();
        let json = serde_json::to_value(&s).unwrap();
        assert!(json.get("barrier_path").is_none() && json.get("sampling").is_none());
        let with = spec(Some(BarrierPath::HighLow), None).content_keyed();
        assert_ne!(
            s.label_spec_id, with.label_spec_id,
            "the path is part of the hash"
        );
    }
}
