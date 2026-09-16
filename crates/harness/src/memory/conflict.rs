//! Contradiction and near-duplicate detection (AGENT-007 §2.2.1, §2.5, P5;
//! Set S tasks S-4.15 and S-4.16).
//!
//! # Why this file is the memory-poisoning guard
//!
//! P5 says conflicts are surfaced and never merged, and §2.2 gives every entry a
//! `conflicts_with[]` — but in the first draft **nothing computed it**, which
//! would have left the principle as a permanently empty column. One wrong
//! `established` finding then becomes an authoritative premise for every later
//! run, and the whole honesty stack rests on an unchallenged claim.
//!
//! # The three outcomes, and why there are three
//!
//! The obvious design has two — conflict or not — and it is wrong in the
//! expensive direction. §2.2.1 step 2 says two findings that disagree on a number
//! whose confidence intervals overlap are *not* a contradiction; they are one
//! finding with a wider interval. A two-outcome check either flags those (and the
//! desk fills with resolution leads that resolve to "measurement noise"), or
//! silences them (and a real disagreement is merged away). So:
//!
//! | Relation | When | What happens |
//! |---|---|---|
//! | [`Relation::Contradiction`] | same quantity, opposing direction, intervals **disjoint** | both rows linked, **neither downgraded**, both enter the brief |
//! | [`Relation::NearDuplicate`] | same quantity, same direction, windows overlap, intervals overlap | no new row; corroboration count incremented |
//! | [`Relation::Independent`] | everything else, **including opposing directions whose intervals overlap** | nothing |
//!
//! # Neither side is downgraded
//!
//! §2.2.1 step 3, and it is the least intuitive rule here: a later finding is not
//! more true for being later. Auto-downgrading the older one is how a single bad
//! measurement quietly overwrites a good one — which is precisely the poisoning
//! this check exists to prevent. Resolution is a *research act*: a third finding
//! citing both.

use super::{Confidence, Entry, Kind};

/// How a new entry stands to an existing one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Relation {
    /// Opposing directions on the same measured quantity, with intervals that do
    /// not touch.
    Contradiction {
        quantity: String,
        /// One line for the desk's resolution lead (AGENT-012 §7).
        why: String,
    },
    /// The same measurement, made again. Evidence, not noise.
    NearDuplicate { quantity: String },
    Independent,
}

impl Relation {
    #[must_use]
    pub fn is_contradiction(&self) -> bool {
        matches!(self, Relation::Contradiction { .. })
    }

    #[must_use]
    pub fn is_near_duplicate(&self) -> bool {
        matches!(self, Relation::NearDuplicate { .. })
    }
}

/// §2.2.1 step 1: is this candidate even comparable?
///
/// Same subject, both findings, both live, both not archived. `stale = false` is
/// in the spec's candidate set: a finding the platform has already stopped
/// believing does not get to contradict a fresh one.
fn comparable(new: &Entry, candidate: &Entry) -> bool {
    new.kind == Kind::Finding
        && candidate.kind == Kind::Finding
        && new.id != candidate.id
        && new.subject == candidate.subject
        && !candidate.stale
        && !candidate.archived
        && candidate.superseded_by.is_none()
}

/// Compare a new finding against one existing entry.
///
/// Pure and total: it reads only the structured [`super::Measurement`], never the
/// claim prose. §2.2.1 step 2 requires that, and the reason is that "does not
/// beat" and "beats" differ by one word a parser gets wrong about as often as it
/// matters.
#[must_use]
pub fn relate(new: &Entry, candidate: &Entry) -> Relation {
    if !comparable(new, candidate) {
        return Relation::Independent;
    }
    let (Some(a), Some(b)) = (new.measurement.as_ref(), candidate.measurement.as_ref()) else {
        return Relation::Independent;
    };
    if a.quantity != b.quantity {
        return Relation::Independent;
    }

    let windows_overlap = new.data_window.overlaps(&candidate.data_window);
    let intervals_overlap = a.intervals_overlap(b);

    if a.direction.opposes(b.direction) {
        // Step 2's exception. Disagreement inside the measurement error is one
        // finding with a wider interval, not two findings in conflict.
        if intervals_overlap {
            return Relation::Independent;
        }
        // A contradiction needs a horizon to disagree *over*. Two opposing
        // measurements on windows that never touch are a regime difference, and
        // calling that a contradiction would flag every honest finding about a
        // changing market.
        if !windows_overlap {
            return Relation::Independent;
        }
        // Only `established` findings can poison. A `suggestive` claim is already
        // marked as the model's arithmetic; linking it as an authoritative
        // contradiction would give it standing it has not earned.
        if candidate.confidence != Confidence::Established
            || new.confidence != Confidence::Established
        {
            return Relation::Independent;
        }
        return Relation::Contradiction {
            quantity: a.quantity.clone(),
            why: format!(
                "{} is {:?} at {} in {} and {:?} at {} in {}; the intervals do not overlap",
                a.quantity,
                a.direction,
                a.value,
                new.id,
                b.direction,
                b.value,
                candidate.id
            ),
        };
    }

    if a.direction == b.direction && windows_overlap && intervals_overlap {
        return Relation::NearDuplicate {
            quantity: a.quantity.clone(),
        };
    }
    Relation::Independent
}

/// What a write should do, given everything already stored about the subject.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteAction {
    /// Store the new row, linked to these ids (possibly none).
    Insert { conflicts_with: Vec<String> },
    /// Do not store. Increment this entry's corroboration count instead (§2.5).
    Corroborate { entry_id: String },
}

/// AGENT-007 §2.2.1 then §2.5, in that order.
///
/// Order matters: a near-duplicate check that ran first would swallow a
/// contradiction whose intervals happened to overlap — but that case is
/// `Independent` by §2.2.1 step 2, so running the contradiction check first is
/// what makes the two rules consistent rather than merely sequenced.
#[must_use]
pub fn decide_write(new: &Entry, existing: &[Entry]) -> WriteAction {
    let mut conflicts = Vec::new();
    let mut duplicate_of = None;
    for c in existing {
        match relate(new, c) {
            Relation::Contradiction { .. } => conflicts.push(c.id.clone()),
            Relation::NearDuplicate { .. } if duplicate_of.is_none() => {
                duplicate_of = Some(c.id.clone());
            }
            _ => {}
        }
    }
    // A finding that both corroborates one row and contradicts another is real —
    // and it must become a row, or the contradiction it raises has nothing to
    // link to. Corroboration is only the right answer when nothing disagrees.
    if conflicts.is_empty() {
        if let Some(id) = duplicate_of {
            return WriteAction::Corroborate { entry_id: id };
        }
    }
    WriteAction::Insert {
        conflicts_with: conflicts,
    }
}

/// Apply a corroboration to the stored entry (§2.5).
///
/// The count is what lets the ranker prefer a finding measured on three windows
/// over a singleton, which is the reason this is better than deduplication.
pub fn corroborate(stored: &mut Entry, from: &Entry) {
    stored.corroborations = stored.corroborations.saturating_add(1);
    stored.corroborated_by.push(from.provenance.run_id.clone());
    for (role, id) in &from.evidence_ids {
        stored
            .evidence_ids
            .entry(format!("{role}@{}", from.provenance.run_id))
            .or_insert_with(|| id.clone());
    }
    // A corroborated finding is not stale, whatever a timer said: someone just
    // measured it again.
    stored.stale = false;
}

/// Link a contradiction, both ways, downgrading neither (§2.2.1 step 3).
pub fn link_conflict(new: &mut Entry, stored: &mut Entry) {
    if !new.conflicts_with.contains(&stored.id) {
        new.conflicts_with.push(stored.id.clone());
    }
    if !stored.conflicts_with.contains(&new.id) {
        stored.conflicts_with.push(new.id.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::{DataWindow, Direction, Entry, Measurement, Provenance};

    fn prov(run: &str) -> Provenance {
        Provenance::new(run, "run_backtest", "v3", "tpl_abc")
    }

    fn finding(
        id: &str,
        run: &str,
        quantity: &str,
        dir: Direction,
        value: f64,
        se: Option<f64>,
        window: (&str, &str),
    ) -> Entry {
        let mut m = Measurement::new(quantity, dir, value);
        if let Some(s) = se {
            m = m.with_se(s);
        }
        let mut e = Entry::finding(
            id,
            "BTC-USD 1h",
            format!("{quantity} is {value}"),
            DataWindow::new(window.0, window.1),
            prov(run),
        )
        .with_measurement(m);
        e.confidence = Confidence::Established;
        e
    }

    /// S-4.15's first acceptance: an opposing-direction finding links both rows
    /// and downgrades neither.
    #[test]
    fn an_opposing_direction_finding_links_both_rows_and_downgrades_neither() {
        let mut old = finding(
            "f_old",
            "run_1",
            "sharpe",
            Direction::Positive,
            0.80,
            Some(0.05),
            ("2026-01-01", "2026-06-30"),
        );
        let mut new = finding(
            "f_new",
            "run_2",
            "sharpe",
            Direction::Negative,
            -0.60,
            Some(0.05),
            ("2026-03-01", "2026-08-31"),
        );
        assert!(relate(&new, &old).is_contradiction());
        link_conflict(&mut new, &mut old);
        assert_eq!(new.conflicts_with, vec!["f_old".to_string()]);
        assert_eq!(old.conflicts_with, vec!["f_new".to_string()]);
        assert_eq!(
            (new.confidence, old.confidence),
            (Confidence::Established, Confidence::Established),
            "a later finding is not more true for being later"
        );
    }

    /// S-4.15's second acceptance, and the trap: disagreement inside the
    /// measurement error is not a contradiction.
    #[test]
    fn overlapping_interval_disagreement_is_not_flagged() {
        let old = finding(
            "f_old",
            "run_1",
            "sharpe",
            Direction::Positive,
            0.10,
            Some(0.30),
            ("2026-01-01", "2026-06-30"),
        );
        let new = finding(
            "f_new",
            "run_2",
            "sharpe",
            Direction::Negative,
            -0.05,
            Some(0.30),
            ("2026-01-01", "2026-06-30"),
        );
        assert_eq!(relate(&new, &old), Relation::Independent);
    }

    #[test]
    fn different_quantities_never_contradict() {
        let old = finding(
            "f_old",
            "run_1",
            "sharpe",
            Direction::Positive,
            0.8,
            Some(0.01),
            ("2026-01-01", "2026-06-30"),
        );
        let new = finding(
            "f_new",
            "run_2",
            "hit_rate",
            Direction::Negative,
            -0.8,
            Some(0.01),
            ("2026-01-01", "2026-06-30"),
        );
        assert_eq!(relate(&new, &old), Relation::Independent);
    }

    #[test]
    fn disjoint_windows_are_a_regime_difference_not_a_contradiction() {
        let old = finding(
            "f_old",
            "run_1",
            "sharpe",
            Direction::Positive,
            0.8,
            Some(0.01),
            ("2025-01-01", "2025-06-30"),
        );
        let new = finding(
            "f_new",
            "run_2",
            "sharpe",
            Direction::Negative,
            -0.8,
            Some(0.01),
            ("2026-01-01", "2026-06-30"),
        );
        assert_eq!(relate(&new, &old), Relation::Independent);
    }

    #[test]
    fn a_suggestive_finding_cannot_raise_an_authoritative_contradiction() {
        let old = finding(
            "f_old",
            "run_1",
            "sharpe",
            Direction::Positive,
            0.8,
            Some(0.01),
            ("2026-01-01", "2026-06-30"),
        );
        let mut new = finding(
            "f_new",
            "run_2",
            "sharpe",
            Direction::Negative,
            -0.8,
            Some(0.01),
            ("2026-01-01", "2026-06-30"),
        );
        new.confidence = Confidence::Suggestive;
        assert_eq!(relate(&new, &old), Relation::Independent);
    }

    /// S-4.16: a near-duplicate increments corroboration rather than inserting.
    #[test]
    fn a_near_duplicate_corroborates_rather_than_inserting() {
        let mut stored = finding(
            "f_old",
            "run_1",
            "sharpe",
            Direction::Positive,
            0.40,
            Some(0.06),
            ("2026-01-01", "2026-06-30"),
        );
        let new = finding(
            "f_new",
            "run_2",
            "sharpe",
            Direction::Positive,
            0.44,
            Some(0.06),
            ("2026-03-01", "2026-08-31"),
        )
        .with_evidence("experiment_id", "exp_44445555");

        assert!(relate(&new, &stored).is_near_duplicate());
        assert_eq!(
            decide_write(&new, std::slice::from_ref(&stored)),
            WriteAction::Corroborate {
                entry_id: "f_old".into()
            }
        );
        corroborate(&mut stored, &new);
        assert_eq!(stored.corroborations, 2);
        assert_eq!(stored.corroborated_by, vec!["run_2".to_string()]);
        assert!(stored
            .evidence_ids
            .values()
            .any(|v| v == "exp_44445555"));
    }

    /// A finding that contradicts something must become a row, or the
    /// contradiction has nothing to link to.
    #[test]
    fn a_contradicting_finding_inserts_even_when_something_else_corroborates_it() {
        let dup = finding(
            "f_dup",
            "run_1",
            "sharpe",
            Direction::Positive,
            0.40,
            Some(0.06),
            ("2026-01-01", "2026-06-30"),
        );
        let opposed = finding(
            "f_opp",
            "run_9",
            "sharpe",
            Direction::Negative,
            -0.90,
            Some(0.02),
            ("2026-01-01", "2026-06-30"),
        );
        let new = finding(
            "f_new",
            "run_2",
            "sharpe",
            Direction::Positive,
            0.42,
            Some(0.06),
            ("2026-02-01", "2026-07-31"),
        );
        assert_eq!(
            decide_write(&new, &[dup, opposed]),
            WriteAction::Insert {
                conflicts_with: vec!["f_opp".to_string()]
            }
        );
    }

    #[test]
    fn a_stale_or_archived_candidate_is_not_a_comparison_target() {
        let mut old = finding(
            "f_old",
            "run_1",
            "sharpe",
            Direction::Positive,
            0.8,
            Some(0.01),
            ("2026-01-01", "2026-06-30"),
        );
        let new = finding(
            "f_new",
            "run_2",
            "sharpe",
            Direction::Negative,
            -0.8,
            Some(0.01),
            ("2026-01-01", "2026-06-30"),
        );
        old.stale = true;
        assert_eq!(relate(&new, &old), Relation::Independent);
        old.stale = false;
        old.archived = true;
        assert_eq!(relate(&new, &old), Relation::Independent);
    }
}
