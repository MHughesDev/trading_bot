//! The stall detector (AGENT-014 P5, §7.1; Set S task S-11.8).
//!
//! A research programme that only makes ad hoc adjustments and produces no
//! novel results is stalled, whatever its dashboards say. That is Lakatos's
//! degenerating problemshift, and it is the one failure a session cannot notice
//! from inside a run: every individual run here is fine. It finished, it spent
//! its lease, it wrote a verdict. It is the *sequence* that has stopped meaning
//! anything, and nothing but a reader of the sequence can say so.
//!
//! So this is the thing a human running the programme would have noticed over
//! lunch, written down as four rules with numbers in them. When one trips, the
//! session stops and hands a human a diagnosis — which rule, which items, and
//! what amendment would answer it. It does not quietly carry on spending a day
//! budget on motion, and it does not adjust itself: a detector that could fix
//! its own stall is a detector that would fix it by lowering the bar.
//!
//! # The suggestion that is never offered
//!
//! [`Amendment`] has no variant that can carry an effect size, and that absence
//! is the point. `delta_practical` is the user's number — it is what they said
//! would change a decision — and "your programme is stalling; try caring about
//! smaller effects" is the one suggestion that would let a stalling programme
//! lower the bar until it passed. Every other amendment here changes *what is
//! searched*; none changes *what counts as a result*. The enum is closed, its
//! variants carry strings and sources rather than numbers, and
//! `no_amendment_can_carry_an_effect_size` will not compile past a new variant
//! that changes it.
//!
//! # Why this is a function over a slice
//!
//! [`detect`] takes finished-run records and returns a diagnosis. No database,
//! no session store, no clock. The rules are the part that is a decision; the
//! reading of rows is plumbing, and mixing them would make the numbers in §7.1
//! testable only against a live Postgres. [`crate::select`] and [`crate::fold`]
//! are shaped this way for the same reason.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::agenda::Source;

/// The smallest window the detector will look at (AGENT-014 §7.1's `max(6, …)`).
pub const MIN_WINDOW: usize = 6;

/// The share of a session's runs the window covers once it is longer than
/// [`MIN_WINDOW`].
pub const WINDOW_SHARE: f64 = 0.20;

/// Barren runs in the window that trip the streak.
pub const BARREN_IN_WINDOW: usize = 4;

/// Barren runs in a row that trip the streak on their own.
pub const BARREN_CONSECUTIVE: usize = 3;

/// Variants of one family with one failure class that count as circling.
pub const CIRCLING_REPEATS: usize = 2;

/// How far above the session's own median cost per knowledge unit the window
/// must sit before it counts as drift.
pub const COST_DRIFT_MULTIPLE: f64 = 3.0;

/// How many windows of history must exist before the median is worth comparing
/// against (AGENT-014 §7.1: "only once ≥ 2K runs exist").
pub const COST_DRIFT_WINDOWS_OF_HISTORY: usize = 2;

/// What `hypothesis_precheck` said about the question this run answered
/// (AGENT-013 §7.1).
///
/// `reject_duplicate` is **absent on purpose**: an item the precheck rejects is
/// marked `superseded` and never runs (AGENT-014 §6.5), so no finished run can
/// carry it. Modelling it here would invite a branch for a state that cannot
/// exist, and a reader would reasonably conclude that the platform sometimes
/// runs known duplicates.
///
/// # The common case is [`Precheck::NotRecorded`]
///
/// §7.1's three words are a vocabulary **nothing on this platform writes**.
/// `hypothesis_precheck` files `blocked | duplicate | new`, and most runs have
/// no `mlops.workflow_verdict` row for that workflow at all. So the absence of
/// a verdict is not the edge case here, it is the ordinary one, and before this
/// variant existed a caller joining for the verdict had to read `NULL` as one
/// of the two words — which meant either inventing a `proceed_new` nobody
/// recorded or manufacturing the `proceed_as_variant` that `circling` counts.
/// Neither is a reading the log supports, so there is a third word for having
/// no word.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Precheck {
    ProceedNew,
    ProceedAsVariant,
    /// No precheck verdict was recorded for this run. **Not a third opinion:**
    /// it is the absence of one, and no rule may read it as evidence. It
    /// cannot group under [`Rule::Circling`], which needs runs the precheck
    /// positively called variants.
    NotRecorded,
}

impl Precheck {
    /// Every word, absence last.
    pub const ALL: &'static [Precheck] = &[
        Precheck::ProceedNew,
        Precheck::ProceedAsVariant,
        Precheck::NotRecorded,
    ];

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Precheck::ProceedNew => "proceed_new",
            Precheck::ProceedAsVariant => "proceed_as_variant",
            Precheck::NotRecorded => "not_recorded",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|p| p.as_str() == s)
    }

    /// Whether a verdict was recorded at all.
    #[must_use]
    pub fn is_recorded(self) -> bool {
        !matches!(self, Precheck::NotRecorded)
    }
}

/// One finished run, as the detector needs to see it.
///
/// Deliberately not the run row and not the dossier: these are the eleven facts
/// §7.1's four rules read, and a record that carried more would invite a fifth
/// rule nobody specified.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FinishedRun {
    pub item: Uuid,
    pub run: Uuid,
    /// The hypothesis family the run tested, when one was recorded. Two of the
    /// four rules group on it.
    ///
    /// **`None` is not a family.** An `AgendaItem` carries no family, so it
    /// comes from the run's own hypothesis row and that row may not have one.
    /// A caller that filled the hole with a per-run name — `unrecorded:run:…` —
    /// was doing the right thing by accident: a name unique to one run cannot
    /// reach [`CIRCLING_REPEATS`] and cannot be the `sole_value` of a window.
    /// This says it instead of arranging it, so that a rule cannot come to
    /// depend on the arrangement, and so that no amendment can ever suggest
    /// excluding a family that has no name.
    pub family: Option<String>,
    /// The workflow that ran, so cost drift can name what got expensive.
    pub workflow: String,
    /// Every source that wanted the item — a list, because
    /// [`crate::agenda::offer`] merges sources onto one item and "came from
    /// `refinement_followup`" is false of a question a contradiction also
    /// wanted.
    pub sources: Vec<Source>,
    pub precheck: Precheck,
    /// `Yield::knowledge_units` (AGENT-013 §10), as the platform computed it.
    /// Zero is what the barren rule reads.
    pub knowledge_units: u32,
    /// Of those units, how many were `established` findings. The degenerating
    /// rule asks for these specifically: a programme can accumulate closed
    /// questions and still never predict anything new.
    pub findings_established: u32,
    /// The failure class this run recorded, if it recorded one. Circling groups
    /// on it.
    pub failure_class: Option<String>,
    /// Whether that class had not been seen for the subject before. A new class
    /// is a door nobody had closed, which is why it rescues a window from
    /// `degenerating`.
    pub new_failure_class: bool,
    /// What the run spent, in whatever unit the caller counts budget in.
    ///
    /// **The unit is the caller's and the rule does not care**, because cost
    /// drift is a *ratio* — the window against the session's own median — and a
    /// ratio has no units. That matters on this platform, where
    /// [`crate::select`] had to score cost in trials rather than dollars
    /// because `llm_usage.cost_usd` keys on the auth session rather than the
    /// run. `cost_drift_is_a_ratio_so_the_unit_cancels` holds it to that.
    pub cost: f64,
}

impl FinishedRun {
    /// A run that produced nothing, from the charter, costing one unit.
    ///
    /// The neutral case is the *barren* one rather than a productive one,
    /// because a fixture that has to opt into failure is a fixture where the
    /// interesting state is the one nobody wrote by accident.
    /// A blank family is no family: see [`Self::family`]. The precheck starts
    /// at [`Precheck::NotRecorded`] because that is what most runs on this
    /// platform actually have, and a fixture whose neutral state is a verdict
    /// nobody filed is a fixture that flatters the rules.
    #[must_use]
    pub fn new(item: Uuid, run: Uuid, family: &str) -> Self {
        Self::without_family(item, run).with_family(Some(family))
    }

    /// A run whose hypothesis recorded no family.
    ///
    /// The other constructor in terms of this one, rather than the reverse,
    /// because absence is the state that must be reachable without opting in.
    #[must_use]
    pub fn without_family(item: Uuid, run: Uuid) -> Self {
        Self {
            item,
            run,
            family: None,
            workflow: "investigate".to_string(),
            sources: vec![Source::CharterSeed],
            precheck: Precheck::NotRecorded,
            knowledge_units: 0,
            findings_established: 0,
            failure_class: None,
            new_failure_class: false,
            cost: 1.0,
        }
    }

    /// The family, if one was recorded — the shape a `LEFT JOIN` hands back.
    ///
    /// A blank name is read as no family, because a family called `""` groups
    /// every unnamed run together and that is the invented evidence
    /// [`Self::family`] exists to refuse.
    #[must_use]
    pub fn with_family(mut self, family: Option<&str>) -> Self {
        self.family = family
            .map(str::trim)
            .filter(|f| !f.is_empty())
            .map(ToString::to_string);
        self
    }

    /// Knowledge units, and how many of them were established findings.
    #[must_use]
    pub fn producing(mut self, knowledge_units: u32, findings_established: u32) -> Self {
        self.knowledge_units = knowledge_units;
        self.findings_established = findings_established;
        self
    }

    #[must_use]
    pub fn from_source(mut self, source: Source) -> Self {
        self.sources = vec![source];
        self
    }

    #[must_use]
    pub fn also_wanted_by(mut self, source: Source) -> Self {
        if !self.sources.contains(&source) {
            self.sources.push(source);
        }
        self
    }

    #[must_use]
    pub fn as_variant(mut self) -> Self {
        self.precheck = Precheck::ProceedAsVariant;
        self
    }

    #[must_use]
    pub fn failing(mut self, class: &str, class_is_new: bool) -> Self {
        self.failure_class = Some(class.to_string());
        self.new_failure_class = class_is_new;
        self
    }

    #[must_use]
    pub fn with_workflow(mut self, workflow: &str) -> Self {
        self.workflow = workflow.to_string();
        self
    }

    #[must_use]
    pub fn costing(mut self, cost: f64) -> Self {
        self.cost = cost;
        self
    }

    /// `knowledge_units = 0` — the barren rule's own words.
    ///
    /// Not `Yield::is_barren`, which also asks that a trial was spent. §7.1
    /// says `knowledge_units = 0` and means it: a run that produced nothing
    /// *without* spending a trial is still a run the session chose, started and
    /// waited for, and a streak of them is still a stalled programme.
    #[must_use]
    pub fn produced_nothing(&self) -> bool {
        self.knowledge_units == 0
    }

    /// Cost per knowledge unit, or `None` when there are no units.
    ///
    /// `None` rather than infinity and rather than the bare cost, for the reason
    /// `Yield::cost_per_knowledge_unit` gives one level down: a barren run has
    /// no cost *per unit*, and a finite number in that column is a number
    /// readers will average.
    #[must_use]
    pub fn cost_per_knowledge_unit(&self) -> Option<f64> {
        (self.knowledge_units > 0).then(|| self.cost / f64::from(self.knowledge_units))
    }

    fn only_from(&self, source: Source) -> bool {
        !self.sources.is_empty() && self.sources.iter().all(|s| *s == source)
    }
}

/// Which of §7.1's four rules tripped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Rule {
    BarrenStreak,
    Degenerating,
    Circling,
    CostDrift,
}

impl Rule {
    /// In the order §7.1 lists them, which is also the order a diagnosis reports
    /// them in.
    pub const ALL: &'static [Rule] = &[
        Rule::BarrenStreak,
        Rule::Degenerating,
        Rule::Circling,
        Rule::CostDrift,
    ];

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Rule::BarrenStreak => "barren_streak",
            Rule::Degenerating => "degenerating",
            Rule::Circling => "circling",
            Rule::CostDrift => "cost_drift",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|r| r.as_str() == s)
    }
}

/// A change to the charter that would answer the stall.
///
/// **Closed, and every variant changes what is searched rather than what counts
/// as a result.** There is no variant carrying an effect size and there must
/// never be one: see the module header. Amendments are suggestions in any case
/// — the session appends nothing and amends nothing; a human reads this and
/// decides (AGENT-014 P6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Amendment {
    /// Stop asking about this family. The strongest answer to circling.
    ExcludeFamily { family: String },
    /// Stop running this workflow, which is where the money went.
    ExcludeWorkflow { workflow: String },
    /// Look somewhere the charter currently does not.
    WidenScope { what: String },
    /// Let something other than this source fill the agenda.
    PreferSourcesOtherThan { source: Source },
}

impl Amendment {
    /// The kind word, for the digest and the desk.
    ///
    /// Exhaustive by construction: a new variant does not compile until it is
    /// named here, and naming it is the moment somebody has to justify it
    /// against the module header's rule.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Amendment::ExcludeFamily { .. } => "exclude_family",
            Amendment::ExcludeWorkflow { .. } => "exclude_workflow",
            Amendment::WidenScope { .. } => "widen_scope",
            Amendment::PreferSourcesOtherThan { .. } => "prefer_sources_other_than",
        }
    }

    /// One line a human can act on.
    #[must_use]
    pub fn text(&self) -> String {
        match self {
            Amendment::ExcludeFamily { family } => {
                format!("exclude the `{family}` family from the charter's scope")
            }
            Amendment::ExcludeWorkflow { workflow } => {
                format!("drop `{workflow}` from the charter's permitted workflows")
            }
            Amendment::WidenScope { what } => format!("widen the charter's scope: {what}"),
            Amendment::PreferSourcesOtherThan { source } => format!(
                "let something other than `{}` fill the agenda",
                source.as_str()
            ),
        }
    }
}

/// One rule, tripped, with everything a human needs to disagree with it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tripped {
    pub rule: Rule,
    /// The items involved. Never empty — a diagnosis that names no item is a
    /// diagnosis nobody can check, and [`detect`] never builds one.
    pub items: Vec<Uuid>,
    /// One sentence saying what was counted and against what threshold.
    pub what: String,
    pub suggestions: Vec<Amendment>,
}

/// Why the session stalled.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Diagnosis {
    /// Never empty. In [`Rule::ALL`]'s order, so two sessions tripping the same
    /// pair of rules report them the same way round.
    pub rules: Vec<Tripped>,
    /// `K` as it was actually applied.
    pub window: usize,
    /// How many runs were in the window — fewer than `window` early on.
    pub runs_considered: usize,
    /// How many runs the session has finished in total.
    pub runs_total: usize,
}

impl Diagnosis {
    /// The rule word the session's `stalled{diagnosis}` state carries.
    ///
    /// The first rule in §7.1's order, because the state column holds one word
    /// and the full list rides in the payload. `fold` reads this key out of the
    /// `session_stalled` event; [`Self::event_payload`] is what writes it.
    #[must_use]
    pub fn headline(&self) -> &'static str {
        self.rules.first().map_or("stalled", |t| t.rule.as_str())
    }

    /// Every item any rule named, deduplicated, in the order they were named.
    #[must_use]
    pub fn items(&self) -> Vec<Uuid> {
        let mut out: Vec<Uuid> = Vec::new();
        for t in &self.rules {
            for i in &t.items {
                if !out.contains(i) {
                    out.push(*i);
                }
            }
        }
        out
    }

    /// Every suggested amendment, deduplicated.
    #[must_use]
    pub fn amendments(&self) -> Vec<Amendment> {
        let mut out: Vec<Amendment> = Vec::new();
        for t in &self.rules {
            for a in &t.suggestions {
                if !out.contains(a) {
                    out.push(a.clone());
                }
            }
        }
        out
    }

    /// The payload of the `session_stalled` event.
    ///
    /// `diagnosis` is the word [`crate::fold::fold`] lifts into
    /// `SessionState::Stalled`; everything else is what the digest and the desk
    /// read. Written here rather than at the driver so the state word and the
    /// detail can never name different rules.
    #[must_use]
    pub fn event_payload(&self) -> serde_json::Value {
        serde_json::json!({
            "diagnosis": self.headline(),
            "window": self.window,
            "runs_considered": self.runs_considered,
            "runs_total": self.runs_total,
            "rules": self.rules,
            "items": self.items(),
            "suggestions": self.amendments(),
        })
    }
}

/// `K = max(6, 20% of runs)` (AGENT-014 §7.1).
///
/// Rounded up: a window that rounds down is a window narrower than the spec's,
/// and the whole point of the share is that a long session is judged over a
/// proportionate stretch rather than over six runs forever.
#[must_use]
pub fn window_size(n_runs: usize) -> usize {
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    let share = (n_runs as f64 * WINDOW_SHARE).ceil() as usize;
    share.max(MIN_WINDOW)
}

/// Evaluate §7.1's four rules over a session's finished runs.
///
/// `runs` is in chronological order, oldest first — the order the session log
/// finished them in. Returns `None` when nothing tripped, which is the ordinary
/// case and is not the same as "the session is healthy": it is "none of these
/// four rules has anything to say yet".
#[must_use]
pub fn detect(runs: &[FinishedRun]) -> Option<Diagnosis> {
    let total = runs.len();
    let k = window_size(total);
    let window = &runs[total.saturating_sub(k)..];
    if window.is_empty() {
        return None;
    }

    let mut rules: Vec<Tripped> = Vec::new();
    for rule in Rule::ALL {
        let tripped = match rule {
            Rule::BarrenStreak => barren_streak(window),
            Rule::Degenerating => degenerating(window, k),
            Rule::Circling => circling(window),
            Rule::CostDrift => cost_drift(runs, window, k),
        };
        if let Some(t) = tripped {
            debug_assert!(!t.items.is_empty(), "a tripped rule must name its items");
            rules.push(t);
        }
    }

    if rules.is_empty() {
        return None;
    }
    Some(Diagnosis {
        rules,
        window: k,
        runs_considered: window.len(),
        runs_total: total,
    })
}

/// `knowledge_units = 0` in ≥ 4 of the last K, or the last 3 consecutively.
fn barren_streak(window: &[FinishedRun]) -> Option<Tripped> {
    let barren: Vec<&FinishedRun> = window.iter().filter(|r| r.produced_nothing()).collect();
    let consecutive = window
        .iter()
        .rev()
        .take(BARREN_CONSECUTIVE)
        .filter(|r| r.produced_nothing())
        .count()
        == BARREN_CONSECUTIVE
        && window.len() >= BARREN_CONSECUTIVE;

    if barren.len() < BARREN_IN_WINDOW && !consecutive {
        return None;
    }

    // Every barren run is named, not the first four: the human is being asked to
    // look at a stretch of work, and a diagnosis that truncated it would be
    // asking them to take the rest on trust.
    let items: Vec<Uuid> = barren.iter().map(|r| r.item).collect();
    let what = if consecutive && barren.len() < BARREN_IN_WINDOW {
        format!(
            "the last {BARREN_CONSECUTIVE} runs in a row produced no knowledge units ({} of the \
             last {} in all)",
            barren.len(),
            window.len()
        )
    } else {
        format!(
            "{} of the last {} runs produced no knowledge units (the rule trips at \
             {BARREN_IN_WINDOW}, or at {BARREN_CONSECUTIVE} in a row)",
            barren.len(),
            window.len()
        )
    };

    let mut suggestions = vec![Amendment::WidenScope {
        what: "these runs were all inside it and none of them found anything".to_string(),
    }];
    if let Some(family) = sole_value(barren.iter().map(|r| r.family.clone())) {
        suggestions.push(Amendment::ExcludeFamily { family });
    }

    Some(Tripped {
        rule: Rule::BarrenStreak,
        items,
        what,
        suggestions,
    })
}

/// Every one of the last K from `refinement_followup`, none producing an
/// `established` finding or a new failure class.
///
/// **Only over a full window.** "Every one of the last K" is a universal
/// quantifier, and a universal quantifier over two runs is not evidence about a
/// programme — it is evidence that a session has run twice. So this rule waits
/// until K runs exist, which is the one place the partial window is refused.
fn degenerating(window: &[FinishedRun], k: usize) -> Option<Tripped> {
    if window.len() < k {
        return None;
    }
    let all_followup = window
        .iter()
        .all(|r| r.only_from(Source::RefinementFollowup));
    let nothing_new = window
        .iter()
        .all(|r| r.findings_established == 0 && !r.new_failure_class);
    if !all_followup || !nothing_new {
        return None;
    }

    let items: Vec<Uuid> = window.iter().map(|r| r.item).collect();
    let mut suggestions = vec![Amendment::PreferSourcesOtherThan {
        source: Source::RefinementFollowup,
    }];
    if let Some(family) = sole_value(window.iter().map(|r| r.family.clone())) {
        suggestions.push(Amendment::ExcludeFamily { family });
    }

    Some(Tripped {
        rule: Rule::Degenerating,
        items,
        what: format!(
            "all {} runs in the window followed up on an earlier run, and none established a \
             finding or closed a new failure class: the programme is adjusting, not predicting",
            window.len()
        ),
        suggestions,
    })
}

/// ≥ 2 of the last K `proceed_as_variant` of the same family with the same
/// failure class — the dedupe did its job and the policy overrode it.
fn circling(window: &[FinishedRun]) -> Option<Tripped> {
    let mut groups: BTreeMap<(String, String), Vec<Uuid>> = BTreeMap::new();
    for r in window {
        if r.precheck != Precheck::ProceedAsVariant {
            continue;
        }
        // A variant with no failure class is not circling: "the same failure
        // class" is the evidence that the variants keep dying the same death,
        // and grouping the classless ones together would invent that evidence.
        let Some(class) = r.failure_class.clone() else {
            continue;
        };
        // Nor is a run with no family. "Variants of one family" is the other
        // half of that evidence, and a `None` key would collect every unnamed
        // run into one group and call it a family — the exact fabrication the
        // caller's per-run sentinel name was avoiding by hand.
        let Some(family) = r.family.clone() else {
            continue;
        };
        groups.entry((family, class)).or_default().push(r.item);
    }

    let ((family, class), items) = groups
        .into_iter()
        .filter(|(_, items)| items.len() >= CIRCLING_REPEATS)
        .max_by_key(|(_, items)| items.len())?;

    Some(Tripped {
        rule: Rule::Circling,
        what: format!(
            "{} runs in the window were variants of `{family}` and all of them failed the same \
             way (`{class}`)",
            items.len()
        ),
        items,
        suggestions: vec![Amendment::ExcludeFamily { family }],
    })
}

/// Cost per knowledge unit over the window above `3×` the session's own median,
/// and only once `2K` runs exist.
///
/// **The gate is the rule's honesty.** With one window of history the "median"
/// is a median of a handful of numbers, and a median of three is a coin toss:
/// it would trip on the ordinary variance between an `investigate` and a
/// `replicate` and teach the human that the detector cries wolf. So the rule
/// says nothing at all until there are two windows to compare, and says nothing
/// rather than saying something weak.
///
/// **The median is taken over the runs *before* the window, and that is a
/// reading of §7.1 rather than its literal words.** "3× the session's own
/// median" can be read as a median over every run including the ones being
/// judged, and that reading is arithmetically inert at the exact moment the
/// gate opens: with `2K` runs the window is half of them, so the median falls
/// between the cheap half and the dear half, and `dear > 3 × (cheap + dear)/2`
/// has no solution for positive costs. A rule that provably cannot fire on the
/// first day it is allowed to is a rule whose gate is lying, so the baseline
/// here is the session's history — still the session's own, and not a
/// platform-wide figure. `the_baseline_is_the_history_because_a_window_inside_
/// its_own_median_can_never_drift` is the proof, kept as a test so the argument
/// is checkable rather than asserted.
fn cost_drift(all: &[FinishedRun], window: &[FinishedRun], k: usize) -> Option<Tripped> {
    if all.len() < COST_DRIFT_WINDOWS_OF_HISTORY * k {
        return None;
    }

    // The window's figure is the aggregate — total cost over total units — and
    // not the mean of the per-run ratios, which would let one cheap productive
    // run cancel out an expensive barren stretch.
    let window_units: u32 = window.iter().map(|r| r.knowledge_units).sum();
    if window_units == 0 {
        // Undefined, not infinite. A window that produced nothing at all is
        // already `barren_streak`'s business, and reporting it here as an
        // enormous cost per unit would be a second alarm about one fact.
        return None;
    }
    let window_cost: f64 = window.iter().map(|r| r.cost).sum();
    let window_cpku = window_cost / f64::from(window_units);

    let mut history: Vec<f64> = all[..all.len() - window.len()]
        .iter()
        .filter_map(FinishedRun::cost_per_knowledge_unit)
        .collect();
    let median = median(&mut history)?;
    if window_cpku <= COST_DRIFT_MULTIPLE * median {
        return None;
    }

    let items: Vec<Uuid> = window.iter().map(|r| r.item).collect();
    let dearest = dearest_workflow(window);
    let mut suggestions = Vec::new();
    if let Some(workflow) = dearest {
        suggestions.push(Amendment::ExcludeWorkflow { workflow });
    }
    suggestions.push(Amendment::WidenScope {
        what: "knowledge inside it has become expensive relative to what this session usually pays"
            .to_string(),
    });

    Some(Tripped {
        rule: Rule::CostDrift,
        items,
        what: format!(
            "the last {} runs cost {window_cpku:.2} per knowledge unit against a median of \
             {median:.2} over everything this session did before them — more than \
             {COST_DRIFT_MULTIPLE}×",
            window.len()
        ),
        suggestions,
    })
}

/// The workflow that took the largest share of the window's cost.
fn dearest_workflow(window: &[FinishedRun]) -> Option<String> {
    let mut by_workflow: BTreeMap<String, f64> = BTreeMap::new();
    for r in window {
        *by_workflow.entry(r.workflow.clone()).or_insert(0.0) += r.cost;
    }
    by_workflow
        .into_iter()
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(w, _)| w)
}

/// The one value they all share, or `None` if they do not all share one.
///
/// **One that was not recorded is not shared.** A single `None` in the window
/// sinks the whole answer, which is what keeps an `ExcludeFamily` amendment off
/// a window where some run's family is simply unknown: suggesting a human
/// exclude the family the *recorded* runs happen to agree on would be
/// suggesting it on evidence the window does not have.
fn sole_value<I: Iterator<Item = Option<String>>>(mut values: I) -> Option<String> {
    let first = values.next()??;
    values
        .all(|v| v.as_deref() == Some(first.as_str()))
        .then_some(first)
}

/// The median of a set of costs, or `None` when the set is empty.
fn median(values: &mut [f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mid = values.len() / 2;
    Some(if values.len() % 2 == 0 {
        (values[mid - 1] + values[mid]) / 2.0
    } else {
        values[mid]
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fold::{fold, EventKind, SessionEvent, SessionState};

    fn run(n: u8, family: &str) -> FinishedRun {
        FinishedRun::new(
            Uuid::from_u128(u128::from(n)),
            Uuid::from_u128(u128::from(n) + 1000),
            family,
        )
    }

    fn productive(n: u8, family: &str) -> FinishedRun {
        run(n, family).producing(2, 1)
    }

    /// [`detect`], with the one assertion that must hold on every fixture.
    ///
    /// Every test in this module goes through this rather than through `detect`
    /// directly, so "asserted on every fixture" is a property of the module and
    /// not of whoever wrote the last test. A fixture added tomorrow is covered
    /// the moment it calls this, and calling the bare `detect` is the thing a
    /// reviewer can see.
    fn detect_checked(runs: &[FinishedRun]) -> Option<Diagnosis> {
        let d = detect(runs);
        if let Some(d) = &d {
            the_effect_size_is_still_the_users(d);
        }
        d
    }

    /// The assertion the module header exists for, run on **every** fixture
    /// rather than once.
    ///
    /// Two nets. The structural one is the enforcement: an exhaustive match over
    /// [`Amendment`] that will not compile past a variant carrying a number, so
    /// the day somebody adds `LowerDeltaPractical { to: f64 }` this test is
    /// where they find out. The text scan is the second net, over fixtures this
    /// test owns, and it is only sound *because* it owns them — a family a user
    /// genuinely named `delta_neutral` would trip it, which is exactly why the
    /// structural check is the one that matters.
    fn the_effect_size_is_still_the_users(d: &Diagnosis) {
        assert_eq!(
            effect_size_violation(d),
            None,
            "the diagnosis reached for the effect size: lowering the bar until the programme \
             passes is the one amendment a stalling session may never suggest"
        );
    }

    /// The forbidden word this diagnosis used, if it used one.
    ///
    /// Split out from the assertion so that
    /// `the_guard_against_lowering_the_bar_would_catch_a_violation` can show it
    /// firing. A guard nobody has seen fail is a guard that might be matching
    /// nothing.
    fn effect_size_violation(d: &Diagnosis) -> Option<&'static str> {
        for a in d.amendments() {
            match a {
                Amendment::ExcludeFamily { .. }
                | Amendment::ExcludeWorkflow { .. }
                | Amendment::WidenScope { .. }
                | Amendment::PreferSourcesOtherThan { .. } => {}
            }
        }
        let rendered = serde_json::to_string(&d.event_payload()).expect("a diagnosis serialises")
            + &d.amendments()
                .iter()
                .map(Amendment::text)
                .collect::<Vec<_>>()
                .join(" ");
        let lowered = rendered.to_lowercase();
        ["delta_practical", "delta practical", "effect size", "δ"]
            .into_iter()
            .find(|forbidden| lowered.contains(forbidden))
    }

    #[test]
    fn the_guard_against_lowering_the_bar_would_catch_a_violation() {
        let planted = Diagnosis {
            rules: vec![Tripped {
                rule: Rule::BarrenStreak,
                items: vec![Uuid::nil()],
                what: "nothing found".into(),
                suggestions: vec![Amendment::WidenScope {
                    what: "or try a smaller delta_practical".into(),
                }],
            }],
            window: 6,
            runs_considered: 6,
            runs_total: 6,
        };
        assert_eq!(
            effect_size_violation(&planted),
            Some("delta_practical"),
            "the guard every fixture runs through does not actually catch anything"
        );
    }

    #[test]
    fn the_window_is_six_until_twenty_percent_is_larger() {
        assert_eq!(window_size(0), MIN_WINDOW);
        assert_eq!(window_size(6), MIN_WINDOW);
        assert_eq!(window_size(30), MIN_WINDOW, "20% of 30 is exactly 6");
        assert_eq!(window_size(31), 7, "rounded up, not down");
        assert_eq!(window_size(100), 20);
        // Monotone, which is the only property the rules depend on.
        let mut last = 0;
        for n in 0..500 {
            let k = window_size(n);
            assert!(k >= last, "the window shrank between {} and {n}", n - 1);
            last = k;
        }
    }

    #[test]
    fn six_barren_runs_trip_the_streak_and_name_all_six() {
        // AGENT-014 §12.7's own fixture.
        let runs: Vec<FinishedRun> = (0..6).map(|n| run(n, "momentum")).collect();
        let d = detect_checked(&runs).expect("six barren runs are a stall");
        assert_eq!(d.headline(), "barren_streak");
        assert_eq!(d.window, 6);
        assert_eq!(d.runs_considered, 6);

        let named = &d
            .rules
            .iter()
            .find(|t| t.rule == Rule::BarrenStreak)
            .expect("the barren rule tripped")
            .items;
        assert_eq!(named.len(), 6, "all six, not the first four");
        for r in &runs {
            assert!(named.contains(&r.item), "{} was not named", r.item);
        }
    }

    #[test]
    fn three_barren_in_a_row_trip_it_even_when_the_window_is_mostly_productive() {
        let mut runs: Vec<FinishedRun> = (0..5).map(|n| productive(n, "carry")).collect();
        runs.extend((5..8).map(|n| run(n, "carry")));
        let d = detect_checked(&runs).expect("three in a row is a stall");
        let t = d
            .rules
            .iter()
            .find(|t| t.rule == Rule::BarrenStreak)
            .expect("the barren rule tripped");
        assert_eq!(t.items.len(), 3);
        assert!(t.what.contains("in a row"), "{}", t.what);
    }

    #[test]
    fn a_productive_session_is_not_stalled() {
        let runs: Vec<FinishedRun> = (0..20)
            .map(|n| productive(n, if n % 2 == 0 { "momentum" } else { "carry" }))
            .collect();
        assert_eq!(
            detect_checked(&runs),
            None,
            "nothing in §7.1 has anything to say about a session that keeps finding things"
        );
    }

    #[test]
    fn two_barren_runs_are_not_a_streak() {
        let mut runs: Vec<FinishedRun> = (0..4).map(|n| productive(n, "carry")).collect();
        runs.extend((4..6).map(|n| run(n, "carry")));
        assert_eq!(
            detect_checked(&runs),
            None,
            "two is not four and not three in a row"
        );
    }

    #[test]
    fn refinement_only_runs_that_establish_nothing_are_degenerating() {
        // Productive in units — questions closed — but nothing established and
        // no new class, which is the distinction the rule is about.
        let runs: Vec<FinishedRun> = (0..6)
            .map(|n| {
                run(n, "momentum")
                    .from_source(Source::RefinementFollowup)
                    .producing(1, 0)
            })
            .collect();
        let d = detect_checked(&runs).expect("refinement-only with nothing established is a stall");
        let t = d
            .rules
            .iter()
            .find(|t| t.rule == Rule::Degenerating)
            .expect("the degenerating rule tripped");
        assert_eq!(t.items.len(), 6);
        assert!(
            d.amendments().contains(&Amendment::PreferSourcesOtherThan {
                source: Source::RefinementFollowup
            }),
            "the suggestion has to say where to look instead"
        );
        assert!(
            d.amendments().contains(&Amendment::ExcludeFamily {
                family: "momentum".into()
            }),
            "one family produced all of it, so it can be named"
        );
    }

    #[test]
    fn one_established_finding_rescues_the_window_from_degenerating() {
        let mut runs: Vec<FinishedRun> = (0..6)
            .map(|n| {
                run(n, "momentum")
                    .from_source(Source::RefinementFollowup)
                    .producing(1, 0)
            })
            .collect();
        runs[3] = runs[3].clone().producing(1, 1);
        let d = detect_checked(&runs);
        assert!(
            d.as_ref()
                .is_none_or(|d| d.rules.iter().all(|t| t.rule != Rule::Degenerating)),
            "a programme that predicted something is not degenerating"
        );

        // And a new failure class rescues it too, which is the other half of the
        // sentence and would be easy to drop.
        let mut runs: Vec<FinishedRun> = (0..6)
            .map(|n| {
                run(n, "momentum")
                    .from_source(Source::RefinementFollowup)
                    .producing(1, 0)
            })
            .collect();
        runs[5] = runs[5].clone().failing("regime_dependent", true);
        assert!(
            detect_checked(&runs)
                .as_ref()
                .is_none_or(|d| d.rules.iter().all(|t| t.rule != Rule::Degenerating)),
            "closing a door nobody had closed is a novel result"
        );
    }

    #[test]
    fn a_question_another_source_also_wanted_is_not_an_ad_hoc_adjustment() {
        let mut runs: Vec<FinishedRun> = (0..6)
            .map(|n| {
                run(n, "momentum")
                    .from_source(Source::RefinementFollowup)
                    .producing(1, 0)
            })
            .collect();
        runs[2] = runs[2].clone().also_wanted_by(Source::Contradiction);
        assert!(
            detect_checked(&runs)
                .as_ref()
                .is_none_or(|d| d.rules.iter().all(|t| t.rule != Rule::Degenerating)),
            "a contradiction wanted this question too, so the window is not all follow-up"
        );
    }

    #[test]
    fn degenerating_says_nothing_until_a_full_window_exists() {
        // Two follow-up runs satisfy "every one of the last K" vacuously if the
        // window is allowed to be partial. It is not.
        let runs: Vec<FinishedRun> = (0..2)
            .map(|n| {
                run(n, "momentum")
                    .from_source(Source::RefinementFollowup)
                    .producing(1, 0)
            })
            .collect();
        assert_eq!(
            detect_checked(&runs),
            None,
            "a universal quantifier over two runs is a statement about two runs"
        );
    }

    #[test]
    fn two_variants_of_one_family_failing_the_same_way_are_circling() {
        let mut runs: Vec<FinishedRun> = (0..4).map(|n| productive(n, "carry")).collect();
        runs.push(
            productive(4, "momentum")
                .as_variant()
                .failing("costs_eat_the_edge", false),
        );
        runs.push(
            productive(5, "momentum")
                .as_variant()
                .failing("costs_eat_the_edge", false),
        );
        let d = detect_checked(&runs).expect("two variants dying the same death is a stall");
        let t = d
            .rules
            .iter()
            .find(|t| t.rule == Rule::Circling)
            .expect("the circling rule tripped");
        assert_eq!(t.items.len(), 2);
        assert_eq!(
            d.amendments(),
            vec![Amendment::ExcludeFamily {
                family: "momentum".into()
            }]
        );
    }

    #[test]
    fn circling_needs_the_same_family_and_the_same_class() {
        let same_family_other_class: Vec<FinishedRun> = vec![
            productive(0, "momentum")
                .as_variant()
                .failing("costs_eat_the_edge", false),
            productive(1, "momentum")
                .as_variant()
                .failing("no_signal_out_of_sample", false),
            productive(2, "carry"),
            productive(3, "carry"),
        ];
        assert_eq!(detect_checked(&same_family_other_class), None);

        let other_family_same_class: Vec<FinishedRun> = vec![
            productive(0, "momentum")
                .as_variant()
                .failing("costs_eat_the_edge", false),
            productive(1, "carry")
                .as_variant()
                .failing("costs_eat_the_edge", false),
            productive(2, "carry"),
            productive(3, "carry"),
        ];
        assert_eq!(detect_checked(&other_family_same_class), None);

        // And a variant with no class recorded is not evidence of anything.
        let classless: Vec<FinishedRun> = vec![
            productive(0, "momentum").as_variant(),
            productive(1, "momentum").as_variant(),
            productive(2, "carry"),
            productive(3, "carry"),
        ];
        assert_eq!(detect_checked(&classless), None);
    }

    #[test]
    fn a_run_with_no_family_recorded_never_groups_with_another() {
        // The caller used to fill this hole with a name unique to the run, so
        // that two of them could not meet in a group. `None` has to do the same
        // thing without the arrangement: two variants that failed the same way
        // are circling *only* if the platform said which family they were.
        let unfamilied = |n: u8| {
            FinishedRun::without_family(
                Uuid::from_u128(u128::from(n)),
                Uuid::from_u128(u128::from(n) + 1000),
            )
            .producing(2, 1)
            .as_variant()
            .failing("costs_eat_the_edge", false)
        };
        let mut runs: Vec<FinishedRun> = (0..4).map(|n| productive(n, "carry")).collect();
        runs.push(unfamilied(4));
        runs.push(unfamilied(5));
        for r in &runs[4..] {
            assert_eq!(r.family, None, "the fixture recorded a family after all");
        }
        assert_eq!(
            detect_checked(&runs),
            None,
            "two runs of no family are not two runs of one family"
        );

        // And the same pair, with the family recorded, is the stall — so the
        // test above is about the absence and not about the fixture.
        let mut named = runs.clone();
        for r in &mut named[4..] {
            *r = r.clone().with_family(Some("momentum"));
        }
        let d = detect_checked(&named).expect("recorded families circle");
        assert_eq!(d.headline(), "circling");
    }

    #[test]
    fn an_unrecorded_family_is_never_suggested_as_an_exclusion() {
        // `sole_value` is what an `ExcludeFamily` amendment is built from, and
        // a window a human is told to exclude by name has to have a name.
        let barren_unfamilied: Vec<FinishedRun> = (0_u8..6)
            .map(|n| {
                FinishedRun::without_family(
                    Uuid::from_u128(u128::from(n)),
                    Uuid::from_u128(u128::from(n) + 1000),
                )
            })
            .collect();
        let d = detect_checked(&barren_unfamilied).expect("six barren runs are still a stall");
        assert_eq!(d.headline(), "barren_streak", "the rule still reads them");
        assert!(
            !d.amendments()
                .iter()
                .any(|a| matches!(a, Amendment::ExcludeFamily { .. })),
            "a family with no name was suggested for exclusion: {:?}",
            d.amendments()
        );
        assert!(
            !d.amendments().is_empty(),
            "the stall still has something to suggest"
        );

        // One recorded family among five unrecorded is not the window's family
        // either.
        let mut mixed = barren_unfamilied;
        mixed[0] = mixed[0].clone().with_family(Some("momentum"));
        let d = detect_checked(&mixed).expect("still a barren streak");
        assert!(
            !d.amendments()
                .iter()
                .any(|a| matches!(a, Amendment::ExcludeFamily { .. })),
            "one named run named the window: {:?}",
            d.amendments()
        );
    }

    #[test]
    fn a_blank_family_is_no_family_rather_than_a_family_called_nothing() {
        assert_eq!(run(0, "").family, None);
        assert_eq!(run(0, "   ").family, None);
        assert_eq!(run(0, "momentum").family, Some("momentum".to_string()));
        assert_eq!(
            run(0, "momentum").with_family(None).family,
            None,
            "a recorded family can be taken back off"
        );
    }

    #[test]
    fn a_run_with_no_precheck_verdict_says_so_and_cannot_be_circling() {
        // The common case, not the edge: `hypothesis_precheck` files
        // `blocked | duplicate | new` and most runs have no row at all, so a
        // fresh record must start at the absence rather than at a verdict.
        let fresh = run(0, "momentum");
        assert_eq!(fresh.precheck, Precheck::NotRecorded);
        assert!(!fresh.precheck.is_recorded());
        assert!(fresh.clone().as_variant().precheck.is_recorded());

        let mut runs: Vec<FinishedRun> = (0..4).map(|n| productive(n, "carry")).collect();
        runs.push(productive(4, "momentum").failing("costs_eat_the_edge", false));
        runs.push(productive(5, "momentum").failing("costs_eat_the_edge", false));
        for r in &runs[4..] {
            assert_eq!(r.precheck, Precheck::NotRecorded);
        }
        assert_eq!(
            detect_checked(&runs),
            None,
            "no verdict is not the `proceed_as_variant` the circling rule counts"
        );
    }

    #[test]
    fn every_precheck_word_round_trips_and_only_one_of_them_is_an_absence() {
        let mut recorded = 0;
        for p in Precheck::ALL {
            assert_eq!(Precheck::parse(p.as_str()), Some(*p));
            if p.is_recorded() {
                recorded += 1;
            }
        }
        assert_eq!(Precheck::ALL.len(), 3);
        assert_eq!(recorded, 2, "exactly one variant means `no verdict`");
        assert_eq!(Precheck::parse("reject_duplicate"), None);
    }

    #[test]
    fn a_new_variant_of_a_dead_family_is_not_circling_if_it_was_not_a_variant() {
        // `proceed_new` twice is two new questions, however alike they look to a
        // reader. The precheck is the platform's judgement and this rule defers
        // to it.
        let runs: Vec<FinishedRun> = vec![
            productive(0, "momentum").failing("costs_eat_the_edge", false),
            productive(1, "momentum").failing("costs_eat_the_edge", false),
            productive(2, "carry"),
            productive(3, "carry"),
        ];
        assert_eq!(detect_checked(&runs), None);
    }

    /// Twelve productive runs at one unit each, the last six of which cost far
    /// more. `2K = 12`, so the gate is exactly satisfied.
    fn drifting() -> Vec<FinishedRun> {
        let mut runs: Vec<FinishedRun> = (0..6)
            .map(|n| productive(n, "carry").producing(1, 1).costing(1.0))
            .collect();
        runs.extend((6..12).map(|n| {
            productive(n, "carry")
                .producing(1, 1)
                .costing(10.0)
                .with_workflow("optimise")
        }));
        runs
    }

    #[test]
    fn cost_drift_trips_above_three_times_the_sessions_own_median() {
        let runs = drifting();
        let d = detect_checked(&runs).expect("ten times the median is drift");
        let t = d
            .rules
            .iter()
            .find(|t| t.rule == Rule::CostDrift)
            .expect("the cost drift rule tripped");
        assert_eq!(t.items.len(), 6);
        assert!(
            d.amendments().contains(&Amendment::ExcludeWorkflow {
                workflow: "optimise".into()
            }),
            "the suggestion names what got expensive: {:?}",
            d.amendments()
        );
    }

    #[test]
    fn cost_drift_says_nothing_until_two_windows_of_history_exist() {
        // The same drift, one run short of 2K. A median of five ordinary runs is
        // not a baseline, and the rule would rather be silent than cry wolf.
        let mut runs = drifting();
        runs.remove(0);
        assert_eq!(runs.len(), 11);
        assert!(
            detect_checked(&runs)
                .as_ref()
                .is_none_or(|d| d.rules.iter().all(|t| t.rule != Rule::CostDrift)),
            "11 runs is below 2K = 12 and the rule must hold its tongue"
        );

        // One more run and it speaks.
        let runs = drifting();
        assert!(detect_checked(&runs)
            .expect("a stall")
            .rules
            .iter()
            .any(|t| t.rule == Rule::CostDrift));
    }

    #[test]
    fn the_baseline_is_the_history_because_a_window_inside_its_own_median_can_never_drift() {
        // Why `cost_drift` takes its median over the runs *before* the window
        // rather than over all of them. At the moment the gate opens the window
        // is half the session, so a median including it sits between the cheap
        // half and the dear half — and no matter how expensive the window gets,
        // `dear > 3 × (cheap + dear) / 2` has no solution. Three orders of
        // magnitude, all inert.
        for dear in [10.0, 100.0, 1_000_000.0] {
            let mut runs: Vec<FinishedRun> = (0..6)
                .map(|n| productive(n, "carry").producing(1, 1).costing(1.0))
                .collect();
            runs.extend((6..12).map(|n| productive(n, "carry").producing(1, 1).costing(dear)));
            let mut every: Vec<f64> = runs
                .iter()
                .filter_map(FinishedRun::cost_per_knowledge_unit)
                .collect();
            let including_the_window = median(&mut every).expect("twelve runs have a median");
            assert!(
                dear <= COST_DRIFT_MULTIPLE * including_the_window,
                "the literal reading was supposed to be unable to fire here, and at {dear} it \
                 fired: the argument in `cost_drift` no longer holds and the reading should be \
                 revisited"
            );
            // The reading that is built does fire, which is the point.
            assert!(
                detect_checked(&runs)
                    .expect("a stall")
                    .rules
                    .iter()
                    .any(|t| t.rule == Rule::CostDrift),
                "{dear} per unit against a history of 1.00 is drift by any reading"
            );
        }
    }

    #[test]
    fn cost_drift_is_a_ratio_so_the_unit_cancels() {
        // §7.1 compares the window against the session's own median. Multiply
        // every cost by a thousand — dollars to trials, or trials to tokens —
        // and the verdict cannot move, which is what makes it sound for
        // `select.rs` to have scored cost in trials rather than dollars.
        let runs = drifting();
        let scaled: Vec<FinishedRun> = runs
            .iter()
            .map(|r| r.clone().costing(r.cost * 1000.0))
            .collect();
        let a = detect_checked(&runs).expect("a stall");
        let b = detect_checked(&scaled).expect("a stall");
        let rules_a: Vec<Rule> = a.rules.iter().map(|t| t.rule).collect();
        let rules_b: Vec<Rule> = b.rules.iter().map(|t| t.rule).collect();
        assert_eq!(rules_a, rules_b);
        assert!(rules_a.contains(&Rule::CostDrift));
    }

    #[test]
    fn a_barren_window_does_not_also_report_infinite_cost() {
        // All twelve barren: 2K exists, so the gate is open, but the window has
        // no knowledge units and a cost per unit is undefined rather than
        // enormous. One fact, one alarm.
        let runs: Vec<FinishedRun> = (0..12).map(|n| run(n, "carry")).collect();
        let d = detect_checked(&runs).expect("twelve barren runs are a stall");
        assert!(d.rules.iter().any(|t| t.rule == Rule::BarrenStreak));
        assert!(
            d.rules.iter().all(|t| t.rule != Rule::CostDrift),
            "a window that produced nothing is barren_streak's business, not cost drift's"
        );
    }

    #[test]
    fn a_diagnosis_reports_its_rules_in_the_specs_order_and_names_items_for_each() {
        // Barren and degenerating at once: six follow-up runs producing nothing.
        let runs: Vec<FinishedRun> = (0..6)
            .map(|n| run(n, "momentum").from_source(Source::RefinementFollowup))
            .collect();
        let d = detect_checked(&runs).expect("a stall");
        let order: Vec<Rule> = d.rules.iter().map(|t| t.rule).collect();
        assert_eq!(order, vec![Rule::BarrenStreak, Rule::Degenerating]);
        assert_eq!(d.headline(), "barren_streak");
        for t in &d.rules {
            assert!(!t.items.is_empty(), "{} named no item", t.rule.as_str());
            assert!(!t.what.is_empty());
        }
        assert_eq!(d.items().len(), 6, "deduplicated across the two rules");
    }

    #[test]
    fn the_stalled_event_carries_the_word_the_fold_reads_and_a_stalled_session_claims_nothing() {
        let runs: Vec<FinishedRun> = (0..6).map(|n| run(n, "momentum")).collect();
        let d = detect_checked(&runs).expect("a stall");

        let session = Uuid::nil();
        let log = vec![
            SessionEvent::new(session, 0, EventKind::SessionCreated, serde_json::json!({})),
            SessionEvent::new(session, 1, EventKind::SessionStarted, serde_json::json!({})),
            SessionEvent::new(session, 2, EventKind::SessionStalled, d.event_payload()),
        ];
        let f = fold(&log);
        assert_eq!(
            f.state,
            Some(SessionState::Stalled {
                diagnosis: "barren_streak".into()
            }),
            "the state word and the diagnosis must be the same rule"
        );
        assert!(f.anomalies.is_empty());
        assert!(
            !f.can_claim(),
            "a stalled session claims nothing until a human acts"
        );

        // And the detail survives the round trip, because the digest reads it.
        let payload = d.event_payload();
        assert_eq!(payload["items"].as_array().expect("items").len(), 6);
        assert!(!payload["suggestions"]
            .as_array()
            .expect("suggestions")
            .is_empty());
        let back: Vec<Tripped> =
            serde_json::from_value(payload["rules"].clone()).expect("the rules round trip");
        assert_eq!(back, d.rules);
    }

    #[test]
    fn every_rule_word_round_trips() {
        for r in Rule::ALL {
            assert_eq!(Rule::parse(r.as_str()), Some(*r));
        }
        assert_eq!(Rule::parse("stalled"), None);
    }

    #[test]
    fn every_amendment_kind_is_named_and_none_of_them_is_a_number() {
        let every = [
            Amendment::ExcludeFamily {
                family: "carry".into(),
            },
            Amendment::ExcludeWorkflow {
                workflow: "optimise".into(),
            },
            Amendment::WidenScope {
                what: "it is narrow".into(),
            },
            Amendment::PreferSourcesOtherThan {
                source: Source::RefinementFollowup,
            },
        ];
        for a in &every {
            assert!(!a.kind().is_empty());
            assert!(!a.text().is_empty());
            let json = serde_json::to_value(a).expect("an amendment serialises");
            let obj = json.as_object().expect("an object");
            for (key, v) in obj {
                assert!(
                    !v.is_number(),
                    "`{key}` is a number on an amendment: the only number a stalled session \
                     could suggest changing is the effect size, and it may not"
                );
            }
        }
        // Every kind word is distinct, so the desk can switch on it.
        let kinds: Vec<&str> = every.iter().map(Amendment::kind).collect();
        let mut sorted = kinds.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), kinds.len());
    }

    #[test]
    fn an_empty_history_stalls_on_nothing() {
        assert_eq!(detect_checked(&[]), None);
        assert_eq!(detect_checked(&[run(0, "carry")]), None);
    }

    #[test]
    fn the_median_is_the_middle_and_the_mean_of_two_middles() {
        assert_eq!(median(&mut []), None);
        assert_eq!(median(&mut [4.0]), Some(4.0));
        assert_eq!(median(&mut [3.0, 1.0, 2.0]), Some(2.0));
        assert_eq!(median(&mut [4.0, 1.0, 3.0, 2.0]), Some(2.5));
    }

    #[test]
    fn cost_per_knowledge_unit_is_none_rather_than_infinite() {
        assert_eq!(run(0, "carry").cost_per_knowledge_unit(), None);
        assert_eq!(
            run(0, "carry")
                .producing(2, 1)
                .costing(5.0)
                .cost_per_knowledge_unit(),
            Some(2.5)
        );
    }
}
