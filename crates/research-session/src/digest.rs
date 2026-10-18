//! The digest (AGENT-014 P7, §2.4; Set S task S-11.12).
//!
//! The human's cost of returning is one digest. A session that runs for a week
//! is useful only if reading what it did takes minutes, and a week of a
//! continuous session is sixty conversations nobody is going to open. So the
//! platform writes one object per `review_every`, and on stall, stop and
//! complete: what ran, what it found, what it could not settle, what it spent,
//! what is next, and what stopped.
//!
//! # Written by the platform, not by a model
//!
//! P7 says this is written from yield and verdicts rather than from the model's
//! prose, and the signature is where that is true rather than promised:
//! [`write`] is a pure function from records to a [`Digest`], with no model
//! handle, no store and no clock in it. The period is passed in. There is
//! nowhere for a sentence to enter that its caller did not already hold as a
//! record, which is the reason a child run reading its own session's digest
//! learns something it did not write.
//!
//! # Every line carries an id
//!
//! "Each line opens the thing it names" (§2.4) and "every line resolves on
//! click" (§12.11, AGENT-012 P9) are the acceptance, and a line with no id
//! cannot resolve. So [`Line`] holds a [`Ref`] that is not optional: a line
//! without one does not exist, because it cannot be constructed.
//!
//! That leaves one hole a type cannot close — a [`Ref`] built from an empty id
//! — and [`Digest::unresolvable`] is the reader for it, so a test can assert on
//! a real digest that every line in it opens something.
//!
//! # Verdicts, not titles
//!
//! §2.4 asks for "runs finished with verdicts (not titles)", and [`RunRecord`]
//! has **no title field**. That is deliberate and it is not an omission: a
//! digest of forty runs listing forty questions is a digest nobody reads, and
//! the question is one click away behind the id. The reader wants to know that
//! six were refuted before they want to know what six things were asked.
//!
//! # The budget line names its unit
//!
//! The one number on this card that a reader can misread is the spend, because
//! it renders beside a day budget in dollars and this platform meters trials.
//! So [`SpendUnit`] is not optional on [`BudgetSummary`], [`write_in`] takes it
//! as an argument rather than assuming it, and it reaches the **sentence** and
//! not only the struct: `12.00 trials spent over 3 runs` is a fact, `12.00
//! spent over 3 runs` under a dollar bar is a wrong one. Migration `0139` makes
//! the stored column NOT NULL with no default for the same reason.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::agenda::Score;
use crate::stall::Diagnosis;

/// How many agenda items the digest looks ahead to (§2.4).
pub const NEXT_UP: usize = 5;

/// What kind of thing a line opens.
///
/// Closed, because the desk has to know how to resolve each one, and a kind it
/// cannot resolve is a line that renders as a dead link.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefKind {
    Session,
    /// An agenda item, which is how a reader reaches the question that ran.
    Item,
    Run,
    Finding,
    Negative,
    Question,
    Contradiction,
    Proposal,
    /// One row of `session_events`, addressed by its `seq`. What a pause line
    /// opens.
    Event,
}

impl RefKind {
    pub const ALL: &'static [RefKind] = &[
        RefKind::Session,
        RefKind::Item,
        RefKind::Run,
        RefKind::Finding,
        RefKind::Negative,
        RefKind::Question,
        RefKind::Contradiction,
        RefKind::Proposal,
        RefKind::Event,
    ];

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            RefKind::Session => "session",
            RefKind::Item => "item",
            RefKind::Run => "run",
            RefKind::Finding => "finding",
            RefKind::Negative => "negative",
            RefKind::Question => "question",
            RefKind::Contradiction => "contradiction",
            RefKind::Proposal => "proposal",
            RefKind::Event => "event",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|k| k.as_str() == s)
    }
}

/// What a line opens.
///
/// The id is a string rather than a `Uuid` because half the things a digest
/// names are memory records with their own ids (`f_31a`), and a type that could
/// only hold a `Uuid` would have forced those lines to carry nothing. It is
/// private so that the only ids in a digest are ones a constructor put there.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Ref {
    pub kind: RefKind,
    id: String,
}

impl Ref {
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    #[must_use]
    pub fn session(id: Uuid) -> Self {
        Self {
            kind: RefKind::Session,
            id: id.to_string(),
        }
    }

    #[must_use]
    pub fn item(id: Uuid) -> Self {
        Self {
            kind: RefKind::Item,
            id: id.to_string(),
        }
    }

    #[must_use]
    pub fn run(id: Uuid) -> Self {
        Self {
            kind: RefKind::Run,
            id: id.to_string(),
        }
    }

    /// An event in the session's own log, addressed by `seq`.
    #[must_use]
    pub fn event(seq: i64) -> Self {
        Self {
            kind: RefKind::Event,
            id: seq.to_string(),
        }
    }

    /// A record in memory or the ledger, by its own id.
    #[must_use]
    pub fn record(kind: RefKind, id: &str) -> Self {
        Self {
            kind,
            id: id.to_string(),
        }
    }

    /// Whether this reference opens anything at all.
    #[must_use]
    pub fn resolves(&self) -> bool {
        !self.id.trim().is_empty()
    }
}

/// One line of the digest: a sentence and the thing it opens.
///
/// There is no constructor that omits the reference, which is the whole design.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Line {
    pub at: Ref,
    pub text: String,
}

impl Line {
    #[must_use]
    pub fn new(at: Ref, text: impl Into<String>) -> Self {
        Self {
            at,
            text: text.into(),
        }
    }
}

/// The digest's sections, in §2.4's order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Heading {
    RunsFinished,
    Findings,
    Negatives,
    Questions,
    Contradictions,
    ProposalsAwaiting,
    Budget,
    NextUp,
    Paused,
}

impl Heading {
    /// In §2.4's order, which is the order the card renders.
    pub const ALL: &'static [Heading] = &[
        Heading::RunsFinished,
        Heading::Findings,
        Heading::Negatives,
        Heading::Questions,
        Heading::Contradictions,
        Heading::ProposalsAwaiting,
        Heading::Budget,
        Heading::NextUp,
        Heading::Paused,
    ];

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Heading::RunsFinished => "runs_finished",
            Heading::Findings => "findings",
            Heading::Negatives => "negatives",
            Heading::Questions => "questions",
            Heading::Contradictions => "contradictions",
            Heading::ProposalsAwaiting => "proposals_awaiting",
            Heading::Budget => "budget",
            Heading::NextUp => "next_up",
            Heading::Paused => "paused",
        }
    }

    /// What the card prints above the lines.
    #[must_use]
    pub fn title(self) -> &'static str {
        match self {
            Heading::RunsFinished => "Runs finished",
            Heading::Findings => "Findings",
            Heading::Negatives => "Negative results",
            Heading::Questions => "Questions",
            Heading::Contradictions => "Contradictions",
            Heading::ProposalsAwaiting => "Awaiting you",
            Heading::Budget => "Budget",
            Heading::NextUp => "Next up",
            Heading::Paused => "Stopped or paused",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|h| h.as_str() == s)
    }
}

/// One heading and its lines. Never empty — an empty section is omitted rather
/// than rendered as "none", because "none" is a line with nothing to open and
/// [`Line`] cannot hold one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Section {
    pub heading: Heading,
    pub lines: Vec<Line>,
}

/// The stretch of the session this digest covers.
///
/// Both a clock range, for the human, and a `seq` range, for everything else: a
/// digest that named only times could not be checked against the log, and two
/// digests written a second apart would be indistinguishable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Period {
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
    pub from_seq: i64,
    pub to_seq: i64,
}

/// What `spent` is counted in.
///
/// **A number beside a dollar bar with no unit on it is the defect this
/// closes.** The digest's budget line renders next to the session's day budget,
/// which is denominated in dollars, while everything this platform can actually
/// meter a run with is trials: [`crate::select`] scores cost in trials because
/// `llm_usage.cost_usd` keys on the auth session rather than on the run. A
/// reader who sees `12.00` beside a dollar bar reads dollars, and they are
/// wrong by whatever the trial price is.
///
/// The two words are migration `0139`'s, which makes `session_digests
/// .spend_unit` **NOT NULL with no default** and CHECKs it to these two
/// exactly, so a writer cannot leave the question open in the database either.
/// An enum rather than a free-form string because a unit that can be spelt two
/// ways is a unit that will be.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpendUnit {
    /// What a run's lease is actually metered in on this platform today.
    Trials,
    /// What the day budget is denominated in, and what nothing yet meters a run
    /// in. Here because `0139` allows it and a digest written the day that
    /// changes must be able to say so.
    Usd,
}

impl SpendUnit {
    /// Both words, in `0139`'s order. Every one of them is a word the CHECK
    /// accepts.
    pub const ALL: &'static [SpendUnit] = &[SpendUnit::Trials, SpendUnit::Usd];

    /// The stored word — what goes in `session_digests.spend_unit` and on the
    /// `digest_written` event.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            SpendUnit::Trials => "trials",
            SpendUnit::Usd => "usd",
        }
    }

    /// What the rendered sentence calls it. Not [`Self::as_str`], because
    /// `usd` reads as a column name and `USD` reads as money.
    #[must_use]
    pub fn in_prose(self) -> &'static str {
        match self {
            SpendUnit::Trials => "trials",
            SpendUnit::Usd => "USD",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|u| u.as_str() == s)
    }
}

/// What the period cost and what it bought (§2.4).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BudgetSummary {
    pub runs_finished: usize,
    pub spent: f64,
    /// What [`Self::spent`] and [`Self::cost_per_knowledge_unit`] are counted
    /// in. Not optional: see [`SpendUnit`]. Every number in this struct is in
    /// this unit, which is why there is one of these rather than one per field.
    pub unit: SpendUnit,
    pub knowledge_units: u32,
    /// `None` when the period produced no knowledge, for the reason
    /// `Yield::cost_per_knowledge_unit` gives: a barren period has no cost *per
    /// unit*, and a finite number there is a number somebody will average.
    pub cost_per_knowledge_unit: Option<f64>,
}

/// A finished run, as the digest needs to see it.
///
/// **No title.** See the module header: §2.4 asks for verdicts, and the id is
/// how a reader gets from a verdict to the question.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunRecord {
    pub item: Uuid,
    pub run: Uuid,
    /// What the run concluded — `supported`, `refuted`, `inconclusive`, or a
    /// terminal state like `failed`. A string, because
    /// [`crate::fold::Folded::finished`] holds it as one and the digest must
    /// report what the log says rather than a vocabulary of its own.
    pub verdict: String,
    pub findings_established: Vec<Entry>,
    pub findings_refuted: Vec<Entry>,
    pub negatives: Vec<NegativeRecord>,
    pub questions_opened: Vec<Entry>,
    pub questions_closed: Vec<Entry>,
    pub contradictions_found: Vec<Entry>,
    pub contradictions_resolved: Vec<Entry>,
    pub proposals: Vec<ProposalRecord>,
    pub knowledge_units: u32,
    pub spent: f64,
}

impl RunRecord {
    /// A run that finished with a verdict and produced nothing else.
    #[must_use]
    pub fn new(item: Uuid, run: Uuid, verdict: &str) -> Self {
        Self {
            item,
            run,
            verdict: verdict.to_string(),
            findings_established: Vec::new(),
            findings_refuted: Vec::new(),
            negatives: Vec::new(),
            questions_opened: Vec::new(),
            questions_closed: Vec::new(),
            contradictions_found: Vec::new(),
            contradictions_resolved: Vec::new(),
            proposals: Vec::new(),
            knowledge_units: 0,
            spent: 0.0,
        }
    }
}

/// A record in memory, by id and by what to call it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub id: String,
    pub label: String,
}

impl Entry {
    #[must_use]
    pub fn new(id: &str, label: &str) -> Self {
        Self {
            id: id.to_string(),
            label: label.to_string(),
        }
    }
}

/// A negative result and the door it closed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NegativeRecord {
    pub id: String,
    pub subject: String,
    pub class: String,
    /// Whether that class had not been recorded for the subject before — the
    /// thing §2.4 asks the digest to single out, and the thing `Yield` pays
    /// double for.
    pub class_is_new: bool,
}

/// A proposal a run made, and whether a human still owes it a decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProposalRecord {
    pub id: String,
    pub what: String,
    /// The ring the action sits in (AGENT-013 §4).
    pub ring: i16,
    /// False once a human has decided. **A session never sets this itself**
    /// (AGENT-014 P6): the digest reports what is waiting, and a session that
    /// could clear its own queue would be a session that approves things.
    pub awaiting: bool,
}

/// An agenda item the session means to run next, with why.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Upcoming {
    pub item: Uuid,
    pub question: String,
    /// `None` before the scorer has seen it. Rendered as "not scored yet"
    /// rather than as zero, which would read as *worthless*.
    pub score: Option<Score>,
    /// Why this item is next — one sentence, from what the agenda row itself
    /// holds: the sources that wanted the question, and the score's own terms
    /// when it has been scored.
    ///
    /// **Not the `rationale` off a `decision` row (§5.3), and it cannot be.** A
    /// `decision` row is written when an item is *chosen*, and every item here
    /// is one that has not been: an unchosen item appears only in the
    /// `candidate_set` of some other item's decision. So no item in the next
    /// [`NEXT_UP`] has a decision row of its own, and a field documented as
    /// carrying one would have been a field no caller could ever fill.
    pub why: String,
}

/// Something that stopped the session during the period.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pause {
    /// The `seq` of the event that did it, which is what the line opens.
    pub seq: i64,
    pub at: DateTime<Utc>,
    pub what: String,
}

impl Pause {
    #[must_use]
    pub fn new(seq: i64, at: DateTime<Utc>, what: &str) -> Self {
        Self {
            seq,
            at,
            what: what.to_string(),
        }
    }

    /// The pause a stall causes, written from the diagnosis rather than
    /// paraphrased.
    ///
    /// The digest is the first thing the human reads after a stall — S-11.12
    /// says one is written immediately — so the line has to carry the rule and
    /// what to do about it, not the word "stalled".
    #[must_use]
    pub fn stalled(seq: i64, at: DateTime<Utc>, diagnosis: &Diagnosis) -> Self {
        let suggestions: Vec<String> = diagnosis
            .amendments()
            .iter()
            .map(crate::stall::Amendment::text)
            .collect();
        let tail = if suggestions.is_empty() {
            String::new()
        } else {
            format!("; you could {}", suggestions.join(", or "))
        };
        Self {
            seq,
            at,
            what: format!(
                "stalled: {} over {} of {} runs{tail}",
                diagnosis.headline(),
                diagnosis.runs_considered,
                diagnosis.runs_total
            ),
        }
    }
}

/// Everything [`write`] reads.
///
/// Named `DigestInput` rather than `Inputs` because [`crate::select::Inputs`] is
/// re-exported at the crate root and two `Inputs` in one namespace is a way to
/// import the wrong one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DigestInput {
    pub session: Uuid,
    pub period: Period,
    /// Runs finished in the period, oldest first.
    pub runs: Vec<RunRecord>,
    /// The agenda as it stands, best first. Only the first [`NEXT_UP`] are
    /// shown.
    pub upcoming: Vec<Upcoming>,
    pub pauses: Vec<Pause>,
    /// P3's scorecard over the period: which of the three layers caught each
    /// duplicate. `None` when the caller did not read one — never a scorecard
    /// of zeros standing in for "not asked".
    #[serde(default)]
    pub dedupe: Option<crate::agenda::DedupeScorecard>,
    /// Contradiction pairs the platform found in the period
    /// (`session_contradictions.found_at`), whichever run's reading surfaced
    /// them. A pair belongs to the memory and not to a run, so it is carried
    /// here and not on a [`RunRecord`]; both render under the same heading.
    #[serde(default)]
    pub contradictions_found: Vec<Entry>,
    /// Pairs that stopped being true in the period.
    #[serde(default)]
    pub contradictions_resolved: Vec<Entry>,
}

/// The digest itself.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Digest {
    pub session: Uuid,
    pub period: Period,
    pub budget: BudgetSummary,
    /// Only the sections with something in them, in [`Heading::ALL`]'s order.
    pub sections: Vec<Section>,
    /// Carried through from [`DigestInput::dedupe`], as a structure rather
    /// than lines: a [`Line`] must resolve to a thing it names, and a count
    /// of duplicates caught at a layer names no single thing to open.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dedupe: Option<crate::agenda::DedupeScorecard>,
}

impl Digest {
    /// Every line, in the order the card renders them.
    pub fn lines(&self) -> impl Iterator<Item = &Line> {
        self.sections.iter().flat_map(|s| s.lines.iter())
    }

    #[must_use]
    pub fn line_count(&self) -> usize {
        self.sections.iter().map(|s| s.lines.len()).sum()
    }

    /// The lines that would render as dead links.
    ///
    /// Empty on every digest [`write`] produces from records that carry ids.
    /// This exists so the acceptance — every line resolves on click — can be
    /// asserted on a real digest rather than argued from the type.
    #[must_use]
    pub fn unresolvable(&self) -> Vec<&Line> {
        self.lines().filter(|l| !l.at.resolves()).collect()
    }

    #[must_use]
    pub fn section(&self, heading: Heading) -> Option<&Section> {
        self.sections.iter().find(|s| s.heading == heading)
    }
}

/// Write the digest for a period, in the unit the caller meters runs in (§2.4).
///
/// Pure and total: the same input gives the same digest, and there is no input
/// it refuses. A digest of a quiet period is a digest saying the session spent
/// nothing, which is a fact the human wants; refusing to write one would leave
/// them wondering whether the job was even running.
///
/// `spend_unit` is a parameter and not a default because the budget line
/// renders beside a dollar bar and this platform meters trials — see
/// [`SpendUnit`]. It reaches the rendered sentence, not only
/// [`BudgetSummary::unit`], because the sentence is the thing a human reads.
#[must_use]
pub fn write_in(input: &DigestInput, spend_unit: SpendUnit) -> Digest {
    let mut sections: Vec<Section> = Vec::new();

    let mut runs = Vec::new();
    let mut findings = Vec::new();
    let mut negatives = Vec::new();
    let mut questions = Vec::new();
    let mut contradictions = Vec::new();
    let mut proposals = Vec::new();

    for r in &input.runs {
        runs.push(Line::new(
            Ref::run(r.run),
            format!("{} — {}", r.verdict, units(r.knowledge_units)),
        ));

        for f in &r.findings_established {
            findings.push(Line::new(
                Ref::record(RefKind::Finding, &f.id),
                format!("established: {}", f.label),
            ));
        }
        for f in &r.findings_refuted {
            findings.push(Line::new(
                Ref::record(RefKind::Finding, &f.id),
                format!("refuted: {}", f.label),
            ));
        }
        for n in &r.negatives {
            negatives.push(Line::new(
                Ref::record(RefKind::Negative, &n.id),
                if n.class_is_new {
                    format!(
                        "{} failed on `{}` — a failure class new for this subject",
                        n.subject, n.class
                    )
                } else {
                    format!("{} failed on `{}`", n.subject, n.class)
                },
            ));
        }
        for q in &r.questions_opened {
            questions.push(Line::new(
                Ref::record(RefKind::Question, &q.id),
                format!("opened: {}", q.label),
            ));
        }
        for q in &r.questions_closed {
            questions.push(Line::new(
                Ref::record(RefKind::Question, &q.id),
                format!("closed: {}", q.label),
            ));
        }
        for c in &r.contradictions_found {
            contradictions.push(Line::new(
                Ref::record(RefKind::Contradiction, &c.id),
                format!("found: {}", c.label),
            ));
        }
        for c in &r.contradictions_resolved {
            contradictions.push(Line::new(
                Ref::record(RefKind::Contradiction, &c.id),
                format!("resolved: {}", c.label),
            ));
        }
        // Only the ones still waiting. A proposal a human has already decided is
        // not something the digest is asking for, and listing it would make the
        // section a log rather than a queue.
        for p in r.proposals.iter().filter(|p| p.awaiting) {
            proposals.push(Line::new(
                Ref::record(RefKind::Proposal, &p.id),
                format!("{} (ring {}) — waiting for you", p.what, p.ring),
            ));
        }
    }

    for c in &input.contradictions_found {
        contradictions.push(Line::new(
            Ref::record(RefKind::Contradiction, &c.id),
            format!("found: {}", c.label),
        ));
    }
    for c in &input.contradictions_resolved {
        contradictions.push(Line::new(
            Ref::record(RefKind::Contradiction, &c.id),
            format!("resolved: {}", c.label),
        ));
    }
    push(&mut sections, Heading::RunsFinished, runs);
    push(&mut sections, Heading::Findings, findings);
    push(&mut sections, Heading::Negatives, negatives);
    push(&mut sections, Heading::Questions, questions);
    push(&mut sections, Heading::Contradictions, contradictions);
    push(&mut sections, Heading::ProposalsAwaiting, proposals);

    let budget = summarise(&input.runs, spend_unit);
    // The unit is in the sentence and not only in the struct. The card shows
    // this line beside a day budget in dollars, so a bare `12.00` is read as
    // money whatever `BudgetSummary::unit` says.
    let unit = spend_unit.in_prose();
    push(
        &mut sections,
        Heading::Budget,
        vec![Line::new(
            Ref::session(input.session),
            match budget.cost_per_knowledge_unit {
                Some(per) => format!(
                    "{:.2} {unit} spent over {} run{}, {} — {per:.2} {unit} per knowledge unit",
                    budget.spent,
                    budget.runs_finished,
                    plural(budget.runs_finished),
                    units(budget.knowledge_units),
                ),
                None => format!(
                    "{:.2} {unit} spent over {} run{} and no knowledge units, so there is no cost \
                     per unit to report",
                    budget.spent,
                    budget.runs_finished,
                    plural(budget.runs_finished),
                ),
            },
        )],
    );

    let next: Vec<Line> = input
        .upcoming
        .iter()
        .take(NEXT_UP)
        .map(|u| {
            Line::new(
                Ref::item(u.item),
                match u.score {
                    Some(s) => format!("{} — score {:.3}; {}", u.question, s.total, u.why),
                    None => format!("{} — not scored yet; {}", u.question, u.why),
                },
            )
        })
        .collect();
    push(&mut sections, Heading::NextUp, next);

    let paused: Vec<Line> = input
        .pauses
        .iter()
        .map(|p| Line::new(Ref::event(p.seq), p.what.clone()))
        .collect();
    push(&mut sections, Heading::Paused, paused);

    Digest {
        session: input.session,
        period: input.period,
        budget,
        sections,
        dedupe: input.dedupe,
    }
}

/// [`write_in`] in [`PLATFORM_SPEND_UNIT`] — what every caller on this platform
/// meters a run in today.
///
/// **This is a bridge, not the interface.** It exists so that a caller written
/// before the unit was a parameter keeps compiling and keeps rendering the
/// truth, because the unit it was silently using was trials and this names it.
/// It is the one place in this crate where the unit is assumed rather than
/// stated, so a second unit must not arrive through it: a caller that meters
/// anything else calls [`write_in`], and `the_bridge_renders_the_platforms_own_
/// unit` is what holds this to trials if somebody changes
/// [`PLATFORM_SPEND_UNIT`] without thinking about the callers of this function.
#[must_use]
pub fn write(input: &DigestInput) -> Digest {
    write_in(input, PLATFORM_SPEND_UNIT)
}

/// The unit a run's lease is metered in on this platform (`api`'s `SPEND_UNIT`).
///
/// Here rather than only in the driver because [`write`] needs a word and a
/// word invented at the render site is the thing `0139` was written to stop.
pub const PLATFORM_SPEND_UNIT: SpendUnit = SpendUnit::Trials;

/// The period's spend and what it bought, in the caller's unit.
///
/// The unit is carried rather than derived: every `RunRecord.spent` in one
/// digest is metered the same way, and a summary that could not say which way
/// is the summary that started this.
fn summarise(runs: &[RunRecord], unit: SpendUnit) -> BudgetSummary {
    let spent: f64 = runs.iter().map(|r| r.spent).sum();
    let knowledge_units: u32 = runs.iter().map(|r| r.knowledge_units).sum();
    BudgetSummary {
        runs_finished: runs.len(),
        spent,
        unit,
        knowledge_units,
        cost_per_knowledge_unit: (knowledge_units > 0).then(|| spent / f64::from(knowledge_units)),
    }
}

fn push(sections: &mut Vec<Section>, heading: Heading, lines: Vec<Line>) {
    if !lines.is_empty() {
        sections.push(Section { heading, lines });
    }
}

fn units(n: u32) -> String {
    format!("{n} knowledge unit{}", if n == 1 { "" } else { "s" })
}

fn plural(n: usize) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stall::{detect, FinishedRun};

    fn when(h: u32) -> DateTime<Utc> {
        DateTime::from_timestamp(1_770_000_000 + i64::from(h) * 3600, 0).expect("a timestamp")
    }

    fn period() -> Period {
        Period {
            from: when(0),
            to: when(24),
            from_seq: 10,
            to_seq: 87,
        }
    }

    fn item(n: u8) -> Uuid {
        Uuid::from_u128(u128::from(n))
    }

    fn run_id(n: u8) -> Uuid {
        Uuid::from_u128(u128::from(n) + 1000)
    }

    /// A week's worth: three runs, one of each verdict, with everything §2.4
    /// asks the digest to carry.
    fn a_busy_week() -> DigestInput {
        let mut supported = RunRecord::new(item(1), run_id(1), "supported");
        supported.findings_established = vec![Entry::new("f_31a", "momentum survives costs on 1h")];
        supported.questions_closed = vec![Entry::new("q_07", "does momentum survive costs?")];
        supported.proposals = vec![ProposalRecord {
            id: "p_11".into(),
            what: "run this in paper for a fortnight".into(),
            ring: 2,
            awaiting: true,
        }];
        supported.knowledge_units = 2;
        supported.spent = 4.0;

        let mut refuted = RunRecord::new(item(2), run_id(2), "refuted");
        refuted.findings_refuted = vec![Entry::new("f_12b", "carry is stable across regimes")];
        refuted.negatives = vec![NegativeRecord {
            id: "n_04".into(),
            subject: "ETH-USD".into(),
            class: "costs_eat_the_edge".into(),
            class_is_new: true,
        }];
        refuted.contradictions_found = vec![Entry::new("c_02", "f_31a disagrees with f_47c")];
        refuted.knowledge_units = 3;
        refuted.spent = 5.0;

        let mut inconclusive = RunRecord::new(item(3), run_id(3), "inconclusive");
        inconclusive.questions_opened = vec![Entry::new("q_09", "is the 1h window too short?")];
        inconclusive.contradictions_resolved = vec![Entry::new("c_01", "f_03 superseded f_02")];
        inconclusive.proposals = vec![ProposalRecord {
            id: "p_12".into(),
            what: "already decided, so not waiting".into(),
            ring: 1,
            awaiting: false,
        }];
        inconclusive.knowledge_units = 1;
        inconclusive.spent = 3.0;

        DigestInput {
            session: Uuid::from_u128(999),
            period: period(),
            runs: vec![supported, refuted, inconclusive],
            upcoming: (0..7)
                .map(|n| Upcoming {
                    item: item(20 + n),
                    question: format!("question {n}"),
                    score: Some(Score::combine(1.0, 1.0, 1.0, f64::from(n) + 1.0)),
                    why: "highest value: resolves a contradiction".into(),
                })
                .collect(),
            pauses: vec![Pause::new(
                61,
                when(12),
                "the schedule window closed at 18:00",
            )],
            dedupe: None,
            contradictions_found: vec![],
            contradictions_resolved: vec![],
        }
    }

    #[test]
    fn every_line_carries_an_id_that_opens_something() {
        // AGENT-014 §12.11 / AGENT-012 P9: every line resolves on click. A line
        // with no id cannot, so this is the acceptance rather than a nicety.
        let d = write(&a_busy_week());
        assert!(
            d.line_count() >= 12,
            "a test over an empty digest would pass by matching nothing; this one saw {} lines",
            d.line_count()
        );
        assert_eq!(
            d.unresolvable(),
            Vec::<&Line>::new(),
            "a line that opens nothing is a dead link in the one card the human reads"
        );
        for l in d.lines() {
            assert!(!l.text.trim().is_empty(), "a blank line under {:?}", l.at);
            assert!(
                RefKind::ALL.contains(&l.at.kind),
                "{:?} is not a kind the desk can resolve",
                l.at.kind
            );
        }
    }

    #[test]
    fn a_run_is_reported_by_its_verdict_and_not_by_its_title() {
        let d = write(&a_busy_week());
        let runs = d
            .section(Heading::RunsFinished)
            .expect("three runs finished");
        assert_eq!(runs.lines.len(), 3);
        let verdicts: Vec<&str> = runs.lines.iter().map(|l| l.text.as_str()).collect();
        assert!(verdicts.iter().any(|t| t.starts_with("supported")));
        assert!(verdicts.iter().any(|t| t.starts_with("refuted")));
        assert!(verdicts.iter().any(|t| t.starts_with("inconclusive")));
        // The questions are one click away and not on the line — `RunRecord`
        // has no field they could have come from, and this asserts the
        // rendering did not invent one.
        for l in &runs.lines {
            assert!(
                !l.text.contains('?'),
                "a question mark in a run line means a title got in: {}",
                l.text
            );
            assert_eq!(l.at.kind, RefKind::Run);
        }
    }

    #[test]
    fn findings_negatives_questions_and_contradictions_each_say_which_way_round() {
        let d = write(&a_busy_week());
        let text = |h: Heading| -> Vec<String> {
            d.section(h)
                .unwrap_or_else(|| panic!("{} has lines", h.as_str()))
                .lines
                .iter()
                .map(|l| l.text.clone())
                .collect()
        };

        let findings = text(Heading::Findings);
        assert_eq!(findings.len(), 2);
        assert!(findings.iter().any(|t| t.starts_with("established:")));
        assert!(
            findings.iter().any(|t| t.starts_with("refuted:")),
            "a refuted finding is knowledge and the digest has to show it as such"
        );

        let negatives = text(Heading::Negatives);
        assert_eq!(negatives.len(), 1);
        assert!(
            negatives[0].contains("new for this subject"),
            "§2.4 asks for negatives *with new classes*: {}",
            negatives[0]
        );

        let questions = text(Heading::Questions);
        assert!(questions.iter().any(|t| t.starts_with("opened:")));
        assert!(questions.iter().any(|t| t.starts_with("closed:")));

        let contradictions = text(Heading::Contradictions);
        assert!(contradictions.iter().any(|t| t.starts_with("found:")));
        assert!(contradictions.iter().any(|t| t.starts_with("resolved:")));
    }

    #[test]
    fn only_the_proposals_still_waiting_are_listed() {
        let d = write(&a_busy_week());
        let awaiting = d
            .section(Heading::ProposalsAwaiting)
            .expect("one proposal is waiting");
        assert_eq!(awaiting.lines.len(), 1);
        assert_eq!(awaiting.lines[0].at.id(), "p_11");
        assert!(awaiting.lines[0].text.contains("ring 2"));
        assert!(
            !d.lines().any(|l| l.at.id() == "p_12"),
            "a proposal a human has already decided is not a queue item"
        );
    }

    #[test]
    fn the_budget_line_reports_cost_per_knowledge_unit() {
        let d = write(&a_busy_week());
        assert_eq!(d.budget.runs_finished, 3);
        assert!((d.budget.spent - 12.0).abs() < 1e-12);
        assert_eq!(d.budget.knowledge_units, 6);
        assert_eq!(d.budget.cost_per_knowledge_unit, Some(2.0));
        let line = &d.section(Heading::Budget).expect("a budget line").lines[0];
        assert!(
            line.text.contains("2.00 trials per knowledge unit"),
            "{}",
            line.text
        );
        assert_eq!(line.at, Ref::session(d.session));
    }

    #[test]
    fn the_budget_sentence_names_its_unit_and_never_renders_a_bare_number() {
        // The whole point: this line renders beside a day budget in dollars.
        // Both branches of it — with a cost per unit and without — have to say
        // what the number is counted in, so neither can be read as money.
        let busy = a_busy_week();
        let mut barren = a_busy_week();
        for r in &mut barren.runs {
            r.knowledge_units = 0;
        }

        let mut checked = 0;
        for input in [&busy, &barren] {
            for unit in SpendUnit::ALL {
                let d = write_in(input, *unit);
                assert_eq!(d.budget.unit, *unit);
                let line = &d.section(Heading::Budget).expect("a budget line").lines[0];
                let spent = format!("{:.2}", d.budget.spent);
                assert!(
                    line.text.contains(&format!("{spent} {}", unit.in_prose())),
                    "the spend is not named in {}: {}",
                    unit.as_str(),
                    line.text
                );
                assert!(
                    !line.text.contains(&format!("{spent} spent")),
                    "a bare number beside a dollar bar: {}",
                    line.text
                );
                checked += 1;
            }
        }
        assert_eq!(checked, 4, "both branches, both units");
    }

    #[test]
    fn the_bridge_renders_the_platforms_own_unit() {
        // `write` assumes a unit; this is the assertion that it assumes the one
        // the platform actually meters, and that the assumption reaches the
        // sentence rather than stopping at the struct.
        assert_eq!(PLATFORM_SPEND_UNIT, SpendUnit::Trials);
        let d = write(&a_busy_week());
        assert_eq!(d.budget.unit, PLATFORM_SPEND_UNIT);
        assert_eq!(d, write_in(&a_busy_week(), PLATFORM_SPEND_UNIT));
        let line = &d.section(Heading::Budget).expect("a budget line").lines[0];
        assert!(
            line.text.contains(PLATFORM_SPEND_UNIT.in_prose()),
            "{}",
            line.text
        );
    }

    #[test]
    fn every_spend_unit_word_round_trips_and_is_one_0139_accepts() {
        // `0139` CHECKs `spend_unit IN ('trials', 'usd')` NOT NULL with no
        // default. A word here that the CHECK refuses is a digest that cannot
        // be stored, which is a failure at write time rather than at review.
        const CHECKED: [&str; 2] = ["trials", "usd"];
        assert_eq!(SpendUnit::ALL.len(), CHECKED.len());
        for unit in SpendUnit::ALL {
            assert_eq!(SpendUnit::parse(unit.as_str()), Some(*unit));
            assert!(
                CHECKED.contains(&unit.as_str()),
                "{} is not a word 0139 accepts",
                unit.as_str()
            );
            assert!(!unit.in_prose().trim().is_empty());
        }
        assert_eq!(SpendUnit::parse("dollars"), None);
    }

    #[test]
    fn a_period_that_produced_nothing_says_so_rather_than_dividing_by_zero() {
        let mut input = a_busy_week();
        for r in &mut input.runs {
            r.knowledge_units = 0;
        }
        let d = write(&input);
        assert_eq!(d.budget.cost_per_knowledge_unit, None);
        let line = &d.section(Heading::Budget).expect("a budget line").lines[0];
        assert!(
            line.text.contains("no cost per unit to report"),
            "{}",
            line.text
        );
    }

    #[test]
    fn the_next_five_are_five_with_their_scores_and_why() {
        let d = write(&a_busy_week());
        let next = d.section(Heading::NextUp).expect("an agenda ahead");
        assert_eq!(next.lines.len(), NEXT_UP, "seven queued, five shown");
        for l in &next.lines {
            assert!(l.text.contains("score "), "{}", l.text);
            assert!(l.text.contains("resolves a contradiction"), "{}", l.text);
            assert_eq!(l.at.kind, RefKind::Item);
        }
    }

    #[test]
    fn an_unscored_item_is_not_scored_yet_rather_than_zero() {
        let mut input = a_busy_week();
        input.upcoming[0].score = None;
        let d = write(&input);
        let first = &d.section(Heading::NextUp).expect("an agenda").lines[0];
        assert!(first.text.contains("not scored yet"), "{}", first.text);
        assert!(
            !first.text.contains("score 0"),
            "a zero reads as worthless, and an unscored item is not worthless: {}",
            first.text
        );
    }

    #[test]
    fn the_dedupe_scorecard_rides_on_the_stored_digest() {
        let mut input = a_busy_week();
        input.dedupe = Some(crate::agenda::DedupeScorecard::over(&[]).with_intake(2));
        let d = write(&input);
        let card = d.dedupe.expect("carried through");
        assert_eq!(card.intake, crate::agenda::LayerCount::Caught(2));
        let json = serde_json::to_value(write(&input)).expect("serialises");
        assert!(json.get("dedupe").is_some(), "{json}");
        // And a digest written without one says nothing rather than zeros.
        let none = serde_json::to_value(write(&a_busy_week())).expect("serialises");
        assert!(none.get("dedupe").is_none(), "{none}");
    }

    #[test]
    fn a_pair_found_in_the_period_is_a_line_that_resolves_to_the_pair_and_not_to_a_run() {
        let mut input = a_busy_week();
        input.contradictions_found = vec![Entry::new("pair-1", "A momentum pays, B it does not")];
        input.contradictions_resolved = vec![Entry::new("pair-0", "f_03 superseded f_02")];
        let d = write(&input);
        let section = d
            .section(Heading::Contradictions)
            .expect("a contradictions section");
        let texts: Vec<&str> = section.lines.iter().map(|l| l.text.as_str()).collect();
        assert!(
            texts.iter().any(|t| t.starts_with("found: A momentum")),
            "{texts:?}"
        );
        assert!(
            texts.iter().any(|t| t.starts_with("resolved: f_03")),
            "{texts:?}"
        );
        assert!(d.unresolvable().is_empty(), "every id resolves");
    }

    #[test]
    fn a_quiet_period_is_still_a_digest_and_it_is_only_the_budget() {
        let d = write(&DigestInput {
            session: Uuid::from_u128(999),
            period: period(),
            runs: vec![],
            upcoming: vec![],
            pauses: vec![],
            dedupe: None,
            contradictions_found: vec![],
            contradictions_resolved: vec![],
        });
        assert_eq!(d.line_count(), 1, "the budget line, and nothing else");
        assert_eq!(d.budget.runs_finished, 0);
        assert!(d.section(Heading::RunsFinished).is_none());
        assert!(
            d.unresolvable().is_empty(),
            "even the quiet digest's one line opens the session"
        );
    }

    #[test]
    fn the_sections_come_out_in_the_specs_order() {
        let d = write(&a_busy_week());
        let order: Vec<Heading> = d.sections.iter().map(|s| s.heading).collect();
        let expected: Vec<Heading> = Heading::ALL
            .iter()
            .copied()
            .filter(|h| order.contains(h))
            .collect();
        assert_eq!(order, expected);
        assert!(
            order.len() >= 8,
            "this fixture should exercise nearly every section, not two of them; it had {}",
            order.len()
        );
    }

    #[test]
    fn a_stall_writes_a_pause_line_naming_the_rule_and_what_to_do() {
        let runs: Vec<FinishedRun> = (0_u8..6)
            .map(|n| {
                FinishedRun::new(
                    Uuid::from_u128(u128::from(n)),
                    Uuid::from_u128(u128::from(n) + 500),
                    "momentum",
                )
            })
            .collect();
        let diagnosis = detect(&runs).expect("six barren runs are a stall");

        let mut input = a_busy_week();
        input.pauses = vec![Pause::stalled(87, when(20), &diagnosis)];
        let d = write(&input);

        let paused = d.section(Heading::Paused).expect("the stall paused it");
        assert_eq!(paused.lines.len(), 1);
        let text = &paused.lines[0].text;
        assert!(text.contains("barren_streak"), "{text}");
        assert!(
            text.contains("you could"),
            "the human needs an action: {text}"
        );
        assert_eq!(paused.lines[0].at, Ref::event(87));
        assert!(
            !text.to_lowercase().contains("delta"),
            "the digest is the one surface the amendment is read from, and it may not suggest \
             lowering the effect size: {text}"
        );
    }

    #[test]
    fn a_digest_round_trips_through_json() {
        // It is stored in `session_digests` and read back by the desk and by
        // `lifecycle.get_digest`, so the serialised shape is part of the
        // contract rather than an implementation detail.
        let d = write(&a_busy_week());
        let json = serde_json::to_string(&d).expect("a digest serialises");
        let back: Digest = serde_json::from_str(&json).expect("and comes back");
        assert_eq!(back, d);
        assert!(json.contains("\"runs_finished\""));
    }

    #[test]
    fn writing_the_same_period_twice_gives_the_same_digest() {
        // P7: written by the platform from records, not by a model. `write` has
        // no clock and no model in its signature, and this is what that buys.
        let input = a_busy_week();
        assert_eq!(write(&input), write(&input));
    }

    #[test]
    fn every_ref_kind_and_heading_word_round_trips() {
        for k in RefKind::ALL {
            assert_eq!(RefKind::parse(k.as_str()), Some(*k));
        }
        for h in Heading::ALL {
            assert_eq!(Heading::parse(h.as_str()), Some(*h));
            assert!(!h.title().is_empty());
        }
        assert_eq!(Heading::parse("runs"), None);
    }

    #[test]
    fn a_reference_with_no_id_does_not_resolve() {
        let empty = Ref::record(RefKind::Finding, "  ");
        assert!(!empty.resolves());
        assert!(Ref::session(Uuid::nil()).resolves());
        assert!(Ref::event(0).resolves(), "seq 0 is a real event");

        // And a digest built from a record whose id went missing reports it
        // rather than rendering it.
        let mut input = a_busy_week();
        input.runs[0].findings_established = vec![Entry::new("", "a finding with no id")];
        let d = write(&input);
        assert_eq!(d.unresolvable().len(), 1);
    }
}
