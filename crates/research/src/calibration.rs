//! Fidelity calibration (P5): racing between two tiers is enabled only when the
//! cheap tier's ranking predicts the expensive tier's.

use serde::{Deserialize, Serialize};

/// Minimum Spearman ρ between adjacent tiers for successive halving to be
/// trusted (the HPO-benchmark threshold for "high rank correlation").
pub const RACING_MIN_RHO: f64 = 0.8;

fn ranks(v: &[f64]) -> Vec<f64> {
    let mut idx: Vec<usize> = (0..v.len()).collect();
    idx.sort_by(|&a, &b| v[a].partial_cmp(&v[b]).unwrap_or(std::cmp::Ordering::Equal));
    let mut r = vec![0.0; v.len()];
    let mut i = 0;
    while i < idx.len() {
        let mut j = i;
        while j + 1 < idx.len() && v[idx[j + 1]] == v[idx[i]] {
            j += 1;
        }
        let avg = (i + j) as f64 / 2.0 + 1.0;
        for &k in &idx[i..=j] {
            r[k] = avg;
        }
        i = j + 1;
    }
    r
}

/// Spearman rank correlation (ties averaged). `NaN` if fewer than 3 pairs.
#[must_use]
pub fn spearman(a: &[f64], b: &[f64]) -> f64 {
    let n = a.len().min(b.len());
    if n < 3 {
        return f64::NAN;
    }
    let (ra, rb) = (ranks(&a[..n]), ranks(&b[..n]));
    let ma = ra.iter().sum::<f64>() / n as f64;
    let mb = rb.iter().sum::<f64>() / n as f64;
    let cov: f64 = ra.iter().zip(&rb).map(|(x, y)| (x - ma) * (y - mb)).sum();
    let va: f64 = ra.iter().map(|x| (x - ma).powi(2)).sum();
    let vb: f64 = rb.iter().map(|y| (y - mb).powi(2)).sum();
    if va <= 0.0 || vb <= 0.0 {
        return 0.0;
    }
    cov / (va * vb).sqrt()
}

/// The measured relationship between two fidelity tiers for one
/// (instrument, strategy family).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FidelityCalibration {
    pub tier_from: String,
    pub tier_to: String,
    pub rho: f64,
    pub n: usize,
    /// `rho >= RACING_MIN_RHO` — racing between these tiers is trustworthy.
    pub racing_enabled: bool,
}

impl FidelityCalibration {
    #[must_use]
    pub fn from_scores(tier_from: &str, tier_to: &str, low: &[f64], high: &[f64]) -> Self {
        let rho = spearman(low, high);
        Self {
            tier_from: tier_from.to_string(),
            tier_to: tier_to.to_string(),
            rho,
            n: low.len().min(high.len()),
            racing_enabled: rho.is_finite() && rho >= RACING_MIN_RHO,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn perfect_and_reversed_rankings() {
        let a = [1.0, 2.0, 3.0, 4.0, 5.0];
        let b = [10.0, 20.0, 30.0, 40.0, 50.0];
        assert!((spearman(&a, &b) - 1.0).abs() < 1e-12);
        let c = [5.0, 4.0, 3.0, 2.0, 1.0];
        assert!((spearman(&a, &c) + 1.0).abs() < 1e-12);
    }

    #[test]
    fn ties_and_short_inputs() {
        assert!(spearman(&[1.0, 2.0], &[1.0, 2.0]).is_nan());
        let a = [1.0, 1.0, 2.0, 3.0];
        let b = [1.0, 1.0, 2.0, 3.0];
        assert!((spearman(&a, &b) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn racing_gate_uses_threshold() {
        let low = [0.1, 0.2, 0.3, 0.4, 0.5, 0.6];
        let high = [0.1, 0.25, 0.2, 0.45, 0.5, 0.7];
        let c = FidelityCalibration::from_scores("1h/90d", "15m/1y", &low, &high);
        assert!(c.racing_enabled);
        let noise = [0.6, 0.1, 0.5, 0.2, 0.4, 0.3];
        let d = FidelityCalibration::from_scores("1h/90d", "15m/1y", &low, &noise);
        assert!(!d.racing_enabled);
    }
}
