//! Samplers over the unit cube: seeded random and a compact TPE.
//!
//! A sampler decides **where to look next**. It sees per-sample scores — that
//! is exploration and is permitted (FEAT-003 §7.3) — but its state is never
//! exposed through a tool, and nothing it ranks is ever carried forward.

use serde::{Deserialize, Serialize};

use crate::space::SearchSpace;

/// Which sampler a sweep uses.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SamplerKind {
    Random,
    #[default]
    Tpe,
}

/// xorshift64* — small, fast, reproducible. Not for anything cryptographic.
#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    #[must_use]
    pub fn new(seed: u64) -> Self {
        // Avoid the all-zero state.
        Self(seed ^ 0x9E37_79B9_7F4A_7C15 | 1)
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform in `[0, 1)`.
    pub fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Standard normal (Box–Muller).
    pub fn next_normal(&mut self) -> f64 {
        let u1 = self.next_f64().max(1e-12);
        let u2 = self.next_f64();
        (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()
    }

    pub fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next_f64() * n as f64) as usize % n
        }
    }
}

pub trait Sampler: Send {
    /// Propose `n` unit-cube points.
    fn propose(&mut self, space: &SearchSpace, n: usize) -> Vec<Vec<f64>>;
    /// Record an evaluated point. `-inf` marks an infeasible/failed sample.
    fn observe(&mut self, x: &[f64], score: f64);
    /// Points observed so far.
    fn n_observed(&self) -> usize;
}

/// Uniform random search — the honest baseline, and what TPE warms up with.
pub struct RandomSampler {
    rng: Rng,
    n: usize,
}

impl RandomSampler {
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self {
            rng: Rng::new(seed),
            n: 0,
        }
    }
}

impl Sampler for RandomSampler {
    fn propose(&mut self, space: &SearchSpace, n: usize) -> Vec<Vec<f64>> {
        (0..n)
            .map(|_| (0..space.n_dims()).map(|_| self.rng.next_f64()).collect())
            .collect()
    }
    fn observe(&mut self, _x: &[f64], _score: f64) {
        self.n += 1;
    }
    fn n_observed(&self) -> usize {
        self.n
    }
}

/// Tree-structured Parzen Estimator over the unit cube.
///
/// After `n_startup` random points, observations are split into the top
/// `gamma` fraction ("good") and the rest ("bad"). Candidates are drawn from a
/// Gaussian KDE over the good set and the one maximising `l(x)/g(x)` is
/// proposed. Batches take the top-`n` distinct candidates.
pub struct TpeSampler {
    rng: Rng,
    n_startup: usize,
    gamma: f64,
    n_candidates: usize,
    obs: Vec<(Vec<f64>, f64)>,
}

impl TpeSampler {
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self {
            rng: Rng::new(seed),
            n_startup: 10,
            gamma: 0.25,
            n_candidates: 32,
            obs: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_startup(mut self, n: usize) -> Self {
        self.n_startup = n.max(2);
        self
    }

    /// Scott's-rule-ish bandwidth in the unit cube, clamped so the KDE neither
    /// collapses onto observed points nor blurs into uniform.
    fn bandwidth(points: &[&Vec<f64>], dim: usize) -> f64 {
        let n = points.len().max(1) as f64;
        let mean = points.iter().map(|p| p[dim]).sum::<f64>() / n;
        let var = points.iter().map(|p| (p[dim] - mean).powi(2)).sum::<f64>() / n;
        (1.06 * var.sqrt() * n.powf(-0.2)).clamp(0.05, 0.4)
    }

    fn kde(points: &[&Vec<f64>], bw: &[f64], x: &[f64]) -> f64 {
        if points.is_empty() {
            return 1e-9;
        }
        let mut total = 0.0;
        for p in points {
            let mut e = 0.0;
            for (d, (&xi, &pi)) in x.iter().zip(p.iter()).enumerate() {
                let z = (xi - pi) / bw[d];
                e += z * z;
            }
            total += (-0.5 * e).exp();
        }
        (total / points.len() as f64).max(1e-12)
    }
}

impl Sampler for TpeSampler {
    fn propose(&mut self, space: &SearchSpace, n: usize) -> Vec<Vec<f64>> {
        let d = space.n_dims();
        let finite: Vec<&(Vec<f64>, f64)> = self.obs.iter().filter(|(_, s)| s.is_finite()).collect();
        if finite.len() < self.n_startup {
            return (0..n)
                .map(|_| (0..d).map(|_| self.rng.next_f64()).collect())
                .collect();
        }
        // Split: good = top gamma fraction of finite scores; bad = everything
        // else, including infeasible (-inf) points so we learn to avoid them.
        let mut sorted: Vec<&(Vec<f64>, f64)> = finite.clone();
        sorted.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        let n_good = ((self.gamma * sorted.len() as f64).ceil() as usize).clamp(2, sorted.len());
        let good: Vec<&Vec<f64>> = sorted[..n_good].iter().map(|(x, _)| x).collect();
        let bad: Vec<&Vec<f64>> = sorted[n_good..]
            .iter()
            .map(|(x, _)| x)
            .chain(self.obs.iter().filter(|(_, s)| !s.is_finite()).map(|(x, _)| x))
            .collect();
        let bw_good: Vec<f64> = (0..d).map(|i| Self::bandwidth(&good, i)).collect();
        let bw_bad: Vec<f64> = (0..d).map(|i| Self::bandwidth(&bad, i)).collect();

        let n_cand = self.n_candidates.max(n * 8);
        let mut candidates: Vec<(Vec<f64>, f64)> = Vec::with_capacity(n_cand);
        for _ in 0..n_cand {
            let anchor = good[self.rng.below(good.len())];
            let x: Vec<f64> = (0..d)
                .map(|i| (anchor[i] + self.rng.next_normal() * bw_good[i]).clamp(0.0, 1.0))
                .collect();
            let l = Self::kde(&good, &bw_good, &x);
            let g = Self::kde(&bad, &bw_bad, &x);
            candidates.push((x, l / g));
        }
        candidates.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        // Greedy diverse pick: a batch of near-duplicates wastes Runs, so skip
        // candidates closer than the good-set bandwidth to an already-chosen one.
        let mut chosen: Vec<Vec<f64>> = Vec::with_capacity(n);
        for (x, _) in &candidates {
            let too_close = chosen.iter().any(|c| {
                c.iter()
                    .zip(x)
                    .zip(&bw_good)
                    .all(|((a, b), bw)| (a - b).abs() < *bw)
            });
            if !too_close {
                chosen.push(x.clone());
                if chosen.len() == n {
                    break;
                }
            }
        }
        // Fill any remainder from the top of the list regardless of distance.
        for (x, _) in candidates {
            if chosen.len() == n {
                break;
            }
            if !chosen.contains(&x) {
                chosen.push(x);
            }
        }
        chosen
    }

    fn observe(&mut self, x: &[f64], score: f64) {
        self.obs.push((x.to_vec(), score));
    }

    fn n_observed(&self) -> usize {
        self.obs.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::space::SearchSpace;
    use serde_json::json;
    use std::collections::BTreeMap;

    fn space() -> SearchSpace {
        let def = serde_json::from_value(json!({
            "strategy_id": "s", "definition_version": "1.0", "asset_class": "x",
            "parameters": {
                "a": { "type": "float", "default": 0.5, "min": 0.0, "max": 1.0 },
                "b": { "type": "float", "default": 0.5, "min": 0.0, "max": 1.0 }
            },
            "inputs": [], "nodes": [], "actions": []
        }))
        .unwrap();
        SearchSpace::from_definition(&def, &BTreeMap::new()).unwrap()
    }

    /// Smooth quadratic with a known optimum at (0.7, 0.3).
    fn score(x: &[f64]) -> f64 {
        -((x[0] - 0.7).powi(2) + (x[1] - 0.3).powi(2))
    }

    fn best_after(sampler: &mut dyn Sampler, rounds: usize) -> f64 {
        let sp = space();
        let mut best = f64::NEG_INFINITY;
        for _ in 0..rounds {
            for x in sampler.propose(&sp, 4) {
                let s = score(&x);
                best = best.max(s);
                sampler.observe(&x, s);
            }
        }
        best
    }

    #[test]
    fn rng_is_deterministic() {
        let (mut a, mut b) = (Rng::new(42), Rng::new(42));
        assert_eq!(a.next_u64(), b.next_u64());
        let u = a.next_f64();
        assert!((0.0..1.0).contains(&u));
    }

    #[test]
    fn tpe_beats_random_on_a_smooth_objective() {
        let (mut worse, mut sum_r, mut sum_t) = (0, 0.0, 0.0);
        for seed in 1..=8u64 {
            let r = best_after(&mut RandomSampler::new(seed), 20);
            let t = best_after(&mut TpeSampler::new(seed), 20);
            sum_r += r;
            sum_t += t;
            if t < r {
                worse += 1;
            }
        }
        assert!(
            sum_t >= sum_r && worse <= 2,
            "TPE mean best {:.4} vs random {:.4}; lost on {worse}/8 seeds",
            sum_t / 8.0,
            sum_r / 8.0
        );
    }

    #[test]
    fn tpe_handles_infeasible_observations() {
        let sp = space();
        let mut t = TpeSampler::new(7).with_startup(4);
        for x in t.propose(&sp, 6) {
            t.observe(&x, f64::NEG_INFINITY);
        }
        for x in t.propose(&sp, 6) {
            t.observe(&x, score(&x));
        }
        let next = t.propose(&sp, 3);
        assert_eq!(next.len(), 3);
        assert!(next.iter().all(|x| x.iter().all(|v| (0.0..=1.0).contains(v))));
    }
}
