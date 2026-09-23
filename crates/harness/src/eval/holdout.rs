//! The eval suite's own holdout (AGENT-009 §4.2; Set S task S-4.17).
//!
//! # The uncomfortable one
//!
//! This platform's entire thesis is that tuning against the data you evaluate on
//! produces a number that means nothing. The eval suite is not exempt. Every
//! harness change is tuned until the suites pass, the suites are small, and after
//! twenty changes the harness is fitted to its own cases — and the scorecard says
//! everything is fine, in exactly the way a curve-fitted backtest says everything
//! is fine.
//!
//! So 20% of every suite's cases are **sealed by hash**, chosen once, and are
//! **not run on PRs**. Nobody tunes against a number they cannot see. The sealed
//! set runs on the nightly promotion to `current`, and
//! [`super::scorecard::fitted_to_its_evals`] refuses to promote a version whose
//! open-set rate exceeds its sealed-set rate by more than the flakiness band.
//!
//! If that sounds severe for an internal test suite: it is the identical argument
//! the platform makes to a researcher who wants one more look at a holdout, and a
//! rule the platform will not apply to itself is a rule it does not believe.
//!
//! # Why by hash, and not by a random draw
//!
//! A random draw needs a seed, and a seed needs to be stored, and a stored seed
//! is a thing somebody re-rolls when the sealed set is inconvenient. A hash of
//! the case id and the draw's salt is reproducible from the case list alone: the
//! same corpus always yields the same seal, on every machine, with nothing to
//! keep. Re-drawing requires changing the salt, which is a visible edit that
//! [`HoldoutDraw`] records with the version that asked for it.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

/// §4.2's share.
pub const SEALED_SHARE: f64 = 0.20;

/// §4.2: the sealed set is re-drawn only when the suite grows by 50%.
pub const REDRAW_GROWTH: f64 = 1.50;

/// A recorded draw. The look-accounting the platform applies to a strategy's
/// holdout, applied to itself.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HoldoutDraw {
    /// What makes this draw different from the last. Changing it is the only way
    /// to re-draw, and it is a visible edit.
    pub salt: String,
    /// Which harness version asked for the draw. §4.2 requires the requester to
    /// be recorded, because "who wanted another look" is the question a holdout
    /// discipline exists to answer.
    pub requested_by: String,
    /// ISO date of the draw.
    pub drawn_on: String,
    /// How many cases existed at the draw. A re-draw is legitimate only once the
    /// corpus has grown by [`REDRAW_GROWTH`].
    pub corpus_size: usize,
}

/// Which cases are sealed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Holdout {
    pub draw: HoldoutDraw,
    pub sealed: BTreeSet<String>,
}

/// A stable hash of a case id under a salt.
///
/// `DefaultHasher` is not a cryptographic commitment and does not need to be:
/// this decides which cases are hidden from a team that is not trying to cheat,
/// and the real defence is that the sealed *results* are only published after
/// promotion. A `sha2` dependency is also not available to this crate (gate
/// H-1).
fn seal_hash(salt: &str, case_id: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    salt.hash(&mut h);
    case_id.hash(&mut h);
    h.finish()
}

impl Holdout {
    /// Seal 20% of each suite's cases, deterministically.
    ///
    /// **Per suite, not over the whole corpus.** A global 20% can empty a small
    /// suite's holdout entirely and take 40% of a large one's — and the small
    /// suites are exactly the ones a harness gets fitted to fastest.
    #[must_use]
    pub fn draw(cases: &[(String, String)], draw: HoldoutDraw) -> Self {
        let mut by_suite: std::collections::BTreeMap<&str, Vec<&String>> =
            std::collections::BTreeMap::new();
        for (suite, id) in cases {
            by_suite.entry(suite.as_str()).or_default().push(id);
        }
        let mut sealed = BTreeSet::new();
        for (_, ids) in by_suite {
            let mut ranked: Vec<(u64, &String)> = ids
                .iter()
                .map(|id| (seal_hash(&draw.salt, id), *id))
                .collect();
            ranked.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(b.1)));
            // At least one whenever the suite has two or more cases: a suite
            // with an empty holdout is a suite with no holdout, and rounding a
            // 4-case suite down to zero is how that happens quietly.
            let n = if ranked.len() < 2 {
                0
            } else {
                ((ranked.len() as f64 * SEALED_SHARE).round() as usize).max(1)
            };
            for (_, id) in ranked.into_iter().take(n) {
                sealed.insert(id.clone());
            }
        }
        Self { draw, sealed }
    }

    #[must_use]
    pub fn is_sealed(&self, case_id: &str) -> bool {
        self.sealed.contains(case_id)
    }

    /// The cases a PR run may execute. §4.2's first rule, as a function.
    #[must_use]
    pub fn open_set<'a>(&self, cases: &'a [(String, String)]) -> Vec<&'a String> {
        cases
            .iter()
            .map(|(_, id)| id)
            .filter(|id| !self.is_sealed(id))
            .collect()
    }

    /// Whether a re-draw is allowed yet (§4.2).
    ///
    /// Returns the reason it is refused, or `None` when it is legitimate. A
    /// re-draw on demand is the loophole: seal the cases you keep failing, draw
    /// again, and the holdout becomes a way of hiding a regression.
    #[must_use]
    pub fn refuse_redraw(&self, corpus_now: usize) -> Option<String> {
        let needed = (self.draw.corpus_size as f64 * REDRAW_GROWTH).ceil() as usize;
        if corpus_now < needed {
            return Some(format!(
                "the suite has {corpus_now} cases and the last draw was at {}; a re-draw needs \
                 {needed} (50% growth). Re-drawing on demand turns the holdout into a way of \
                 hiding a regression.",
                self.draw.corpus_size
            ));
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eval::cases;

    fn corpus() -> Vec<(String, String)> {
        cases::all()
            .into_iter()
            .map(|c| (c.suite.as_str().to_string(), c.id))
            .collect()
    }

    fn draw() -> HoldoutDraw {
        HoldoutDraw {
            salt: "set-s-phase-4".into(),
            requested_by: "S4".into(),
            drawn_on: "2026-09-15".into(),
            corpus_size: corpus().len(),
        }
    }

    /// The same corpus always yields the same seal, on every machine, with
    /// nothing to keep but the salt.
    #[test]
    fn the_draw_is_reproducible_from_the_case_list_and_the_salt() {
        let a = Holdout::draw(&corpus(), draw());
        let b = Holdout::draw(&corpus(), draw());
        assert_eq!(a.sealed, b.sealed);
        assert!(!a.sealed.is_empty());
    }

    /// Per suite, not over the corpus. A global 20% empties the small suites,
    /// which are the ones a harness gets fitted to fastest.
    #[test]
    fn every_suite_with_two_or_more_cases_has_a_sealed_case() {
        let c = corpus();
        let h = Holdout::draw(&c, draw());
        let mut sizes: std::collections::BTreeMap<&str, (usize, usize)> =
            std::collections::BTreeMap::new();
        for (suite, id) in &c {
            let e = sizes.entry(suite.as_str()).or_default();
            e.0 += 1;
            if h.is_sealed(id) {
                e.1 += 1;
            }
        }
        for (suite, (total, sealed)) in sizes {
            if total >= 2 {
                assert!(sealed >= 1, "{suite} has {total} cases and none sealed");
            }
            assert!(sealed < total, "{suite} sealed all of its cases");
        }
    }

    /// S-4.17's acceptance: the sealed set never runs on a PR.
    #[test]
    fn the_open_set_a_pr_runs_excludes_every_sealed_case() {
        let c = corpus();
        let h = Holdout::draw(&c, draw());
        let open = h.open_set(&c);
        assert!(open.iter().all(|id| !h.is_sealed(id)));
        assert_eq!(open.len(), c.len() - h.sealed.len());
        assert!(!open.is_empty());
    }

    /// The loophole, closed: seal the cases you keep failing, draw again.
    #[test]
    fn a_redraw_before_fifty_percent_growth_is_refused_and_says_why() {
        let h = Holdout::draw(&corpus(), draw());
        let why = h.refuse_redraw(h.draw.corpus_size + 1).expect("refused");
        assert!(why.contains("hiding a regression"), "{why}");
        assert!(h
            .refuse_redraw((h.draw.corpus_size as f64 * 1.5).ceil() as usize)
            .is_none());
    }

    /// §4.2 requires the requester to be recorded. "Who wanted another look" is
    /// the question a holdout discipline exists to answer.
    #[test]
    fn a_draw_records_who_asked_for_it_and_when() {
        let h = Holdout::draw(&corpus(), draw());
        assert_eq!(h.draw.requested_by, "S4");
        assert_eq!(h.draw.drawn_on, "2026-09-15");
        let s = serde_json::to_string(&h).unwrap();
        let back: Holdout = serde_json::from_str(&s).unwrap();
        assert_eq!(back, h);
    }

    /// A different salt draws a different set — which is what makes a re-draw a
    /// visible edit rather than a silent one.
    #[test]
    fn changing_the_salt_is_what_redraws_and_it_is_visible() {
        let a = Holdout::draw(&corpus(), draw());
        let mut d2 = draw();
        d2.salt = "set-s-phase-4-redraw".into();
        d2.requested_by = "S5".into();
        let b = Holdout::draw(&corpus(), d2);
        assert_ne!(a.sealed, b.sealed, "a re-draw must actually move the set");
        assert_ne!(a.draw.requested_by, b.draw.requested_by);
    }

    #[test]
    fn a_single_case_suite_seals_nothing_rather_than_sealing_everything() {
        let one = vec![("solo".to_string(), "solo/only".to_string())];
        let h = Holdout::draw(&one, draw());
        assert!(
            h.sealed.is_empty(),
            "sealing the only case leaves no open set"
        );
    }
}
