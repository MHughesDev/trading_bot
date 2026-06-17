//! The seven null-world generators (spec §2.1 catalog).
//!
//! Each generator preserves one structure and destroys another — the explicit
//! hypothesis rendered in every report. All are deterministic given a seed (the
//! `permutation_null`/`synthetic_paths` Studies vary that seed).

use serde::{Deserialize, Serialize};

use crate::rng::DetRng;

use super::{Null, NullKind};

/// A single OHLC bar (statistical, f64 — D-10; this is null-world data, not money).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Bar {
    pub ts_ns: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
}

impl Bar {
    /// True if OHLC are internally consistent (`low ≤ open,close ≤ high`).
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.low <= self.open
            && self.low <= self.close
            && self.high >= self.open
            && self.high >= self.close
            && self.low <= self.high
    }
}

/// The data a null operates on. Different kinds use different fields; a generator
/// touches only what its hypothesis concerns and copies the rest through.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct NullData {
    pub bars: Vec<Bar>,
    /// Signal value per bar (for `signal_return_decouple`).
    pub signals: Vec<f64>,
    /// Forward return per bar.
    pub forward_returns: Vec<f64>,
    /// Regime id per bar (for `regime_block`).
    pub regime_labels: Vec<u32>,
    /// Trade entry indices into the series (for `random_entry_matched`).
    pub entry_indices: Vec<usize>,
    /// Holding period (in bars) per trade.
    pub holding_periods: Vec<usize>,
}

/// Produces one null-world dataset from real data + a seed.
pub trait NullGenerator {
    fn kind(&self) -> NullKind;
    fn generate(&self, data: &NullData, seed: u64) -> NullData;
}

/// Resolve a [`Null`] to its generator.
#[must_use]
pub fn generator_for(null: &Null) -> Box<dyn NullGenerator> {
    match null.kind {
        NullKind::SignalReturnDecouple => Box::new(SignalReturnDecouple),
        NullKind::BlockPermutation => Box::new(BlockPermutation {
            block_length: null.params.block_length.unwrap_or(10).max(1),
        }),
        NullKind::StationaryBootstrap => Box::new(StationaryBootstrap {
            mean_block: null.params.mean_block.unwrap_or(10).max(1),
        }),
        NullKind::BarPermutation => Box::new(BarPermutation),
        NullKind::SyntheticGarch => Box::new(SyntheticGarch {
            alpha: null.params.garch_alpha.unwrap_or(0.10),
            beta: null.params.garch_beta.unwrap_or(0.85),
        }),
        NullKind::RegimeBlock => Box::new(RegimeBlock),
        NullKind::RandomEntryMatched => Box::new(RandomEntryMatched),
    }
}

/// Fisher–Yates shuffle of a vector's *indices*, returning the permutation.
fn permutation(n: usize, rng: &mut DetRng) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..n).collect();
    for i in (1..n).rev() {
        let j = rng.below(i + 1);
        idx.swap(i, j);
    }
    idx
}

// ── 1. signal_return_decouple ────────────────────────────────────────────────
/// Preserves both marginal distributions; destroys the signal→return pairing.
pub struct SignalReturnDecouple;
impl NullGenerator for SignalReturnDecouple {
    fn kind(&self) -> NullKind {
        NullKind::SignalReturnDecouple
    }
    fn generate(&self, data: &NullData, seed: u64) -> NullData {
        let mut rng = DetRng::new(seed);
        let perm = permutation(data.forward_returns.len(), &mut rng);
        let shuffled: Vec<f64> = perm.iter().map(|&i| data.forward_returns[i]).collect();
        NullData {
            forward_returns: shuffled,
            ..data.clone()
        }
    }
}

// ── 2. block_permutation ─────────────────────────────────────────────────────
/// Preserves within-block autocorrelation; destroys cross-block timing.
pub struct BlockPermutation {
    pub block_length: usize,
}
impl NullGenerator for BlockPermutation {
    fn kind(&self) -> NullKind {
        NullKind::BlockPermutation
    }
    fn generate(&self, data: &NullData, seed: u64) -> NullData {
        let mut rng = DetRng::new(seed);
        let r = &data.forward_returns;
        let blocks: Vec<&[f64]> = r.chunks(self.block_length.max(1)).collect();
        let perm = permutation(blocks.len(), &mut rng);
        let mut out = Vec::with_capacity(r.len());
        for &b in &perm {
            out.extend_from_slice(blocks[b]);
        }
        NullData {
            forward_returns: out,
            ..data.clone()
        }
    }
}

// ── 3. stationary_bootstrap ──────────────────────────────────────────────────
/// Preserves autocorrelation structure (random block lengths); destroys the
/// specific historical ordering.
pub struct StationaryBootstrap {
    pub mean_block: usize,
}
impl NullGenerator for StationaryBootstrap {
    fn kind(&self) -> NullKind {
        NullKind::StationaryBootstrap
    }
    fn generate(&self, data: &NullData, seed: u64) -> NullData {
        let mut rng = DetRng::new(seed);
        let r = &data.forward_returns;
        let n = r.len();
        if n == 0 {
            return data.clone();
        }
        // Geometric block length with mean `mean_block` → continuation prob p.
        let p = 1.0 / self.mean_block.max(1) as f64;
        let mut out = Vec::with_capacity(n);
        let mut i = rng.below(n);
        while out.len() < n {
            out.push(r[i]);
            if rng.next_f64() < p {
                i = rng.below(n); // start a new block
            } else {
                i = (i + 1) % n; // continue current block (wraparound)
            }
        }
        NullData {
            forward_returns: out,
            ..data.clone()
        }
    }
}

// ── 4. bar_permutation ───────────────────────────────────────────────────────
/// Preserves bar-level OHLC integrity; destroys inter-bar sequence.
pub struct BarPermutation;
impl NullGenerator for BarPermutation {
    fn kind(&self) -> NullKind {
        NullKind::BarPermutation
    }
    fn generate(&self, data: &NullData, seed: u64) -> NullData {
        let mut rng = DetRng::new(seed);
        let perm = permutation(data.bars.len(), &mut rng);
        let bars: Vec<Bar> = perm.iter().map(|&i| data.bars[i]).collect();
        NullData {
            bars,
            ..data.clone()
        }
    }
}

// ── 5. synthetic_garch ───────────────────────────────────────────────────────
/// Preserves volatility clustering + fat tails (GARCH(1,1)-t-ish); destroys the
/// specific realized path. A pure-Rust simulation (no Python sidecar — D-9).
pub struct SyntheticGarch {
    pub alpha: f64,
    pub beta: f64,
}
impl NullGenerator for SyntheticGarch {
    fn kind(&self) -> NullKind {
        NullKind::SyntheticGarch
    }
    fn generate(&self, data: &NullData, seed: u64) -> NullData {
        let r = &data.forward_returns;
        let n = r.len();
        if n == 0 {
            return data.clone();
        }
        let mean = r.iter().sum::<f64>() / n as f64;
        let uncond_var = (r.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / n as f64).max(1e-12);
        let alpha = self.alpha.clamp(0.0, 0.99);
        let beta = self.beta.clamp(0.0, 0.99 - alpha.min(0.98));
        let omega = uncond_var * (1.0 - alpha - beta).max(1e-6);

        let mut rng = DetRng::new(seed);
        let mut sigma2 = uncond_var;
        let mut prev_shock = 0.0;
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            sigma2 = omega + alpha * prev_shock * prev_shock + beta * sigma2;
            let z = standard_normal(&mut rng);
            let shock = sigma2.sqrt() * z;
            out.push(mean + shock);
            prev_shock = shock;
        }
        NullData {
            forward_returns: out,
            ..data.clone()
        }
    }
}

/// Standard normal via Box–Muller.
fn standard_normal(rng: &mut DetRng) -> f64 {
    let u1 = rng.next_f64().max(1e-12);
    let u2 = rng.next_f64();
    (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()
}

// ── 6. regime_block ──────────────────────────────────────────────────────────
/// Preserves within-regime structure; destroys cross-regime arrangement by
/// permuting the order of contiguous same-regime runs.
pub struct RegimeBlock;
impl NullGenerator for RegimeBlock {
    fn kind(&self) -> NullKind {
        NullKind::RegimeBlock
    }
    fn generate(&self, data: &NullData, seed: u64) -> NullData {
        let labels = &data.regime_labels;
        let r = &data.forward_returns;
        let n = r.len().min(labels.len());
        if n == 0 {
            return data.clone();
        }
        // Identify contiguous runs of identical regime label.
        let mut runs: Vec<(usize, usize)> = Vec::new(); // (start, len)
        let mut start = 0;
        for i in 1..n {
            if labels[i] != labels[start] {
                runs.push((start, i - start));
                start = i;
            }
        }
        runs.push((start, n - start));

        let mut rng = DetRng::new(seed);
        let perm = permutation(runs.len(), &mut rng);
        let mut out_r = Vec::with_capacity(n);
        let mut out_l = Vec::with_capacity(n);
        for &ri in &perm {
            let (s, len) = runs[ri];
            out_r.extend_from_slice(&r[s..s + len]);
            out_l.extend_from_slice(&labels[s..s + len]);
        }
        NullData {
            forward_returns: out_r,
            regime_labels: out_l,
            ..data.clone()
        }
    }
}

// ── 7. random_entry_matched ──────────────────────────────────────────────────
/// Preserves trade frequency, holding period, and exposure; destroys entry
/// timing skill by drawing random entries with resampled holding periods.
pub struct RandomEntryMatched;
impl NullGenerator for RandomEntryMatched {
    fn kind(&self) -> NullKind {
        NullKind::RandomEntryMatched
    }
    fn generate(&self, data: &NullData, seed: u64) -> NullData {
        let n_trades = data.entry_indices.len();
        let series_len = data.forward_returns.len().max(data.bars.len()).max(1);
        if n_trades == 0 {
            return data.clone();
        }
        let mut rng = DetRng::new(seed);
        let new_entries: Vec<usize> = (0..n_trades).map(|_| rng.below(series_len)).collect();
        // Resample holding periods (with replacement) from the original set —
        // preserves the holding-period distribution, destroys which entry had which.
        let holds = if data.holding_periods.is_empty() {
            vec![1usize; n_trades]
        } else {
            (0..n_trades)
                .map(|_| data.holding_periods[rng.below(data.holding_periods.len())])
                .collect()
        };
        NullData {
            entry_indices: new_entries,
            holding_periods: holds,
            ..data.clone()
        }
    }
}

// ── recommendation (J-3.7) ───────────────────────────────────────────────────

/// Recommend a null for a declared strategy type (spec §2.1 — surfaced as a
/// *prompt*, never an invisible default). The caller must still choose
/// explicitly; this only seeds the prompt.
#[must_use]
pub fn recommend_null(strategy_type: &str) -> NullKind {
    match strategy_type.to_ascii_lowercase().as_str() {
        s if s.contains("intraday")
            || s.contains("mean_reversion")
            || s.contains("mean-reversion") =>
        {
            NullKind::BlockPermutation
        }
        s if s.contains("trend") || s.contains("daily") => NullKind::StationaryBootstrap,
        s if s.contains("regime") => NullKind::RegimeBlock,
        s if s.contains("timing") || s.contains("entry") => NullKind::RandomEntryMatched,
        // General purpose.
        _ => NullKind::SignalReturnDecouple,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nulls::NullParams;

    fn ret_data(returns: Vec<f64>) -> NullData {
        let signals: Vec<f64> = returns.iter().enumerate().map(|(i, _)| i as f64).collect();
        NullData {
            forward_returns: returns,
            signals,
            ..Default::default()
        }
    }

    fn sorted(v: &[f64]) -> Vec<f64> {
        let mut s = v.to_vec();
        s.sort_by(|a, b| a.partial_cmp(b).unwrap());
        s
    }

    #[test]
    fn signal_return_decouple_preserves_marginal_returns() {
        let data = ret_data(vec![0.1, -0.2, 0.3, 0.0, 0.05, -0.1]);
        let null = Null::new(NullKind::SignalReturnDecouple, NullParams::default()).unwrap();
        let out = null.generate(&data, 42);
        assert_eq!(sorted(&out.forward_returns), sorted(&data.forward_returns));
        assert_eq!(out.signals, data.signals, "signals untouched");
    }

    #[test]
    fn block_permutation_preserves_block_contents() {
        let data = ret_data((0..20).map(f64::from).collect());
        let null = Null::new(
            NullKind::BlockPermutation,
            NullParams {
                block_length: Some(5),
                ..Default::default()
            },
        )
        .unwrap();
        let out = null.generate(&data, 7);
        // Same multiset overall.
        assert_eq!(sorted(&out.forward_returns), sorted(&data.forward_returns));
        // Each original block (contiguous run of 5) must survive intact somewhere.
        let orig_blocks: Vec<Vec<f64>> = data
            .forward_returns
            .chunks(5)
            .map(<[f64]>::to_vec)
            .collect();
        let out_blocks: Vec<Vec<f64>> =
            out.forward_returns.chunks(5).map(<[f64]>::to_vec).collect();
        for b in &orig_blocks {
            assert!(out_blocks.contains(b), "block {b:?} not preserved intact");
        }
    }

    #[test]
    fn stationary_bootstrap_draws_only_from_source_and_keeps_length() {
        let src = vec![0.1, -0.2, 0.3, 0.0, 0.05];
        let data = ret_data(src.clone());
        let null = Null::new(
            NullKind::StationaryBootstrap,
            NullParams {
                mean_block: Some(2),
                ..Default::default()
            },
        )
        .unwrap();
        let out = null.generate(&data, 9);
        assert_eq!(out.forward_returns.len(), src.len());
        for v in &out.forward_returns {
            assert!(
                src.iter().any(|s| (s - v).abs() < 1e-12),
                "value {v} not from source"
            );
        }
    }

    #[test]
    fn bar_permutation_keeps_ohlc_integrity_and_multiset() {
        let bars = vec![
            Bar {
                ts_ns: 1,
                open: 10.0,
                high: 12.0,
                low: 9.0,
                close: 11.0,
            },
            Bar {
                ts_ns: 2,
                open: 11.0,
                high: 13.0,
                low: 10.5,
                close: 12.5,
            },
            Bar {
                ts_ns: 3,
                open: 12.5,
                high: 12.6,
                low: 11.0,
                close: 11.2,
            },
        ];
        assert!(bars.iter().all(Bar::is_valid));
        let data = NullData {
            bars: bars.clone(),
            ..Default::default()
        };
        let null = Null::new(NullKind::BarPermutation, NullParams::default()).unwrap();
        let out = null.generate(&data, 3);
        assert!(
            out.bars.iter().all(Bar::is_valid),
            "OHLC integrity preserved"
        );
        assert_eq!(out.bars.len(), bars.len());
        for b in &bars {
            assert!(out.bars.contains(b));
        }
    }

    #[test]
    fn synthetic_garch_is_deterministic_and_shows_clustering() {
        // Build a clustered series: calm then volatile.
        let mut returns = vec![0.001; 100];
        returns.extend([0.05, -0.06, 0.055, -0.05, 0.06, -0.058].repeat(20));
        let data = ret_data(returns);
        let null = Null::new(NullKind::SyntheticGarch, NullParams::default()).unwrap();
        let a = null.generate(&data, 11);
        let b = null.generate(&data, 11);
        assert_eq!(
            a.forward_returns, b.forward_returns,
            "deterministic per seed"
        );
        let c = null.generate(&data, 12);
        assert_ne!(
            a.forward_returns, c.forward_returns,
            "distinct seeds differ"
        );
        // Vol clustering: lag-1 autocorrelation of squared returns is positive.
        assert!(
            acf1_squared(&a.forward_returns) > 0.0,
            "GARCH output should cluster volatility"
        );
    }

    #[test]
    fn regime_block_preserves_each_regimes_values() {
        let data = NullData {
            forward_returns: vec![1.0, 1.1, 1.2, 2.0, 2.1, 3.0, 3.1, 3.2],
            regime_labels: vec![0, 0, 0, 1, 1, 2, 2, 2],
            ..Default::default()
        };
        let null = Null::new(NullKind::RegimeBlock, NullParams::default()).unwrap();
        let out = null.generate(&data, 5);
        assert_eq!(out.forward_returns.len(), data.forward_returns.len());
        // Each label's value multiset is preserved.
        for label in [0u32, 1, 2] {
            let orig = values_for_label(&data, label);
            let now = values_for_label(&out, label);
            assert_eq!(
                sorted(&orig),
                sorted(&now),
                "regime {label} structure preserved"
            );
        }
    }

    #[test]
    fn random_entry_matched_preserves_count_and_holding_distribution() {
        let data = NullData {
            forward_returns: vec![0.0; 100],
            entry_indices: vec![3, 20, 55, 80],
            holding_periods: vec![5, 5, 10, 2],
            ..Default::default()
        };
        let null = Null::new(NullKind::RandomEntryMatched, NullParams::default()).unwrap();
        let out = null.generate(&data, 8);
        assert_eq!(
            out.entry_indices.len(),
            data.entry_indices.len(),
            "trade count preserved"
        );
        assert_eq!(out.holding_periods.len(), data.holding_periods.len());
        for h in &out.holding_periods {
            assert!(
                data.holding_periods.contains(h),
                "holding {h} drawn from source set"
            );
        }
        // Entries are within the series.
        assert!(out.entry_indices.iter().all(|&i| i < 100));
    }

    #[test]
    fn recommendation_matches_the_catalog() {
        assert_eq!(
            recommend_null("intraday_mean_reversion"),
            NullKind::BlockPermutation
        );
        assert_eq!(recommend_null("daily_trend"), NullKind::StationaryBootstrap);
        assert_eq!(recommend_null("regime_switch"), NullKind::RegimeBlock);
        assert_eq!(recommend_null("entry_timing"), NullKind::RandomEntryMatched);
        assert_eq!(recommend_null("whatever"), NullKind::SignalReturnDecouple);
    }

    fn values_for_label(d: &NullData, label: u32) -> Vec<f64> {
        d.forward_returns
            .iter()
            .zip(&d.regime_labels)
            .filter(|(_, &l)| l == label)
            .map(|(&r, _)| r)
            .collect()
    }

    fn acf1_squared(r: &[f64]) -> f64 {
        let sq: Vec<f64> = r.iter().map(|x| x * x).collect();
        let n = sq.len();
        if n < 2 {
            return 0.0;
        }
        let mean = sq.iter().sum::<f64>() / n as f64;
        let var = sq.iter().map(|x| (x - mean).powi(2)).sum::<f64>();
        if var <= 0.0 {
            return 0.0;
        }
        let cov: f64 = (1..n).map(|i| (sq[i] - mean) * (sq[i - 1] - mean)).sum();
        cov / var
    }
}
