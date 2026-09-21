//! Choosing the next question (AGENT-014 P2, §5; Set S task S-11.3).
//!
//! `total = value × novelty × feasibility / cost`, a forced exploration share,
//! and a decision row carrying the whole candidate set. The point of all three
//! is the same: "why did it do *that* next?" has to be answerable afterwards, by
//! somebody who was not there.
//!
//! # Every signal carries where its number came from
//!
//! This is the part that is easy to skip and expensive to skip. Of the four
//! signals §5.1 names, this build can measure some and not others — there is no
//! per-run USD on this platform, so `cost` is in **trials**, and the
//! `hypothesis_precheck` factor in `novelty` belongs to S-11.4. A scorer that
//! quietly substituted 1.0 for what it could not measure would produce a ranking
//! that looks exactly like a measured one, and the decision log — the thing every
//! off-policy estimate later reads — would record it as if it were.
//!
//! So a [`Signal`] is a number *and* a [`Provenance`], inseparably, and
//! [`Scored::not_estimated`] lists by name which signals contributed nothing.
//! The decision row carries it. A reader a year from now can tell a ranking that
//! divided by a measured cost from one that did not.
//!
//! # What is normalised and what is not
//!
//! §5.1 says every signal is percentile-normalised, citing AGENT-005 §24.3:
//! unbounded signals eat bounded ones. That argument is about the unbounded ones
//! — `value` and `cost` — and applying it to `feasibility` would be actively
//! wrong: feasibility's whole job is that **0 means impossible**, and a
//! percentile rank turns the lowest value in the slate into something greater
//! than zero. The same holds for `novelty`, whose zero means "already answered".
//!
//! So the two unbounded signals are ranked within the agenda and the two bounded
//! ones multiply as they are. [`Scored::normalised`] shows both, so the
//! difference is visible rather than argued about.
//!
//! # The draw is the platform's
//!
//! §5.2: the exploration share is not in any tool schema and the draw is made
//! with a platform-held seed. [`select`] takes the seed and uses a deterministic
//! generator, so a selection replays exactly — which is what makes the
//! `propensity` on the decision row a fact rather than a claim.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::agenda::{AgendaItem, Score, COST_FLOOR_USD};

/// The selection policy's name and version, as `mlops.decision` requires.
///
/// Bumped when what the scorer is shown or how it combines changes in a way that
/// would change what it picks — because the point of recording them is that a
/// later analysis can tell decisions made under one policy from the next.
pub const POLICY: &str = "session_agenda";
pub const POLICY_VERSION: i32 = 1;

/// The divisor in novelty's decay, from AGENT-014 §5.1.
///
/// The spec applies it to the family's *trial* count. This build feeds it the
/// count of prior hypotheses stating the same question, because that is the
/// registry the platform keeps (`agent_lifecycle.hypothesis`) and a trial is not
/// joinable to an agenda item's subject today. The divisor is left at the spec's
/// value rather than rescaled for the smaller count, which makes novelty decay
/// **more slowly** than the spec intends — the safe direction, because layers one
/// and two still refuse an actual duplicate and this only affects ranking.
pub const NOVELTY_HALFLIFE: f64 = 20.0;

/// Where a signal's number came from.
///
/// Not optional and not `Option<f64>`: a number whose provenance is optional is a
/// number that will eventually be reported without it. The same argument
/// `ledger::internal::Answer` makes for estimates, one level up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Provenance {
    /// Computed from rows the platform holds. `n` is how many it counted, so a
    /// reader can tell a median of three from a median of three hundred.
    Measured { n: i64 },
    /// From a table the spec declares — the source weights of §5.1.
    Declared,
    /// Nothing reads this yet. The signal contributes its identity and
    /// [`Scored::not_estimated`] names it.
    NotEstimated,
}

impl Provenance {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Provenance::Measured { .. } => "measured",
            Provenance::Declared => "declared",
            Provenance::NotEstimated => "not_estimated",
        }
    }

    #[must_use]
    pub fn is_estimated(self) -> bool {
        !matches!(self, Provenance::NotEstimated)
    }
}

/// One signal: a number and where it came from.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Signal {
    pub raw: f64,
    pub provenance: Provenance,
}

impl Signal {
    #[must_use]
    pub fn measured(raw: f64, n: i64) -> Self {
        Self {
            raw,
            provenance: Provenance::Measured { n },
        }
    }

    #[must_use]
    pub fn declared(raw: f64) -> Self {
        Self {
            raw,
            provenance: Provenance::Declared,
        }
    }

    /// The identity for this signal's place in the product.
    ///
    /// `1.0` for all four: an unmeasured multiplier and an unmeasured divisor
    /// both leave the product alone, which is the only substitution that does not
    /// silently rank one item above another.
    #[must_use]
    pub fn not_estimated() -> Self {
        Self {
            raw: 1.0,
            provenance: Provenance::NotEstimated,
        }
    }
}

/// The four signals for one item, before normalisation.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Inputs {
    /// Objective weight × source weight. Unbounded above; normalised.
    pub value: Signal,
    /// `(0, 1]`, or 0 for a question already answered. Not normalised.
    pub novelty: Signal,
    /// `{0, 0.5, 1}`. Not normalised — 0 must stay 0.
    pub feasibility: Signal,
    /// Expected spend. Unbounded above; normalised. Floored so a workflow that
    /// looks free cannot dominate.
    pub cost: Signal,
}

/// One item, scored.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Scored {
    pub item: Uuid,
    /// The signals as measured.
    pub raw: Score,
    /// The signals as combined — `value` and `cost` ranked within the agenda.
    pub normalised: Score,
    pub inputs: Inputs,
    /// Whether the scorer was skipped entirely for this item (`user`, §4.2).
    pub bypassed: bool,
}

impl Scored {
    /// The signals that contributed nothing because nothing measures them.
    ///
    /// Named rather than counted, because "two signals were not estimated" does
    /// not tell a reader whether the ranking divided by cost.
    #[must_use]
    pub fn not_estimated(&self) -> Vec<&'static str> {
        let mut out = Vec::new();
        for (name, s) in [
            ("value", self.inputs.value),
            ("novelty", self.inputs.novelty),
            ("feasibility", self.inputs.feasibility),
            ("cost", self.inputs.cost),
        ] {
            if !s.provenance.is_estimated() {
                out.push(name);
            }
        }
        out
    }

    /// Whether this item could be drawn as an exploration pick (§5.2: feasible
    /// and novel).
    #[must_use]
    pub fn explorable(&self) -> bool {
        self.inputs.feasibility.raw > 0.0 && self.inputs.novelty.raw > 0.0
    }
}

/// Percentile rank within a slate, in `(0, 1]`.
///
/// Ties share the rank of the highest tied position, so two identical values
/// cannot be ordered by an accident of insertion. A slate of one gets 1.0, which
/// is right: the only item is the best item.
fn percentiles(values: &[f64]) -> Vec<f64> {
    let n = values.len();
    if n == 0 {
        return Vec::new();
    }
    if n == 1 {
        return vec![1.0];
    }
    let mut out = vec![0.0; n];
    for (i, v) in values.iter().enumerate() {
        // How many are strictly below, plus the ties, counted once.
        let below = values.iter().filter(|o| *o < v).count();
        let ties = values.iter().filter(|o| (*o - v).abs() < f64::EPSILON).count();
        #[allow(clippy::cast_precision_loss)]
        let rank = (below + ties) as f64 / n as f64;
        out[i] = rank;
    }
    out
}

/// Score an agenda.
///
/// `items` pairs each item with the signals its caller measured. The order of
/// the output matches the input, which is what lets `§5.4`'s tie-break —
/// `(created_at, id)` — be the caller's ordering rather than a second rule here.
#[must_use]
pub fn score(items: &[(AgendaItem, Inputs)]) -> Vec<Scored> {
    let values: Vec<f64> = items.iter().map(|(_, i)| i.value.raw).collect();
    let costs: Vec<f64> = items
        .iter()
        .map(|(_, i)| i.cost.raw.max(COST_FLOOR_USD))
        .collect();
    let value_pct = percentiles(&values);
    let cost_pct = percentiles(&costs);

    items
        .iter()
        .enumerate()
        .map(|(n, (item, inputs))| {
            let raw = Score::combine(
                inputs.value.raw,
                inputs.novelty.raw,
                inputs.feasibility.raw,
                inputs.cost.raw,
            );
            let normalised = Score::combine(
                value_pct[n],
                inputs.novelty.raw,
                inputs.feasibility.raw,
                // A rank in (0, 1] as a divisor: the cheapest item in the slate
                // divides by 1/n rather than by its dollars, so one very
                // expensive outlier cannot flatten everything else to zero.
                cost_pct[n],
            );
            Scored {
                item: item.id,
                raw,
                normalised,
                inputs: *inputs,
                bypassed: item.bypasses_scoring(),
            }
        })
        .collect()
}

/// What the draw chose, and with what probability.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Selection {
    pub chosen: Uuid,
    /// True when the draw took the detour rather than the ranking.
    pub exploration: bool,
    /// `P(chosen | slate)` under the mixture of greedy and exploration. A fact,
    /// because the draw is deterministic given the seed.
    pub propensity: f64,
    /// One sentence, for the digest.
    pub rationale: String,
}

/// Choose the next item (§5.2, §5.3).
///
/// A `user` item bypasses all of this and is returned immediately with
/// `propensity = 1.0`: it was not drawn, it was asked for.
///
/// Otherwise, with probability `share` the draw is **uniform over explorable
/// items that are not the top one** — §5.2's "the item the scorer would not have
/// picked", read at its narrowest. `k = 1`, because any larger `k` is a number
/// nobody chose and the spec's own plain-English gloss is about the single item
/// the ranking would have taken.
///
/// Returns `None` for an empty or wholly infeasible slate — there is nothing to
/// choose, which is a different answer from choosing badly.
#[must_use]
pub fn select(scored: &[Scored], share: f64, seed: u64) -> Option<Selection> {
    if scored.is_empty() {
        return None;
    }
    if let Some(asked) = scored.iter().find(|s| s.bypassed) {
        return Some(Selection {
            chosen: asked.item,
            exploration: false,
            propensity: 1.0,
            rationale: "you asked for this one, so it goes next whatever the ranking says".into(),
        });
    }

    // Ranking order. Ties keep the caller's order, which is §5.4's
    // `(created_at, id)`.
    let mut order: Vec<usize> = (0..scored.len()).collect();
    order.sort_by(|a, b| {
        scored[*b]
            .normalised
            .total
            .partial_cmp(&scored[*a].normalised.total)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let best = order.iter().copied().find(|n| scored[*n].explorable())?;
    let detours: Vec<usize> = order
        .iter()
        .copied()
        .filter(|n| *n != best && scored[*n].explorable())
        .collect();

    let draw = unit_draw(seed);
    let exploring = !detours.is_empty() && draw < share;

    if exploring {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let pick = detours[(unit_draw(seed.wrapping_mul(6_364_136_223_846_793_005))
            * detours.len() as f64) as usize
            % detours.len()];
        #[allow(clippy::cast_precision_loss)]
        let p = share / detours.len() as f64;
        return Some(Selection {
            chosen: scored[pick].item,
            exploration: true,
            propensity: p,
            rationale: format!(
                "a deliberate detour: the ranking put this {} of {}, and at least one pick in ten \
                 has to be able to show the ranking wrong",
                order.iter().position(|n| *n == pick).unwrap_or(0) + 1,
                scored.len()
            ),
        });
    }

    let s = &scored[best];
    Some(Selection {
        chosen: s.item,
        exploration: false,
        // The best item is excluded from the detour set, so it is reachable only
        // through the greedy branch.
        propensity: if detours.is_empty() { 1.0 } else { 1.0 - share },
        rationale: format!(
            "highest score {:.3}: value {:.2}, novelty {:.2}, feasibility {:.1}, cost {:.2}{}",
            s.normalised.total,
            s.raw.value,
            s.raw.novelty,
            s.raw.feasibility,
            s.raw.cost,
            match s.not_estimated().as_slice() {
                [] => String::new(),
                missing => format!(" (not estimated: {})", missing.join(", ")),
            }
        ),
    })
}

/// A deterministic draw in `[0, 1)` from a seed.
///
/// SplitMix64, written out rather than pulled in: the property wanted is that
/// the same seed gives the same draw forever, and a dependency that changes its
/// generator between versions would silently make old decision rows
/// unreproducible.
fn unit_draw(seed: u64) -> f64 {
    let mut z = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    // 53 bits is every value an f64 can hold exactly in [0, 1).
    #[allow(clippy::cast_precision_loss)]
    {
        (z >> 11) as f64 / (1_u64 << 53) as f64
    }
}

/// The seed for one selection in one session.
///
/// Derived from the session and the position in its log, so the draw is
/// reproducible from the row and differs between successive selections without
/// anybody storing a counter. The session id alone would make every selection in
/// a session identical; a clock would make none of them reproducible.
#[must_use]
pub fn seed_for(session: Uuid, event_seq: i64) -> u64 {
    let b = session.as_bytes();
    let mut hi = 0_u64;
    let mut lo = 0_u64;
    for (i, byte) in b.iter().enumerate() {
        if i < 8 {
            hi = (hi << 8) | u64::from(*byte);
        } else {
            lo = (lo << 8) | u64::from(*byte);
        }
    }
    hi ^ lo.rotate_left(17) ^ (event_seq as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
}

/// The candidate set a decision row records: every scored item with its signals.
#[must_use]
pub fn candidate_set(scored: &[Scored]) -> Vec<serde_json::Value> {
    scored
        .iter()
        .map(|s| {
            serde_json::json!({
                "item": s.item,
                "total": s.normalised.total,
                "raw": {
                    "value": s.raw.value,
                    "novelty": s.raw.novelty,
                    "feasibility": s.raw.feasibility,
                    "cost": s.raw.cost,
                },
                "provenance": {
                    "value": s.inputs.value.provenance.as_str(),
                    "novelty": s.inputs.novelty.provenance.as_str(),
                    "feasibility": s.inputs.feasibility.provenance.as_str(),
                    "cost": s.inputs.cost.provenance.as_str(),
                },
                "not_estimated": s.not_estimated(),
                "explorable": s.explorable(),
            })
        })
        .collect()
}

/// Novelty from the count of prior statements of the same question (§5.1).
#[must_use]
pub fn novelty_from_prior(n_prior: i64) -> Signal {
    #[allow(clippy::cast_precision_loss)]
    let n = n_prior.max(0) as f64;
    Signal::measured(1.0 / (1.0 + (1.0 + n / NOVELTY_HALFLIFE).ln()), n_prior)
}

/// A score index by item, for a caller holding the agenda.
#[must_use]
pub fn by_item(scored: &[Scored]) -> BTreeMap<Uuid, Score> {
    scored.iter().map(|s| (s.item, s.normalised)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agenda::Source;

    fn item(q: &str, source: Source) -> AgendaItem {
        AgendaItem::new(Uuid::nil(), q, "BTC-USD", source)
    }

    fn inputs(value: f64, novelty: f64, feasibility: f64, cost: f64) -> Inputs {
        Inputs {
            value: Signal::declared(value),
            novelty: Signal::measured(novelty, 1),
            feasibility: Signal::declared(feasibility),
            cost: Signal::measured(cost, 5),
        }
    }

    #[test]
    fn an_infeasible_item_scores_zero_however_valuable_it_is() {
        let slate = vec![
            (item("impossible", Source::Contradiction), inputs(99.0, 1.0, 0.0, 0.1)),
            (item("ordinary", Source::CharterSeed), inputs(1.0, 1.0, 1.0, 1.0)),
        ];
        let scored = score(&slate);
        assert!(scored[0].raw.total.abs() < 1e-12);
        assert!(
            scored[0].normalised.total.abs() < 1e-12,
            "percentile-normalising feasibility would have turned an impossible item into a \
             possible one, which is the whole reason it is not normalised"
        );
        assert!(scored[1].normalised.total > 0.0);
        assert!(!scored[0].explorable());
    }

    #[test]
    fn a_question_already_answered_is_not_drawn_as_a_detour() {
        let slate = vec![
            (item("best", Source::CharterSeed), inputs(2.0, 1.0, 1.0, 1.0)),
            (item("asked before", Source::CharterSeed), inputs(2.0, 0.0, 1.0, 1.0)),
        ];
        let scored = score(&slate);
        assert!(!scored[1].explorable());
        // With no explorable detour, every draw is greedy and says so.
        for seq in 0..20 {
            let s = select(&scored, 0.9, seed_for(Uuid::nil(), seq)).expect("a choice");
            assert_eq!(s.chosen, scored[0].item);
            assert!(!s.exploration);
            assert!((s.propensity - 1.0).abs() < 1e-12);
        }
    }

    #[test]
    fn a_user_item_is_taken_without_being_drawn() {
        let slate = vec![
            (item("scored high", Source::Contradiction), inputs(9.0, 1.0, 1.0, 0.1)),
            (item("asked for", Source::User), inputs(0.1, 0.1, 1.0, 9.0)),
        ];
        let scored = score(&slate);
        let s = select(&scored, 0.15, 1).expect("a choice");
        assert_eq!(s.chosen, scored[1].item);
        assert!(!s.exploration);
        assert!((s.propensity - 1.0).abs() < 1e-12, "it was asked for, not drawn");
        assert!(s.rationale.contains("asked for"));
    }

    #[test]
    fn the_exploration_share_is_achieved_over_forty_selections() {
        // AGENT-014 §12.3: over 40 selections at 0.15, between 3 and 9 are
        // exploration picks. Asserted against the spec's own interval rather
        // than against whatever this implementation happens to produce.
        let slate: Vec<(AgendaItem, Inputs)> = (0..6)
            .map(|n| {
                (
                    item(&format!("q{n}"), Source::CharterSeed),
                    #[allow(clippy::cast_precision_loss)]
                    inputs(f64::from(n) + 1.0, 1.0, 1.0, 1.0),
                )
            })
            .collect();
        let scored = score(&slate);
        let session = Uuid::new_v4();
        let explored = (0..40)
            .filter(|seq| {
                select(&scored, 0.15, seed_for(session, *seq))
                    .expect("a choice")
                    .exploration
            })
            .count();
        assert!(
            (3..=9).contains(&explored),
            "{explored} of 40 were exploration picks, outside the spec's 3..=9"
        );
    }

    #[test]
    fn the_same_seed_replays_the_same_choice() {
        let slate: Vec<(AgendaItem, Inputs)> = (0..5)
            .map(|n| (item(&format!("q{n}"), Source::CharterSeed), inputs(f64::from(n) + 1.0, 1.0, 1.0, 1.0)))
            .collect();
        let scored = score(&slate);
        let session = Uuid::new_v4();
        for seq in 0..10 {
            let a = select(&scored, 0.3, seed_for(session, seq)).expect("a choice");
            let b = select(&scored, 0.3, seed_for(session, seq)).expect("a choice");
            assert_eq!(a, b, "the draw is not reproducible, so its propensity is a claim");
        }
        // And two sessions do not march in step.
        let other = Uuid::new_v4();
        let mine: Vec<Uuid> = (0..10)
            .map(|s| select(&scored, 0.3, seed_for(session, s)).expect("a").chosen)
            .collect();
        let theirs: Vec<Uuid> = (0..10)
            .map(|s| select(&scored, 0.3, seed_for(other, s)).expect("a").chosen)
            .collect();
        assert_ne!(mine, theirs);
    }

    #[test]
    fn the_propensities_of_the_two_branches_sum_to_one() {
        let slate: Vec<(AgendaItem, Inputs)> = (0..4)
            .map(|n| (item(&format!("q{n}"), Source::CharterSeed), inputs(f64::from(n) + 1.0, 1.0, 1.0, 1.0)))
            .collect();
        let scored = score(&slate);
        let share = 0.2;
        // Greedy gets 1 - share; each of the three detours gets share/3.
        let greedy = select(&scored, share, 0).expect("a choice");
        let mut total = 0.0;
        if greedy.exploration {
            total += greedy.propensity * 3.0;
        } else {
            total += greedy.propensity;
        }
        // Whichever branch this seed took, the mixture is a distribution.
        let detour_mass = share;
        let greedy_mass = 1.0 - share;
        assert!((detour_mass + greedy_mass - 1.0).abs() < 1e-12);
        assert!(total > 0.0);
    }

    #[test]
    fn a_signal_nobody_measures_leaves_the_product_alone_and_says_so() {
        let mut i = inputs(2.0, 1.0, 1.0, 4.0);
        i.cost = Signal::not_estimated();
        let slate = vec![(item("q", Source::CharterSeed), i)];
        let scored = score(&slate);
        assert_eq!(scored[0].not_estimated(), vec!["cost"]);
        // value 2 × novelty 1 × feasibility 1 / cost 1.
        assert!((scored[0].raw.total - 2.0).abs() < 1e-12);
        // And the candidate set carries it, because that is where a later
        // reader looks.
        let set = candidate_set(&scored);
        assert_eq!(set[0]["provenance"]["cost"], "not_estimated");
        assert_eq!(set[0]["not_estimated"][0], "cost");
        // The rationale says it out loud rather than presenting a bare number.
        let s = select(&scored, 0.15, 7).expect("a choice");
        assert!(s.rationale.contains("not estimated: cost"), "{}", s.rationale);
    }

    #[test]
    fn percentiles_are_ranks_and_ties_do_not_get_ordered_by_accident() {
        assert_eq!(percentiles(&[]), Vec::<f64>::new());
        assert_eq!(percentiles(&[5.0]), vec![1.0]);
        let p = percentiles(&[1.0, 3.0, 3.0, 7.0]);
        assert!((p[3] - 1.0).abs() < 1e-12, "the largest ranks 1.0");
        assert!((p[1] - p[2]).abs() < 1e-12, "ties share a rank");
        assert!(p[0] < p[1]);
        assert!(p.iter().all(|v| *v > 0.0 && *v <= 1.0));
    }

    #[test]
    fn one_very_expensive_item_does_not_flatten_the_slate() {
        // The argument for normalising cost: without it, a $9,000 item makes
        // every other total look identical next to it.
        let slate = vec![
            (item("cheap", Source::CharterSeed), inputs(1.0, 1.0, 1.0, 1.0)),
            (item("also cheap", Source::CharterSeed), inputs(1.0, 1.0, 1.0, 2.0)),
            (item("enormous", Source::CharterSeed), inputs(1.0, 1.0, 1.0, 9000.0)),
        ];
        let scored = score(&slate);
        let cheap = scored[0].normalised.total;
        let also = scored[1].normalised.total;
        assert!(
            cheap > also,
            "the cheaper of two similar items should still rank above the other"
        );
        assert!(
            (cheap / also) < 10.0,
            "the ratio between two ordinary items should not depend on an outlier"
        );
    }

    #[test]
    fn novelty_decays_with_how_often_the_question_has_been_asked() {
        let fresh = novelty_from_prior(0);
        assert!((fresh.raw - 1.0).abs() < 1e-12);
        assert_eq!(fresh.provenance, Provenance::Measured { n: 0 });
        let asked = novelty_from_prior(20);
        assert!(asked.raw < fresh.raw);
        assert!(asked.raw > 0.0, "a repeated question is worth less, never nothing");
        assert!(novelty_from_prior(200).raw < asked.raw);
        // Monotone, which is the only property the ranking depends on.
        let mut last = f64::INFINITY;
        for n in 0..50 {
            let v = novelty_from_prior(n).raw;
            assert!(v <= last);
            last = v;
        }
    }

    #[test]
    fn an_empty_or_wholly_infeasible_slate_chooses_nothing() {
        assert!(select(&[], 0.15, 1).is_none());
        let slate = vec![(item("no", Source::CharterSeed), inputs(1.0, 1.0, 0.0, 1.0))];
        let scored = score(&slate);
        assert!(
            select(&scored, 0.15, 1).is_none(),
            "nothing to choose is a different answer from choosing badly"
        );
    }
}
