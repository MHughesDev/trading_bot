//! Label specifications (SPEC §3.4).

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LabelKind {
    TripleBarrier,
    HorizonReturn,
    MetaLabel,
    Custom,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SampleWeightMethod {
    Uniqueness,
    ReturnAttribution,
    TimeDecay,
    None,
}

/// `sample_weight_method` has no serde default: a spec that omits it fails to parse.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LabelSpec {
    pub label_spec_id: String,
    pub kind: LabelKind,
    pub horizon_bars: u32,
    #[serde(default)]
    pub pt_sl_multiples: Vec<f64>,
    #[serde(default)]
    pub vol_estimator: Option<String>,
    #[serde(default)]
    pub min_return_threshold: Option<f64>,
    pub sample_weight_method: SampleWeightMethod,
    pub code_hash: String,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum LabelError {
    #[error("horizon_bars must be ≥ 1")]
    ZeroHorizon,
    #[error("triple_barrier requires two pt/sl multiples")]
    BarrierMultiples,
}

impl LabelSpec {
    ///
    /// # Errors
    /// Structural violations.
    pub fn validate(&self) -> Result<(), LabelError> {
        if self.horizon_bars == 0 {
            return Err(LabelError::ZeroHorizon);
        }
        if self.kind == LabelKind::TripleBarrier && self.pt_sl_multiples.len() != 2 {
            return Err(LabelError::BarrierMultiples);
        }
        Ok(())
    }

    /// Re-key the spec by its own content, so two callers that ask for the same
    /// labelling get the same `label_spec_id` and a changed field is a new spec
    /// rather than a silent redefinition of the old one.
    ///
    /// # Panics
    /// Never in practice: every field is JSON-representable.
    #[must_use]
    pub fn content_keyed(mut self) -> Self {
        self.label_spec_id = String::new();
        let id = crate::hash::content_hash(&self).expect("label spec serializes");
        self.label_spec_id = id;
        self
    }

    /// Overlapping labels with no weighting: permitted, but every trial using this
    /// spec carries `overlapping_labels_unweighted = true`, surfaced in head-to-heads.
    #[must_use]
    pub fn overlapping_labels_unweighted(&self) -> bool {
        self.horizon_bars > 1 && self.sample_weight_method == SampleWeightMethod::None
    }
}

/// Label interval `[t0, t1]` in bar indices.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LabelInterval {
    pub t0: usize,
    pub t1: usize,
}

#[must_use]
pub fn horizon_intervals(n: usize, horizon: u32) -> Vec<LabelInterval> {
    (0..n).map(|i| LabelInterval { t0: i, t1: (i + horizon as usize).min(n.saturating_sub(1)) }).collect()
}

/// Average-uniqueness sample weights: each label's mean of 1/concurrency over its span.
#[must_use]
pub fn uniqueness_weights(intervals: &[LabelInterval], n_bars: usize) -> Vec<f64> {
    let mut conc = vec![0u32; n_bars];
    for iv in intervals {
        for c in conc.iter_mut().take(iv.t1.min(n_bars - 1) + 1).skip(iv.t0) {
            *c += 1;
        }
    }
    intervals
        .iter()
        .map(|iv| {
            let span = iv.t0..=iv.t1.min(n_bars - 1);
            let len = span.clone().count() as f64;
            span.map(|b| 1.0 / f64::from(conc[b].max(1))).sum::<f64>() / len
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_weight_method_does_not_parse() {
        let bad = serde_json::json!({"label_spec_id":"l","kind":"horizon_return","horizon_bars":10,"code_hash":"x"});
        assert!(serde_json::from_value::<LabelSpec>(bad).is_err());
    }

    #[test]
    fn unweighted_overlap_is_flagged() {
        let s = LabelSpec {
            label_spec_id: "l".into(),
            kind: LabelKind::HorizonReturn,
            horizon_bars: 10,
            pt_sl_multiples: vec![],
            vol_estimator: None,
            min_return_threshold: None,
            sample_weight_method: SampleWeightMethod::None,
            code_hash: "x".into(),
        };
        assert!(s.overlapping_labels_unweighted());
    }

    #[test]
    fn uniqueness_downweights_overlap() {
        let iv = horizon_intervals(20, 4);
        let w = uniqueness_weights(&iv, 20);
        assert!(w[10] < 1.0);
        assert!(w.iter().all(|x| *x > 0.0 && *x <= 1.0));
    }
}
