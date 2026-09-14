//! The tier-2 reference portfolio (SPEC §5.2, checklist 3.6, R-03).
//!
//! A small set of strategy configurations chosen so that *between them* they
//! cover the outcome space: for every cell of the tensor, at least one member
//! has been run in it. The portfolio's **scores** are then the tier-2 embedding
//! — "how does this asset respond to these eight things" — and R-03's point is
//! that those scores carry the routing signal, not the raw statistics.
//!
//! Selection is greedy over coverage, which is the standard algorithm for a
//! submodular objective and comes with the standard `1 − 1/e` guarantee. The
//! guarantee matters less than the property underneath it: adding a member that
//! covers cells nobody else covers is worth more than adding one that is
//! individually excellent, and greedy coverage is the rule that prefers the
//! first. A portfolio picked by "the eight best strategies" is eight views of
//! the same corner.

use std::collections::BTreeSet;

/// Where the portfolio starts, and where it stops growing.
///
/// Eight is the cold-start size — small enough that running all of them on a new
/// asset is affordable, large enough to span the archetypes. Twenty-four is the
/// ceiling: past it the marginal member covers almost nothing and every new
/// asset pays for it forever.
pub const COLD_START_SIZE: usize = 8;
pub const MAX_SIZE: usize = 24;

/// One candidate member and the tensor cells it has been observed in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub config_hash: String,
    /// `(cluster, regime)` pairs this configuration has an outcome for.
    pub covers: BTreeSet<(i32, i32)>,
}

/// What the greedy selection produced.
#[derive(Clone, Debug, PartialEq)]
pub struct Portfolio {
    pub members: Vec<String>,
    /// Cells covered by at least one member.
    pub covered: usize,
    /// Cells no member reaches. Reported rather than hidden: a portfolio is a
    /// basis for the tier-2 embedding, and a cell nothing covers is a direction
    /// the embedding cannot express.
    pub uncovered: usize,
}

impl Portfolio {
    #[must_use]
    pub fn coverage(&self) -> f64 {
        let total = self.covered + self.uncovered;
        if total == 0 {
            return 0.0;
        }
        #[allow(clippy::cast_precision_loss)]
        {
            self.covered as f64 / total as f64
        }
    }
}

/// Grow a portfolio greedily by marginal coverage.
///
/// `target` is clamped to `[COLD_START_SIZE, MAX_SIZE]`. Ties break on the
/// config hash so the selection is reproducible: a portfolio that differs
/// between two runs over the same ledger is a basis nobody can compare against.
///
/// Stops early when no remaining candidate covers anything new — a member that
/// adds nothing is a run every future asset pays for and no one reads.
#[must_use]
pub fn select(candidates: &[Candidate], universe: &BTreeSet<(i32, i32)>, target: usize) -> Portfolio {
    let target = target.clamp(COLD_START_SIZE, MAX_SIZE);
    let mut covered: BTreeSet<(i32, i32)> = BTreeSet::new();
    let mut members: Vec<String> = Vec::new();
    let mut remaining: Vec<&Candidate> = candidates.iter().collect();

    while members.len() < target && !remaining.is_empty() {
        let best = remaining
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| {
                let ga = a.covers.difference(&covered).count();
                let gb = b.covers.difference(&covered).count();
                // Reproducible ties: the hash decides, not the iteration order.
                ga.cmp(&gb).then_with(|| b.config_hash.cmp(&a.config_hash))
            })
            .map(|(i, c)| (i, (*c).clone()));

        let Some((idx, chosen)) = best else { break };
        if chosen.covers.difference(&covered).count() == 0 && !members.is_empty() {
            break;
        }
        covered.extend(chosen.covers.iter().copied());
        members.push(chosen.config_hash.clone());
        remaining.remove(idx);
    }

    let reached = universe.intersection(&covered).count();
    Portfolio { members, covered: reached, uncovered: universe.len() - reached }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(hash: &str, cells: &[(i32, i32)]) -> Candidate {
        Candidate { config_hash: hash.into(), covers: cells.iter().copied().collect() }
    }

    fn universe(cells: &[(i32, i32)]) -> BTreeSet<(i32, i32)> {
        cells.iter().copied().collect()
    }

    /// The whole reason for greedy coverage: a member that reaches somewhere
    /// nobody else does beats one that is individually excellent in a corner
    /// everyone already covers.
    #[test]
    fn coverage_beats_individual_excellence() {
        let u = universe(&[(0, 0), (0, 1), (1, 0), (1, 1), (2, 0), (2, 1), (3, 0), (3, 1)]);
        let mut cands = vec![
            candidate("broad", &[(0, 0), (0, 1), (1, 0), (1, 1)]),
            candidate("corner", &[(0, 0)]),
            candidate("far", &[(2, 0), (2, 1), (3, 0), (3, 1)]),
        ];
        // Eight more duplicates of the crowded corner: individually fine, and
        // between them they add one cell.
        for i in 0..8 {
            cands.push(candidate(&format!("dup{i}"), &[(0, 0), (0, 1)]));
        }
        let p = select(&cands, &u, COLD_START_SIZE);
        assert!(p.members.contains(&"far".to_string()), "{:?}", p.members);
        assert_eq!(p.uncovered, 0, "two members already span it");
    }

    #[test]
    fn the_selection_is_reproducible() {
        let u = universe(&[(0, 0), (1, 1)]);
        let c = vec![candidate("a", &[(0, 0)]), candidate("b", &[(0, 0)]), candidate("z", &[(1, 1)])];
        let first = select(&c, &u, COLD_START_SIZE);
        let second = select(&c, &u, COLD_START_SIZE);
        assert_eq!(first, second, "a portfolio that moves between runs is no basis");
    }

    /// A member that covers nothing new is a run every future asset pays for and
    /// nobody reads.
    #[test]
    fn growth_stops_when_nothing_new_is_reachable() {
        let u = universe(&[(0, 0)]);
        let c: Vec<Candidate> = (0..20).map(|i| candidate(&format!("c{i}"), &[(0, 0)])).collect();
        let p = select(&c, &u, MAX_SIZE);
        assert_eq!(p.members.len(), 1);
        assert!((p.coverage() - 1.0).abs() < f64::EPSILON);
    }

    /// A cell nothing reaches is a direction the tier-2 embedding cannot
    /// express, so it is reported rather than rounded away.
    #[test]
    fn unreachable_cells_are_reported_not_hidden() {
        let u = universe(&[(0, 0), (9, 9)]);
        let p = select(&[candidate("only", &[(0, 0)])], &u, COLD_START_SIZE);
        assert_eq!(p.covered, 1);
        assert_eq!(p.uncovered, 1);
        assert!((p.coverage() - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn the_size_is_clamped_to_the_declared_range() {
        let u = universe(&(0..40).map(|i| (i, 0)).collect::<Vec<_>>());
        let c: Vec<Candidate> = (0..40).map(|i| candidate(&format!("c{i}"), &[(i, 0)])).collect();
        assert_eq!(select(&c, &u, 1).members.len(), COLD_START_SIZE);
        assert_eq!(select(&c, &u, 1_000).members.len(), MAX_SIZE);
    }

    #[test]
    fn an_empty_ledger_yields_an_empty_portfolio_rather_than_a_panic() {
        let p = select(&[], &universe(&[(0, 0)]), COLD_START_SIZE);
        assert!(p.members.is_empty());
        assert_eq!(p.uncovered, 1);
        assert!((p.coverage() - 0.0).abs() < f64::EPSILON);
    }
}
