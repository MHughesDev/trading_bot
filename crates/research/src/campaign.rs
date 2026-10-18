//! The campaign's outer loop: islands, a feature-map archive, de-duplication and bandit
//! allocation (Set R-b RB-5.7; FEAT-003 §15 Phase 2).
//!
//! A campaign that keeps only its best candidate converges on one idea. The archive
//! keeps the best candidate **per behavioural cell**, so a campaign that has found
//! three kinds of strategy holds three, and the next generation can start from any.
//! Islands run independent archives and exchange their best members only every few
//! generations, which keeps them diverse. A bandit decides which island, or which move
//! family, gets the next share of a fixed budget.
//!
//! Everything here is pure and seeded; the campaign driver (`crates/api`) owns the
//! calls into it. `crates/api/src/campaign_driver.rs` is not edited by this set, so the
//! hand-off is the exact calls listed in the Set R-b report.
//!
//! # De-duplication
//!
//! A candidate whose `config_hash` the campaign has already evaluated is not
//! evaluated again: [`Archive::offer`] returns [`Offer::Duplicate`] before anything is
//! compared, so a duplicate can neither fill a cell nor spend a trial.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::sampler::Rng;

/// How behaviour is binned: one `(lo, hi, bins)` per descriptor dimension.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GridSpec {
    pub dims: Vec<(f64, f64, usize)>,
}

impl GridSpec {
    /// The cell of a descriptor vector. Values outside `[lo, hi]` clamp to the edge
    /// bin, and a non-finite value is `None`: a candidate with no behaviour has no cell.
    #[must_use]
    pub fn cell(&self, descriptor: &[f64]) -> Option<Vec<usize>> {
        if descriptor.len() != self.dims.len() {
            return None;
        }
        descriptor
            .iter()
            .zip(&self.dims)
            .map(|(v, (lo, hi, n))| {
                if !v.is_finite() || *n == 0 || hi <= lo {
                    return None;
                }
                let t = ((v - lo) / (hi - lo)).clamp(0.0, 1.0 - f64::EPSILON);
                Some((t * *n as f64) as usize)
            })
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Candidate {
    pub id: String,
    pub config_hash: String,
    pub score: f64,
    pub descriptor: Vec<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Offer {
    /// The campaign has already evaluated this configuration.
    Duplicate,
    /// No finite score or no cell.
    Unplaceable,
    /// A cell nobody held.
    NewCell(Vec<usize>),
    /// Beat the cell's elite.
    Replaced(Vec<usize>),
    /// Placed but did not beat the cell's elite.
    Rejected(Vec<usize>),
}

/// One island's archive: the elite of each cell.
#[derive(Clone, Debug)]
pub struct Archive {
    grid: GridSpec,
    elites: BTreeMap<Vec<usize>, Candidate>,
    seen: BTreeSet<String>,
}

impl Archive {
    #[must_use]
    pub fn new(grid: GridSpec) -> Self {
        Self {
            grid,
            elites: BTreeMap::new(),
            seen: BTreeSet::new(),
        }
    }

    pub fn offer(&mut self, c: Candidate) -> Offer {
        if !self.seen.insert(c.config_hash.clone()) {
            return Offer::Duplicate;
        }
        let Some(cell) = self.grid.cell(&c.descriptor).filter(|_| c.score.is_finite()) else {
            return Offer::Unplaceable;
        };
        match self.elites.get(&cell) {
            None => {
                self.elites.insert(cell.clone(), c);
                Offer::NewCell(cell)
            }
            Some(cur) if c.score > cur.score => {
                self.elites.insert(cell.clone(), c);
                Offer::Replaced(cell)
            }
            Some(_) => Offer::Rejected(cell),
        }
    }

    #[must_use]
    pub fn occupied(&self) -> usize {
        self.elites.len()
    }

    #[must_use]
    pub fn elites(&self) -> Vec<&Candidate> {
        self.elites.values().collect()
    }

    #[must_use]
    pub fn best(&self) -> Option<&Candidate> {
        self.elites.values().max_by(|a, b| a.score.total_cmp(&b.score))
    }

    #[must_use]
    pub fn evaluated(&self) -> usize {
        self.seen.len()
    }
}

/// Islands: independent archives that exchange their best member on a ring every
/// `period` generations.
#[derive(Clone, Debug)]
pub struct Islands {
    pub archives: Vec<Archive>,
    pub period: u32,
    generation: u32,
}

impl Islands {
    #[must_use]
    pub fn new(n: usize, grid: &GridSpec, period: u32) -> Self {
        Self {
            archives: (0..n.max(1)).map(|_| Archive::new(grid.clone())).collect(),
            period: period.max(1),
            generation: 0,
        }
    }

    /// End a generation. Returns the migrants moved, `(from, to, id)`, when it was a
    /// migration generation.
    pub fn end_generation(&mut self) -> Vec<(usize, usize, String)> {
        self.generation += 1;
        if self.generation % self.period != 0 || self.archives.len() < 2 {
            return Vec::new();
        }
        let n = self.archives.len();
        let best: Vec<Option<Candidate>> = self.archives.iter().map(|a| a.best().cloned()).collect();
        let mut moved = Vec::new();
        for (from, b) in best.into_iter().enumerate() {
            if let Some(c) = b {
                let to = (from + 1) % n;
                // A migrant is offered like any candidate: it can fill a cell or be
                // refused as a duplicate, and either way nothing is evaluated twice.
                if matches!(self.archives[to].offer(c.clone()), Offer::NewCell(_) | Offer::Replaced(_)) {
                    moved.push((from, to, c.id));
                }
            }
        }
        moved
    }
}

/// A bandit arm: an island or a move family.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ArmStat {
    pub pulls: u32,
    pub reward_sum: f64,
}

/// Split `budget` trials across arms by UCB1, one trial at a time, updating a local
/// copy with `expected_reward` as each is "pulled" is the caller's job; this returns
/// the arm to pull next. Untried arms first (in index order), then the highest
/// `mean + c * sqrt(2 ln N / n)`; ties break through the seeded `rng`, never by
/// position, so no arm is systematically favoured.
#[must_use]
pub fn next_arm(arms: &[ArmStat], c: f64, rng: &mut Rng) -> Option<usize> {
    if arms.is_empty() {
        return None;
    }
    if let Some(i) = arms.iter().position(|a| a.pulls == 0) {
        return Some(i);
    }
    let total: f64 = arms.iter().map(|a| f64::from(a.pulls)).sum();
    let scores: Vec<f64> = arms
        .iter()
        .map(|a| a.reward_sum / f64::from(a.pulls) + c * (2.0 * total.ln() / f64::from(a.pulls)).sqrt())
        .collect();
    let best = scores.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let tied: Vec<usize> = (0..arms.len())
        .filter(|&i| (scores[i] - best).abs() < 1e-12)
        .collect();
    Some(tied[rng.below(tied.len())])
}

/// The campaign's probability of backtest overfitting over every candidate it evaluated
/// (`performance[config][period]`). Reported as `campaign_pbo`.
#[must_use]
pub fn campaign_pbo(performance: &[Vec<f64>], n_groups: usize) -> f64 {
    backtest::stats::probability_of_backtest_overfitting(performance, n_groups)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid() -> GridSpec {
        GridSpec { dims: vec![(0.0, 1.0, 4), (0.0, 1.0, 4)] }
    }

    fn cand(id: &str, hash: &str, score: f64, d: [f64; 2]) -> Candidate {
        Candidate { id: id.into(), config_hash: hash.into(), score, descriptor: d.to_vec() }
    }

    #[test]
    fn the_archive_keeps_one_elite_per_cell_and_a_duplicate_never_competes() {
        let mut a = Archive::new(grid());
        assert!(matches!(a.offer(cand("a", "h1", 1.0, [0.1, 0.1])), Offer::NewCell(_)));
        assert!(matches!(a.offer(cand("b", "h2", 0.5, [0.1, 0.1])), Offer::Rejected(_)));
        assert!(matches!(a.offer(cand("c", "h3", 2.0, [0.1, 0.1])), Offer::Replaced(_)));
        assert_eq!(a.offer(cand("d", "h3", 9.0, [0.9, 0.9])), Offer::Duplicate, "same config hash, even with a better score");
        assert_eq!(a.occupied(), 1);
        assert_eq!(a.best().unwrap().id, "c");
        assert_eq!(a.evaluated(), 3);
        assert_eq!(a.offer(cand("e", "h4", f64::NAN, [0.5, 0.5])), Offer::Unplaceable);
        assert_eq!(a.offer(cand("f", "h5", 1.0, [f64::NAN, 0.5])), Offer::Unplaceable);
    }

    #[test]
    fn values_outside_the_range_clamp_to_the_edge_bin() {
        let g = grid();
        assert_eq!(g.cell(&[-5.0, 99.0]), Some(vec![0, 3]));
        assert_eq!(g.cell(&[1.0, 1.0]), Some(vec![3, 3]));
        assert_eq!(g.cell(&[0.5]), None);
    }

    #[test]
    fn islands_migrate_only_every_period_and_a_migrant_is_still_deduplicated() {
        let mut isl = Islands::new(3, &grid(), 2);
        isl.archives[0].offer(cand("a", "ha", 1.0, [0.1, 0.1]));
        isl.archives[1].offer(cand("b", "hb", 1.0, [0.9, 0.9]));
        assert!(isl.end_generation().is_empty(), "generation 1 of period 2");
        let moved = isl.end_generation();
        assert_eq!(moved, vec![(0, 1, "a".to_string()), (1, 2, "b".to_string())]);
        // Next migration: island 0's best is already known to island 1 (duplicate).
        isl.end_generation();
        let again = isl.end_generation();
        assert!(!again.iter().any(|(f, t, _)| (*f, *t) == (0, 1)), "{again:?}");
    }

    #[test]
    fn the_bandit_tries_every_arm_then_favours_the_better_one_without_starving_the_others() {
        let mut rng = Rng::new(3);
        let mut arms = vec![ArmStat::default(); 3];
        let truth = [0.2, 0.8, 0.3];
        let mut pulls = [0u32; 3];
        for _ in 0..300 {
            let i = next_arm(&arms, 1.0, &mut rng).unwrap();
            pulls[i] += 1;
            arms[i].pulls += 1;
            arms[i].reward_sum += truth[i] + 0.05 * rng.next_normal();
        }
        assert!(pulls.iter().all(|p| *p >= 5), "no arm starved: {pulls:?}");
        assert!(pulls[1] > pulls[0] && pulls[1] > pulls[2], "{pulls:?}");
        assert_eq!(next_arm(&[], 1.0, &mut rng), None);
        assert_eq!(next_arm(&[ArmStat::default(), ArmStat { pulls: 4, reward_sum: 1.0 }], 1.0, &mut rng), Some(0));
    }

    /// The acceptance fixture: a 200-backtest campaign occupies at least five distinct
    /// feature cells and its campaign PBO is at most 0.5.
    #[test]
    fn a_two_hundred_backtest_fixture_campaign_fills_cells_and_is_not_overfit() {
        let mut rng = Rng::new(11);
        let periods = 240usize;
        let mut perf: Vec<Vec<f64>> = Vec::new();
        let mut arch = Archive::new(GridSpec { dims: vec![(0.0, 1.0, 5), (0.0, 1.0, 5)] });
        for i in 0..200 {
            // 30 configurations carry a genuine per-period edge; the rest are noise.
            let edge = if i % 7 == 0 { 0.04 + 0.0004 * (i as f64) } else { 0.0 };
            let row: Vec<f64> = (0..periods).map(|_| edge + rng.next_normal()).collect();
            let score = row.iter().sum::<f64>() / periods as f64;
            let descriptor = [rng.next_f64(), rng.next_f64()];
            arch.offer(cand(&format!("c{i}"), &format!("h{i}"), score, descriptor));
            perf.push(row);
        }
        assert!(arch.occupied() >= 5, "occupied {}", arch.occupied());
        assert_eq!(arch.evaluated(), 200);
        let pbo = campaign_pbo(&perf, 8);
        assert!(pbo <= 0.5, "campaign_pbo {pbo}");
    }
}
