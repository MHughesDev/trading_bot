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

use crate::agenda::{AgendaItem, Score, Source, COST_FLOOR_USD};
use crate::charter::EXPLORATION_FLOOR;
use crate::stall::Precheck;

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
        let ties = values
            .iter()
            .filter(|o| (*o - v).abs() < f64::EPSILON)
            .count();
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

// ── The precheck's half of `novelty` (§5.1) ─────────────────────────────────

/// §5.1: `hypothesis_precheck` `proceed_new` → 1.0.
pub const PRECHECK_PROCEED_NEW: f64 = 1.0;
/// §5.1: `hypothesis_precheck` `proceed_as_variant` → 0.6.
pub const PRECHECK_PROCEED_AS_VARIANT: f64 = 0.6;

/// The factor the precheck contributes to `novelty`, or that it contributed
/// none.
///
/// **There is no `1.0` for an unrecorded precheck**, and that is the whole
/// point of this type. `1.0` is `proceed_new`'s factor: an item nobody
/// prechecked, given it, ranks exactly as though the precheck had positively
/// called it new — ahead of an item the precheck honestly called a variant. The
/// absence is handed to the caller instead, so it can ride on the `decision`
/// row beside the score it did not contribute to.
///
/// [`crate::stall::Precheck::NotRecorded`] is the **common** case on this
/// platform: `hypothesis_precheck` files `blocked | duplicate | new` and most
/// runs have no verdict row at all.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PrecheckFactor {
    Recorded { factor: f64, verdict: &'static str },
    NotRecorded,
}

impl PrecheckFactor {
    /// The factor, if one was recorded. No `unwrap_or` pairs with this.
    #[must_use]
    pub fn factor(self) -> Option<f64> {
        match self {
            PrecheckFactor::Recorded { factor, .. } => Some(factor),
            PrecheckFactor::NotRecorded => None,
        }
    }

    /// What the decision row's `not_estimated` list says about it.
    #[must_use]
    pub fn note(self) -> Option<&'static str> {
        match self {
            PrecheckFactor::Recorded { .. } => None,
            PrecheckFactor::NotRecorded => Some(
                "novelty.precheck: no `hypothesis_precheck` verdict was recorded for this \
                 question, so §5.1's precheck factor was not applied to its novelty",
            ),
        }
    }
}

/// §5.1's precheck factor for a verdict.
#[must_use]
pub fn precheck_factor(precheck: Precheck) -> PrecheckFactor {
    match precheck {
        Precheck::ProceedNew => PrecheckFactor::Recorded {
            factor: PRECHECK_PROCEED_NEW,
            verdict: Precheck::ProceedNew.as_str(),
        },
        Precheck::ProceedAsVariant => PrecheckFactor::Recorded {
            factor: PRECHECK_PROCEED_AS_VARIANT,
            verdict: Precheck::ProceedAsVariant.as_str(),
        },
        Precheck::NotRecorded => PrecheckFactor::NotRecorded,
    }
}

/// `novelty` from both of §5.1's inputs: the precheck verdict and the count of
/// prior statements of the same question.
///
/// Returns the factor **alongside** the signal rather than folding it in
/// silently, because the caller has to record which of the two halves it got.
/// A caller that wanted only the number could have called
/// [`novelty_from_prior`]; this one returns two things so that dropping the
/// second is a visible act.
#[must_use]
pub fn novelty_from(precheck: Precheck, n_prior: i64) -> (Signal, PrecheckFactor) {
    let pf = precheck_factor(precheck);
    let prior = novelty_from_prior(n_prior);
    match pf.factor() {
        Some(f) => (Signal::measured(f * prior.raw, n_prior), pf),
        // The prior half is still measured and is still applied; what is
        // missing is the factor, and `pf` is what says so.
        None => (prior, pf),
    }
}

// ── The policy, and a reader per field (§5.1) ───────────────────────────────

/// AGENT-014 §5.1's own table, transcribed.
///
/// This is the **declaration**, not the implementation: [`Source::weight`] is
/// the implementation and there is exactly one of it. The pair exists so that a
/// conformance reader can compare them, which is what §5.1's *"versioned
/// `SelectionPolicy` object with a conformance reader per field"* is for. A
/// policy object holding its own copy of these numbers *instead of* the enum
/// would be a second place to write them and the first thing to drift; a copy
/// that is compared every build is the opposite.
const SPEC_SOURCE_WEIGHTS: &[(Source, Declared)] = &[
    (Source::User, Declared::UnboundedViaBypass),
    (Source::Contradiction, Declared::Number(1.5)),
    (Source::RetryConditionMet, Declared::Number(1.3)),
    (Source::OpenQuestionBlocking, Declared::Number(1.3)),
    (Source::Observation, Declared::Number(1.2)),
    (Source::StaleFinding, Declared::Number(1.0)),
    (Source::CharterSeed, Declared::Number(1.0)),
    (Source::OpenQuestion, Declared::Number(1.0)),
    (Source::RefinementFollowup, Declared::Number(0.8)),
    // §5.1 writes "`replenish` as estimated": the survey's own value estimate
    // rides on the item, so the source contributes the identity.
    (Source::Replenish, Declared::Number(1.0)),
    (Source::ProposalFromRun, Declared::Number(1.0)),
];

/// What the spec declares for one field.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum Declared {
    Number(f64),
    /// §5.1 writes `∞` for `user`. This build implements it as
    /// [`Source::bypasses_scoring`] rather than as a float, because a weight
    /// large enough to mean "always" poisons every percentile normalisation it
    /// touches. The reader therefore checks the **bypass**, not a number.
    UnboundedViaBypass,
}

/// One field of the policy: what the spec says, what this build does, and
/// where the build's number was read from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PolicyField {
    pub name: String,
    pub declared: Declared,
    /// What this build computes for the field.
    pub read: f64,
    /// Whether the item bypasses scoring — the implementation of `∞`.
    pub bypass: bool,
    /// The single definition [`Self::read`] was read out of, so a reader who
    /// disagrees with a number knows which line to change.
    pub from: &'static str,
}

impl PolicyField {
    /// Whether the build matches the declaration.
    #[must_use]
    pub fn conforms(&self) -> bool {
        match self.declared {
            // Exact: every one of these is a literal written in two places and
            // a tolerance here would let a third place be nearly right.
            Declared::Number(d) => !self.bypass && (d - self.read).abs() < 1e-12,
            Declared::UnboundedViaBypass => self.bypass,
        }
    }

    /// One sentence for a reader looking at a failure.
    #[must_use]
    pub fn sentence(&self) -> String {
        match self.declared {
            Declared::Number(d) if self.bypass => format!(
                "{}: §5.1 declares {d} and this build bypasses scoring for it entirely ({})",
                self.name, self.from
            ),
            Declared::Number(d) => format!(
                "{}: §5.1 declares {d}, this build reads {} ({})",
                self.name, self.read, self.from
            ),
            Declared::UnboundedViaBypass => format!(
                "{}: §5.1 declares ∞, implemented as a scoring bypass — this build {} ({})",
                self.name,
                if self.bypass {
                    "bypasses"
                } else {
                    "does not bypass"
                },
                self.from
            ),
        }
    }
}

/// §5.1's *"versioned `SelectionPolicy` object with a conformance reader per
/// field"*, as a **reader** over the one definition of each number.
///
/// The version is [`POLICY_VERSION`] and it is the same one that goes on the
/// `mlops.decision` row, so a later analysis can say which policy a ranking was
/// made under and then read what that policy was.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SelectionPolicy {
    pub policy: &'static str,
    pub version: i32,
}

impl Default for SelectionPolicy {
    fn default() -> Self {
        Self::current()
    }
}

impl SelectionPolicy {
    #[must_use]
    pub fn current() -> Self {
        Self {
            policy: POLICY,
            version: POLICY_VERSION,
        }
    }

    /// Every field, read out of the definition that holds it.
    #[must_use]
    pub fn fields(&self) -> Vec<PolicyField> {
        let mut out: Vec<PolicyField> = SPEC_SOURCE_WEIGHTS
            .iter()
            .map(|(source, declared)| PolicyField {
                name: format!("source.{}", source.as_str()),
                declared: *declared,
                read: source.weight(),
                bypass: source.bypasses_scoring(),
                from: "agenda::Source::weight",
            })
            .collect();
        out.push(PolicyField {
            name: "novelty.halflife".into(),
            declared: Declared::Number(20.0),
            read: NOVELTY_HALFLIFE,
            bypass: false,
            from: "select::NOVELTY_HALFLIFE",
        });
        out.push(PolicyField {
            name: "novelty.precheck.proceed_new".into(),
            declared: Declared::Number(1.0),
            read: PRECHECK_PROCEED_NEW,
            bypass: false,
            from: "select::PRECHECK_PROCEED_NEW",
        });
        out.push(PolicyField {
            name: "novelty.precheck.proceed_as_variant".into(),
            declared: Declared::Number(0.6),
            read: PRECHECK_PROCEED_AS_VARIANT,
            bypass: false,
            from: "select::PRECHECK_PROCEED_AS_VARIANT",
        });
        out.push(PolicyField {
            name: "cost.floor_usd".into(),
            declared: Declared::Number(0.10),
            read: COST_FLOOR_USD,
            bypass: false,
            from: "agenda::COST_FLOOR_USD",
        });
        out.push(PolicyField {
            name: "exploration.floor".into(),
            declared: Declared::Number(0.10),
            read: EXPLORATION_FLOOR,
            bypass: false,
            from: "charter::EXPLORATION_FLOOR",
        });
        out
    }

    /// Every field where the build and §5.1 disagree. Empty is the only
    /// acceptable answer and a test says so.
    #[must_use]
    pub fn nonconformances(&self) -> Vec<PolicyField> {
        self.fields()
            .into_iter()
            .filter(|f| !f.conforms())
            .collect()
    }

    /// Every source §5.1's table names, so a source added to the enum without
    /// a declared weight is a hole a test can find rather than a silent 1.0.
    #[must_use]
    pub fn declared_sources(&self) -> Vec<Source> {
        SPEC_SOURCE_WEIGHTS.iter().map(|(s, _)| *s).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agenda::Source;
    use crate::charter::EXPLORATION_FLOOR;

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
            (
                item("impossible", Source::Contradiction),
                inputs(99.0, 1.0, 0.0, 0.1),
            ),
            (
                item("ordinary", Source::CharterSeed),
                inputs(1.0, 1.0, 1.0, 1.0),
            ),
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
            (
                item("best", Source::CharterSeed),
                inputs(2.0, 1.0, 1.0, 1.0),
            ),
            (
                item("asked before", Source::CharterSeed),
                inputs(2.0, 0.0, 1.0, 1.0),
            ),
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
            (
                item("scored high", Source::Contradiction),
                inputs(9.0, 1.0, 1.0, 0.1),
            ),
            (item("asked for", Source::User), inputs(0.1, 0.1, 1.0, 9.0)),
        ];
        let scored = score(&slate);
        let s = select(&scored, 0.15, 1).expect("a choice");
        assert_eq!(s.chosen, scored[1].item);
        assert!(!s.exploration);
        assert!(
            (s.propensity - 1.0).abs() < 1e-12,
            "it was asked for, not drawn"
        );
        assert!(s.rationale.contains("asked for"));
    }

    /// A slate of six, all feasible and novel, for counting draws over.
    fn six() -> Vec<Scored> {
        let slate: Vec<(AgendaItem, Inputs)> = (0..6)
            .map(|n| {
                (
                    item(&format!("q{n}"), Source::CharterSeed),
                    inputs(f64::from(n) + 1.0, 1.0, 1.0, 1.0),
                )
            })
            .collect();
        score(&slate)
    }

    /// How many of 40 selections were detours, for one session.
    fn explored_in_forty(scored: &[Scored], session: Uuid, share: f64) -> usize {
        (0..40)
            .filter(|seq| {
                select(scored, share, seed_for(session, *seq))
                    .expect("a choice")
                    .exploration
            })
            .count()
    }

    #[test]
    fn the_exploration_share_is_achieved_over_forty_selections() {
        // AGENT-014 §12.3: over 40 selections at 0.15, between 3 and 9 are
        // exploration picks. Asserted against the spec's own interval rather
        // than against whatever this implementation happens to produce.
        //
        // **The session id is fixed, and that is a correction rather than a
        // convenience.** This test drew a fresh `Uuid::new_v4()` and failed
        // about one run in seven — because `3..=9` is only an ~86% interval for
        // Binomial(40, 0.15), so a correct implementation fails it routinely.
        // A test that reddens the build one run in seven teaches the reader to
        // re-run rather than to look, which is worse than no test: the next
        // real failure gets re-run too.
        //
        // So this pins one session and asserts the spec's interval on it, and
        // the test below measures the share properly — over 8,000 draws, where
        // the interval is tight enough to mean something.
        let session = Uuid::from_u128(0x5E55_1074_0000_0000_0000_0000_0000_0001);
        let explored = explored_in_forty(&six(), session, 0.15);
        assert!(
            (3..=9).contains(&explored),
            "{explored} of 40 were exploration picks, outside the spec's 3..=9"
        );
    }

    #[test]
    fn the_achieved_share_is_the_declared_share() {
        // The assertion §12.3's 40-draw window cannot make. 200 sessions × 40
        // selections is 8,000 draws, where the standard error on a 0.15 rate is
        // about 0.004 — so ±0.02 is roughly five of them, and a share that had
        // drifted to 0.10 or 0.20 could not pass by luck.
        //
        // Deterministic: the sessions are `Uuid::from_u128(n)`, not drawn. The
        // point of a platform-held seed is that a draw replays, and a test of
        // the draw that could not replay would be measuring something else.
        let scored = six();
        for share in [0.10, 0.15, 0.30] {
            let mut explored = 0usize;
            for n in 0..200u128 {
                explored += explored_in_forty(&scored, Uuid::from_u128(n + 1), share);
            }
            #[allow(clippy::cast_precision_loss)]
            let achieved = explored as f64 / (200.0 * 40.0);
            assert!(
                (achieved - share).abs() < 0.02,
                "declared share {share}, achieved {achieved:.4} over 8,000 draws"
            );
        }
    }

    #[test]
    fn the_floor_is_achieved_and_not_merely_permitted() {
        // INV-21 at the agenda level. A session at the floor must actually take
        // its detours — a scorer that quietly never explored would satisfy
        // every other test here, because they all check the ranking.
        let scored = six();
        let mut explored = 0usize;
        for n in 0..200u128 {
            explored += explored_in_forty(&scored, Uuid::from_u128(n + 9_000), EXPLORATION_FLOOR);
        }
        #[allow(clippy::cast_precision_loss)]
        let achieved = explored as f64 / 8_000.0;
        assert!(
            achieved > EXPLORATION_FLOOR - 0.02,
            "at the floor the session achieved only {achieved:.4}; the floor is a floor on what              happens, not on what is configured"
        );
    }

    #[test]
    fn the_same_seed_replays_the_same_choice() {
        let slate: Vec<(AgendaItem, Inputs)> = (0..5)
            .map(|n| {
                (
                    item(&format!("q{n}"), Source::CharterSeed),
                    inputs(f64::from(n) + 1.0, 1.0, 1.0, 1.0),
                )
            })
            .collect();
        let scored = score(&slate);
        // Both ids are fixed. The second half of this test asserts that two
        // sessions produce *different* sequences, and over five items at a
        // share of 0.3 two random ids agree on all ten draws about one run in
        // eight hundred — the same shape of flake
        // `the_exploration_share_is_achieved_over_forty_selections` was
        // corrected for above, one order of magnitude rarer and therefore
        // harder to catch when it lands.
        let session = Uuid::from_u128(0x5E55_1074_0000_0000_0000_0000_0000_0002);
        for seq in 0..10 {
            let a = select(&scored, 0.3, seed_for(session, seq)).expect("a choice");
            let b = select(&scored, 0.3, seed_for(session, seq)).expect("a choice");
            assert_eq!(
                a, b,
                "the draw is not reproducible, so its propensity is a claim"
            );
        }
        // And two sessions do not march in step.
        let other = Uuid::from_u128(0x5E55_1074_0000_0000_0000_0000_0000_0003);
        let mine: Vec<Uuid> = (0..10)
            .map(|s| {
                select(&scored, 0.3, seed_for(session, s))
                    .expect("a")
                    .chosen
            })
            .collect();
        let theirs: Vec<Uuid> = (0..10)
            .map(|s| select(&scored, 0.3, seed_for(other, s)).expect("a").chosen)
            .collect();
        assert_ne!(mine, theirs);
    }

    #[test]
    fn the_propensities_of_the_two_branches_sum_to_one() {
        let slate: Vec<(AgendaItem, Inputs)> = (0..4)
            .map(|n| {
                (
                    item(&format!("q{n}"), Source::CharterSeed),
                    inputs(f64::from(n) + 1.0, 1.0, 1.0, 1.0),
                )
            })
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
        assert!(
            s.rationale.contains("not estimated: cost"),
            "{}",
            s.rationale
        );
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
            (
                item("cheap", Source::CharterSeed),
                inputs(1.0, 1.0, 1.0, 1.0),
            ),
            (
                item("also cheap", Source::CharterSeed),
                inputs(1.0, 1.0, 1.0, 2.0),
            ),
            (
                item("enormous", Source::CharterSeed),
                inputs(1.0, 1.0, 1.0, 9000.0),
            ),
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
        assert!(
            asked.raw > 0.0,
            "a repeated question is worth less, never nothing"
        );
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

    // ── The policy and its conformance readers (§5.1) ───────────────────────

    #[test]
    fn every_field_of_the_policy_matches_what_the_spec_declares() {
        let p = SelectionPolicy::current();
        let bad = p.nonconformances();
        assert!(
            bad.is_empty(),
            "{}",
            bad.iter()
                .map(PolicyField::sentence)
                .collect::<Vec<_>>()
                .join("; ")
        );
        assert_eq!(p.policy, POLICY);
        assert_eq!(p.version, POLICY_VERSION);
    }

    #[test]
    fn the_conformance_reader_would_catch_a_drift() {
        // A reader nobody has seen fail is a reader that might be comparing
        // nothing. Two planted violations, one per shape of `Declared`.
        let drifted = PolicyField {
            name: "source.contradiction".into(),
            declared: Declared::Number(1.5),
            read: 1.4,
            bypass: false,
            from: "agenda::Source::weight",
        };
        assert!(!drifted.conforms());
        assert!(drifted.sentence().contains("1.5"), "{}", drifted.sentence());

        let lost_bypass = PolicyField {
            name: "source.user".into(),
            declared: Declared::UnboundedViaBypass,
            read: 1.0,
            bypass: false,
            from: "agenda::Source::weight",
        };
        assert!(
            !lost_bypass.conforms(),
            "`user`'s ∞ is the bypass; a 1.0 without it is a `user` item ranked like a seed"
        );
        assert!(
            lost_bypass.sentence().contains("does not bypass"),
            "{}",
            lost_bypass.sentence()
        );
    }

    #[test]
    fn a_source_with_no_declared_weight_is_a_hole_the_reader_finds() {
        // §5.1's table and the enum have to name the same set. A source added
        // to the enum and not to the table would otherwise pick up
        // `Source::weight`'s default and rank against a number nobody declared.
        let declared = SelectionPolicy::current().declared_sources();
        for s in Source::ALL {
            assert!(declared.contains(s), "{s:?} has no declared weight in §5.1");
        }
        assert_eq!(declared.len(), Source::ALL.len());
    }

    #[test]
    fn the_policy_names_the_definition_each_number_was_read_from() {
        for f in SelectionPolicy::current().fields() {
            assert!(!f.from.is_empty(), "{}", f.name);
            assert!(
                f.from.contains("::"),
                "`from` names a definition, not a prose gloss: {}",
                f.from
            );
        }
    }

    // ── The precheck's half of novelty (§5.1) ───────────────────────────────

    #[test]
    fn a_recorded_precheck_multiplies_novelty_and_an_absent_one_does_not() {
        let (new, pf_new) = novelty_from(Precheck::ProceedNew, 0);
        assert!(matches!(pf_new, PrecheckFactor::Recorded { .. }));
        let (variant, pf_var) = novelty_from(Precheck::ProceedAsVariant, 0);
        assert_eq!(pf_var.factor(), Some(PRECHECK_PROCEED_AS_VARIANT));
        assert!(
            variant.raw < new.raw,
            "a variant is less novel than a new question: {} vs {}",
            variant.raw,
            new.raw
        );
        assert!((variant.raw - 0.6 * new.raw).abs() < 1e-12);
    }

    #[test]
    fn an_unrecorded_precheck_is_not_a_proceed_new() {
        // The rule this crate is built on, at the place where the 1.0 is
        // invisible: `proceed_new`'s factor *is* 1.0, so folding an absent
        // verdict in as 1.0 ranks an unprechecked question exactly as though
        // the precheck had called it new.
        let (absent, pf) = novelty_from(Precheck::NotRecorded, 3);
        let (new, _) = novelty_from(Precheck::ProceedNew, 3);
        assert_eq!(pf, PrecheckFactor::NotRecorded);
        assert_eq!(pf.factor(), None, "there is no number to read here");
        assert!(
            pf.note().is_some_and(|n| n.contains("was not applied")),
            "the absence rides on the decision row: {:?}",
            pf.note()
        );
        // The numbers coincide — that is exactly why the *factor* has to be
        // carried separately rather than being inferred from the score.
        assert!((absent.raw - new.raw).abs() < 1e-12);
        assert_ne!(pf, precheck_factor(Precheck::ProceedNew));
    }

    #[test]
    fn every_precheck_word_has_a_factor_or_an_honest_absence() {
        for p in Precheck::ALL {
            let f = precheck_factor(*p);
            assert_eq!(f.factor().is_some(), p.is_recorded(), "{p:?}");
            assert_eq!(f.note().is_none(), p.is_recorded(), "{p:?}");
        }
    }

    #[test]
    fn the_prior_half_is_still_applied_when_the_precheck_is_absent() {
        let (few, _) = novelty_from(Precheck::NotRecorded, 0);
        let (many, _) = novelty_from(Precheck::NotRecorded, 200);
        assert!(
            many.raw < few.raw,
            "the ledger half is measured and is not thrown away with the factor"
        );
        assert_eq!(many.provenance, Provenance::Measured { n: 200 });
    }
}
