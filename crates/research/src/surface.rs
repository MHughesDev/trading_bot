//! [`SurfaceSummary`] — what the agent reads instead of an argmax (§7.4).
//!
//! Binned marginals per parameter from the sampled points: where the plateau
//! is, where the cliffs are, how sensitive the objective is. Computed from the
//! sweep's own samples; no LLM, no simulator.

use backtest::run::ParamMap;
use serde::{Deserialize, Serialize};

use crate::space::{DimKind, SearchSpace};

/// Bins per numeric dimension.
const N_BINS: usize = 8;
/// Text budget for the LLM-facing rendering.
const MAX_TEXT_BYTES: usize = 1_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Sensitivity {
    Low,
    Medium,
    High,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Bin {
    /// Human-readable bin label (a value for enums, a lower edge for numerics).
    pub label: String,
    pub mean: f64,
    pub n: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ParamSurface {
    pub name: String,
    pub bins: Vec<Bin>,
    /// Inclusive `[lo, hi]` labels of the widest contiguous high-scoring run
    /// containing the best bin, when at least two bins qualify.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plateau: Option<(String, String)>,
    /// Bin edges across which the mean drops sharply.
    pub cliffs: Vec<String>,
    pub sensitivity: Sensitivity,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SurfaceSummary {
    pub n_samples: usize,
    pub n_finite: usize,
    pub per_param: Vec<ParamSurface>,
    /// ≤ 1 KB rendering for the model.
    pub text: String,
}

fn std_dev(v: &[f64]) -> f64 {
    if v.len() < 2 {
        return 0.0;
    }
    let m = v.iter().sum::<f64>() / v.len() as f64;
    (v.iter().map(|x| (x - m).powi(2)).sum::<f64>() / (v.len() - 1) as f64).sqrt()
}

impl SurfaceSummary {
    /// Build from `(point, score)` samples; non-finite scores count toward
    /// `n_samples` but not the marginals.
    #[must_use]
    pub fn build(space: &SearchSpace, samples: &[(ParamMap, f64)]) -> Self {
        let finite: Vec<(Vec<f64>, f64)> = samples
            .iter()
            .filter(|(_, s)| s.is_finite())
            .map(|(p, s)| (space.encode(p), *s))
            .collect();
        let global_sd = std_dev(&finite.iter().map(|(_, s)| *s).collect::<Vec<_>>());
        let mut per_param = Vec::with_capacity(space.n_dims());

        for (i, dim) in space.dims.iter().enumerate() {
            let n_bins = match &dim.kind {
                DimKind::Enum { choices } => choices.len().max(1),
                DimKind::Int { min, max, .. } => ((max - min + 1) as usize).clamp(1, N_BINS),
                DimKind::Float { .. } => N_BINS,
            };
            let mut sum = vec![0.0; n_bins];
            let mut cnt = vec![0usize; n_bins];
            for (x, s) in &finite {
                let b = ((x[i] * n_bins as f64) as usize).min(n_bins - 1);
                sum[b] += s;
                cnt[b] += 1;
            }
            let label = |b: usize| -> String {
                let u = if n_bins <= 1 {
                    0.0
                } else {
                    b as f64 / (n_bins - 1) as f64
                };
                dim.render(u)
            };
            let bins: Vec<Bin> = (0..n_bins)
                .map(|b| Bin {
                    label: label(b),
                    mean: if cnt[b] > 0 {
                        sum[b] / cnt[b] as f64
                    } else {
                        f64::NAN
                    },
                    n: cnt[b],
                })
                .collect();

            let means: Vec<(usize, f64)> = bins
                .iter()
                .enumerate()
                .filter(|(_, b)| b.n > 0)
                .map(|(i, b)| (i, b.mean))
                .collect();
            let (plateau, cliffs, sensitivity) = if means.len() < 2 {
                (None, Vec::new(), Sensitivity::Low)
            } else {
                let top = means
                    .iter()
                    .cloned()
                    .fold(
                        (0usize, f64::NEG_INFINITY),
                        |a, b| if b.1 > a.1 { b } else { a },
                    );
                let lo = means.iter().map(|m| m.1).fold(f64::INFINITY, f64::min);
                let range = top.1 - lo;
                let threshold = top.1 - 0.2 * range;
                // Widest contiguous run of occupied bins ≥ threshold containing the top bin.
                let occupied: Vec<usize> = means.iter().map(|m| m.0).collect();
                let pos = occupied.iter().position(|&b| b == top.0).unwrap_or(0);
                let ok = |k: usize| means[k].1 >= threshold;
                let (mut a, mut z) = (pos, pos);
                while a > 0 && ok(a - 1) {
                    a -= 1;
                }
                while z + 1 < means.len() && ok(z + 1) {
                    z += 1;
                }
                let plateau = if z > a {
                    Some((
                        bins[means[a].0].label.clone(),
                        bins[means[z].0].label.clone(),
                    ))
                } else {
                    None
                };
                let cliffs: Vec<String> = means
                    .windows(2)
                    .filter(|w| range > 0.0 && (w[0].1 - w[1].1).abs() > 0.5 * range)
                    .map(|w| bins[w[1].0].label.clone())
                    .collect();
                let sens = if global_sd <= 0.0 {
                    Sensitivity::Low
                } else {
                    match range / global_sd {
                        r if r < 0.5 => Sensitivity::Low,
                        r if r < 1.5 => Sensitivity::Medium,
                        _ => Sensitivity::High,
                    }
                };
                (plateau, cliffs, sens)
            };
            per_param.push(ParamSurface {
                name: dim.name.clone(),
                bins,
                plateau,
                cliffs,
                sensitivity,
            });
        }

        let mut s = Self {
            n_samples: samples.len(),
            n_finite: finite.len(),
            per_param,
            text: String::new(),
        };
        s.text = s.render();
        s
    }

    fn render(&self) -> String {
        let mut out = format!(
            "surface from {} samples ({} feasible):\n",
            self.n_samples, self.n_finite
        );
        for p in &self.per_param {
            let mut line = format!("- {} [{:?}]", p.name, p.sensitivity);
            match &p.plateau {
                Some((lo, hi)) => line.push_str(&format!(": plateau {lo}–{hi}")),
                None => {
                    if let Some(best) = p.bins.iter().filter(|b| b.n > 0).max_by(|a, b| {
                        a.mean
                            .partial_cmp(&b.mean)
                            .unwrap_or(std::cmp::Ordering::Equal)
                    }) {
                        line.push_str(&format!(
                            ": single best bin {} (spike — treat as fragile)",
                            best.label
                        ));
                    }
                }
            }
            if !p.cliffs.is_empty() {
                line.push_str(&format!("; cliff at {}", p.cliffs.join(", ")));
            }
            out.push_str(&line);
            out.push('\n');
        }
        if out.len() > MAX_TEXT_BYTES {
            let mut end = MAX_TEXT_BYTES;
            while !out.is_char_boundary(end) {
                end -= 1;
            }
            out.truncate(end);
            out.push('…');
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::BTreeMap;

    fn space() -> SearchSpace {
        let def = serde_json::from_value(json!({
            "strategy_id": "s", "definition_version": "1.0", "asset_class": "x",
            "parameters": {
                "fast": { "type": "int", "default": 20, "min": 5, "max": 45 },
                "exit": { "type": "enum", "default": "a", "choices": ["a", "b"] }
            },
            "inputs": [], "nodes": [], "actions": []
        }))
        .unwrap();
        SearchSpace::from_definition(&def, &BTreeMap::new()).unwrap()
    }

    #[test]
    fn plateau_and_cliff_are_detected() {
        let sp = space();
        // Score: flat 1.0 for fast in [10, 30], falls to 0 above 35.
        let mut samples = Vec::new();
        for fast in (5..=45).step_by(2) {
            for exit in ["a", "b"] {
                let s = if (10..=30).contains(&fast) {
                    1.0
                } else if fast > 35 {
                    -1.0
                } else {
                    0.6
                };
                let mut p = ParamMap::new();
                p.insert("fast".into(), json!(fast));
                p.insert("exit".into(), json!(exit));
                samples.push((p, s));
            }
        }
        let sum = SurfaceSummary::build(&sp, &samples);
        // Dimensions follow the declaration's BTreeMap order — select by name.
        let by_name = |n: &str| sum.per_param.iter().find(|p| p.name == n).unwrap();
        let fast = by_name("fast");
        assert!(fast.plateau.is_some(), "{fast:?}");
        assert!(!fast.cliffs.is_empty(), "bins: {:?}", fast.bins);
        assert_eq!(fast.sensitivity, Sensitivity::High);
        let exit = by_name("exit");
        assert_eq!(exit.sensitivity, Sensitivity::Low);
        assert!(sum.text.contains("plateau"));
        assert!(sum.text.len() <= 1_000);
    }

    #[test]
    fn handles_no_finite_samples() {
        let sp = space();
        let sum = SurfaceSummary::build(&sp, &[(sp.default_point(), f64::NEG_INFINITY)]);
        assert_eq!(sum.n_finite, 0);
        assert!(sum.per_param.iter().all(|p| p.plateau.is_none()));
    }
}
