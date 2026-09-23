//! Open questions are memory (AGENT-013 §9; AGENT-007 §2.2 amended; Set S task
//! S-10.12).
//!
//! # What this fixes
//!
//! A run notices something it cannot answer — *"the funding series has a gap in
//! March and I do not know whether it is missing or zero"* — and that noticing
//! dies with the conversation. The next run rediscovers it, spends a step on it,
//! and notices it again. Over a month the platform pays for the same realisation
//! repeatedly and never once acts on it.
//!
//! So a question is an entry like any other: it has a subject, it is retrieved
//! into the next run's brief for that subject, and it is *closed* when somebody
//! answers it.
//!
//! # The model cannot close one
//!
//! Same discipline as the verdict (AGENT-013 §6.2), for the same reason. A run
//! that could mark its own questions answered would mark them answered — not
//! dishonestly, but because a run that has been thinking about something for
//! nine steps genuinely feels it has resolved it.
//!
//! The platform closes a question when a later finding **cites** it. That is a
//! fact about what was written down, not about how anybody felt.
//!
//! # `blocking` is the field that earns its keep
//!
//! Most open questions are curiosities and belong at the bottom of a brief slot
//! that holds 10% of `Reference`. A *blocking* one is different: it is a question
//! whose answer changes what the run should do next, and a brief that buried it
//! under six curiosities would be a brief that let the run repeat the mistake
//! the question exists to prevent.

use serde::{Deserialize, Serialize};

/// How much the asker thought the answer was worth.
///
/// The **asker's** estimate, and deliberately not scored into
/// [`Yield`](crate::lifecycle::r#yield::Yield): opening a question is worth
/// nothing there, and a value field that fed a metric would be a run grading its
/// own output. It orders a brief slot; it does not earn anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuestionValue {
    /// Worth knowing if it is cheap.
    #[default]
    Curious,
    /// Would change how a future run is designed.
    Useful,
    /// Something is being decided on an assumption nobody has checked.
    Decisive,
}

impl QuestionValue {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            QuestionValue::Curious => "curious",
            QuestionValue::Useful => "useful",
            QuestionValue::Decisive => "decisive",
        }
    }

    pub const ALL: &'static [QuestionValue] = &[
        QuestionValue::Curious,
        QuestionValue::Useful,
        QuestionValue::Decisive,
    ];
}

/// How a question stopped being open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum How {
    /// A later finding cited it. The only way the platform closes one by itself.
    Answered,
    /// The thing it was about no longer exists — a retired strategy, a dropped
    /// venue. A human says so; the platform cannot tell this from unanswered.
    Obsolete,
    /// It turned out to be another question already open.
    Merged,
}

impl How {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            How::Answered => "answered",
            How::Obsolete => "obsolete",
            How::Merged => "merged",
        }
    }

    pub const ALL: &'static [How] = &[How::Answered, How::Obsolete, How::Merged];
}

/// Whether anybody has answered it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Status {
    Open,
    Closed {
        /// The finding, question or person that closed it. Required, because a
        /// question closed by nobody is a question somebody deleted.
        by: String,
        how: How,
    },
}

/// Something a run noticed and could not answer (§9).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpenQuestion {
    pub id: String,
    /// What it is about, so the next run working on that subject is told.
    pub subject: String,
    pub question: String,
    /// Why anyone should care. **Required**: a question with no stated stakes is
    /// one nobody will ever prioritise, so it will sit in the store forever
    /// being retrieved and skipped.
    pub why_it_matters: String,
    /// Whether the answer changes what a run should do next.
    pub blocking: bool,
    pub value: QuestionValue,
    pub status: Status,
    /// The run that noticed it.
    pub opened_by: String,
}

/// Why a question was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum QuestionError {
    #[error("a question needs a subject, so the run that could answer it is told about it")]
    NoSubject,
    #[error("a question with no stated stakes is one nobody will prioritise: say why it matters")]
    NoStakes,
    #[error("say what the question is")]
    Empty,
}

/// Open one.
///
/// # Errors
/// [`QuestionError`] naming the missing part.
pub fn open(
    id: impl Into<String>,
    subject: &str,
    question: &str,
    why_it_matters: &str,
    blocking: bool,
    value: QuestionValue,
    opened_by: impl Into<String>,
) -> Result<OpenQuestion, QuestionError> {
    if question.trim().is_empty() {
        return Err(QuestionError::Empty);
    }
    if subject.trim().is_empty() {
        return Err(QuestionError::NoSubject);
    }
    if why_it_matters.trim().is_empty() {
        return Err(QuestionError::NoStakes);
    }
    Ok(OpenQuestion {
        id: id.into(),
        subject: subject.trim().to_string(),
        question: question.trim().to_string(),
        why_it_matters: why_it_matters.trim().to_string(),
        blocking,
        value,
        status: Status::Open,
        opened_by: opened_by.into(),
    })
}

impl OpenQuestion {
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.status == Status::Open
    }

    /// Whether this question was opened with stated stakes.
    ///
    /// The desk sorts on it and the brief reads it: a question carrying
    /// [`STAKES_NOT_STATED`] was noticed, not asked, and a reader deciding what
    /// to spend a run on wants to know which.
    #[must_use]
    pub fn stakes_were_stated(&self) -> bool {
        self.why_it_matters != STAKES_NOT_STATED
    }

    /// The `extra` payload for a memory entry.
    ///
    /// **One function, two writers.** `lifecycle.open_question` and the loop's
    /// run-end write both produce it, and before this existed each inlined its
    /// own object — which is the shape where one of them grows a key the reader
    /// does not know about, and the entries that came from the other path
    /// silently lose it.
    #[must_use]
    pub fn extra(&self) -> serde_json::Value {
        let status = match &self.status {
            Status::Open => serde_json::json!("open"),
            Status::Closed { by, how } => serde_json::json!({
                "closed": { "by": by, "how": how.as_str() }
            }),
        };
        serde_json::json!({
            "question": self.question,
            "why_it_matters": self.why_it_matters,
            "blocking": self.blocking,
            "value": self.value.as_str(),
            "status": status,
        })
    }

    /// Close it, because a finding cited it.
    ///
    /// The **platform's** move, and the only one it makes by itself. A run that
    /// could mark its own questions answered would — not dishonestly, but
    /// because a run that has been thinking about something for nine steps
    /// genuinely feels it has resolved it.
    ///
    /// Already-closed questions are left alone: a second close would overwrite
    /// who closed it first, and "who answered this" is the part worth keeping.
    pub fn close_as_answered(&mut self, finding_id: &str) -> bool {
        if !self.is_open() || finding_id.trim().is_empty() {
            return false;
        }
        self.status = Status::Closed {
            by: finding_id.trim().to_string(),
            how: How::Answered,
        };
        true
    }
}

/// What a run-end write says when the stakes were never stated.
///
/// A sentinel with one home. It reaches the store through
/// [`noticed`] and is recognised through [`OpenQuestion::stakes_were_stated`],
/// so the deliberate tool and the reflection path can be told apart afterwards
/// — which is the whole reason the two paths exist separately.
pub const STAKES_NOT_STATED: &str =
    "not stated: noticed during a reflection rather than opened deliberately";

/// The keys an open question occupies in a memory entry's `extra`.
///
/// Named so a reader of `agent_memory.entry` knows what to expect and a second
/// writer cannot invent a sixth key nothing reads.
pub const EXTRA_KEYS: &[&str] = &["why_it_matters", "blocking", "value", "status", "question"];

/// A question a run noticed in passing, with its stakes unstated.
///
/// Separate from [`open`] because the difference is the point: [`open`] refuses
/// a question with no stakes, and this one *records* that there were none rather
/// than inventing some. A reflection produces prose, and prose does not contain
/// a blocking flag however confidently it is read.
#[must_use]
pub fn noticed(
    id: impl Into<String>,
    subject: impl Into<String>,
    question: impl Into<String>,
    opened_by: impl Into<String>,
) -> OpenQuestion {
    OpenQuestion {
        id: id.into(),
        subject: subject.into(),
        question: question.into(),
        why_it_matters: STAKES_NOT_STATED.to_string(),
        // Not invented. A reflection that said "I could not tell whether the
        // funding leg matters" has not said whether the answer changes what to
        // do next, and guessing `true` would put it at the head of the next
        // run's brief on nobody's judgement.
        blocking: false,
        value: QuestionValue::Curious,
        status: Status::Open,
        opened_by: opened_by.into(),
    }
}

/// Close every open question a finding cited.
///
/// Returns the ids it closed. A finding that cites a question the store does not
/// hold closes nothing and is not an error — the citation may be to a question
/// from another tenant, or to one already closed, and refusing the finding over
/// it would make citing a question risky.
pub fn close_cited<'a>(
    questions: &'a mut [OpenQuestion],
    finding_id: &str,
    cited: &[String],
) -> Vec<&'a str> {
    let mut closed = Vec::new();
    for q in questions.iter_mut() {
        if cited.iter().any(|c| c == &q.id) && q.close_as_answered(finding_id) {
            closed.push(q.id.as_str());
        }
    }
    closed
}

/// Order for the brief's 10% slot (AGENT-007 §4): blocking first.
///
/// A brief that buried a blocking question under six curiosities would let the
/// run repeat the mistake the question exists to prevent — which is the one
/// thing this slot is for.
#[must_use]
pub fn brief_order(questions: &[OpenQuestion]) -> Vec<&OpenQuestion> {
    let mut open: Vec<&OpenQuestion> = questions.iter().filter(|q| q.is_open()).collect();
    open.sort_by_key(|q| {
        (
            // Blocking first.
            u8::from(!q.blocking),
            // Then by stated value, descending.
            std::cmp::Reverse(q.value),
            // Then by id, so the order is reproducible rather than whatever the
            // store happened to return.
            q.id.clone(),
        )
    });
    open
}

#[cfg(test)]
mod tests {
    use super::*;

    fn question(id: &str, blocking: bool, value: QuestionValue) -> OpenQuestion {
        open(
            id,
            "BTC-USD",
            "is the March funding gap missing or zero?",
            "every carry study over that window silently treats it as zero",
            blocking,
            value,
            "run_1",
        )
        .expect("complete")
    }

    /// A question with no stated stakes never gets prioritised.
    #[test]
    fn a_question_must_say_why_it_matters() {
        let err = open(
            "q1",
            "BTC-USD",
            "is it zero?",
            "  ",
            false,
            QuestionValue::Curious,
            "run_1",
        )
        .expect_err("refused");
        assert_eq!(err, QuestionError::NoStakes);
        assert!(err.to_string().contains("nobody will prioritise"), "{err}");
    }

    /// And a subject, so the run that could answer it is told.
    #[test]
    fn a_question_without_a_subject_reaches_nobody() {
        assert_eq!(
            open(
                "q1",
                " ",
                "is it zero?",
                "it matters",
                false,
                QuestionValue::Curious,
                "r"
            ),
            Err(QuestionError::NoSubject)
        );
    }

    /// The platform closes it when a finding cites it.
    #[test]
    fn a_finding_that_cites_a_question_closes_it() {
        let mut qs = vec![question("q1", false, QuestionValue::Useful)];
        let closed = close_cited(&mut qs, "f_9", &["q1".to_string()]);
        assert_eq!(closed, vec!["q1"]);
        assert_eq!(
            qs[0].status,
            Status::Closed {
                by: "f_9".into(),
                how: How::Answered
            }
        );
    }

    /// A second close does not overwrite the first.
    ///
    /// "Who answered this" is the part worth keeping.
    #[test]
    fn the_first_answer_is_the_one_recorded() {
        let mut qs = vec![question("q1", false, QuestionValue::Useful)];
        close_cited(&mut qs, "f_9", &["q1".to_string()]);
        let second = close_cited(&mut qs, "f_12", &["q1".to_string()]);
        assert!(second.is_empty());
        assert_eq!(
            qs[0].status,
            Status::Closed {
                by: "f_9".into(),
                how: How::Answered
            }
        );
    }

    /// Citing a question nobody holds closes nothing and is not an error.
    ///
    /// Refusing the finding over it would make citing a question risky, and a
    /// run that has learned citing is risky stops citing.
    #[test]
    fn citing_an_unknown_question_is_not_a_failure() {
        let mut qs = vec![question("q1", false, QuestionValue::Useful)];
        assert!(close_cited(&mut qs, "f_9", &["q_other".to_string()]).is_empty());
        assert!(qs[0].is_open());
    }

    /// A blocking question leads the slot, whatever its stated value.
    #[test]
    fn a_blocking_question_is_not_buried_under_curiosities() {
        let qs = vec![
            question("q_a", false, QuestionValue::Decisive),
            question("q_b", true, QuestionValue::Curious),
            question("q_c", false, QuestionValue::Useful),
        ];
        let order: Vec<&str> = brief_order(&qs).iter().map(|q| q.id.as_str()).collect();
        assert_eq!(
            order,
            vec!["q_b", "q_a", "q_c"],
            "a brief that buried the blocking one would let the run repeat the mistake \
             the question exists to prevent"
        );
    }

    /// Closed questions leave the slot.
    #[test]
    fn a_closed_question_is_not_in_the_brief() {
        let mut qs = vec![
            question("q_a", true, QuestionValue::Decisive),
            question("q_b", false, QuestionValue::Useful),
        ];
        close_cited(&mut qs, "f_1", &["q_a".to_string()]);
        let order: Vec<&str> = brief_order(&qs).iter().map(|q| q.id.as_str()).collect();
        assert_eq!(order, vec!["q_b"]);
    }

    /// The order is reproducible, not whatever the store returned.
    #[test]
    fn two_equal_questions_order_by_id() {
        let qs = vec![
            question("q_z", false, QuestionValue::Useful),
            question("q_a", false, QuestionValue::Useful),
        ];
        let order: Vec<&str> = brief_order(&qs).iter().map(|q| q.id.as_str()).collect();
        assert_eq!(order, vec!["q_a", "q_z"]);
    }

    /// A question closed by nobody is a question somebody deleted.
    #[test]
    fn a_close_needs_somebody_to_attribute_it_to() {
        let mut q = question("q1", false, QuestionValue::Useful);
        assert!(!q.close_as_answered("  "));
        assert!(q.is_open());
    }

    /// The value field orders a brief and earns nothing.
    ///
    /// A value that fed a metric would be a run grading its own output, and
    /// `Yield` already scores an opened question at zero.
    #[test]
    fn the_value_is_the_askers_estimate_and_scores_nothing() {
        let mut y = crate::lifecycle::r#yield::Yield {
            trials_spent: 1,
            ..Default::default()
        };
        for value in QuestionValue::ALL {
            let _ = value;
            y.questions_opened += 1;
        }
        assert_eq!(y.knowledge_units(), 0);
        assert!(y.is_barren());
    }

    /// The deliberate path and the reflection path write the same keys.
    ///
    /// Two writers produce these entries — `lifecycle.open_question` and the
    /// loop at run end — and a reader of `agent_memory.entry` has to be able to
    /// treat them as one kind. Before `extra()` existed each built its own
    /// object, which is exactly the arrangement where one grows a key and the
    /// other's rows quietly lack it.
    #[test]
    fn both_writers_produce_the_same_extra_keys() {
        let deliberate = open(
            "q_1",
            "BTC-USD",
            "does the funding leg survive costs",
            "the whole carry thesis rests on it",
            true,
            QuestionValue::Decisive,
            "run_1",
        )
        .expect("a complete question");
        let in_passing = noticed(
            "question-0",
            "BTC-USD",
            "something odd in the tail",
            "run_1",
        );

        for q in [&deliberate, &in_passing] {
            let extra = q.extra();
            let keys: Vec<&str> = extra
                .as_object()
                .expect("an object")
                .keys()
                .map(String::as_str)
                .collect();
            let mut expected: Vec<&str> = EXTRA_KEYS.to_vec();
            expected.sort_unstable();
            let mut got = keys;
            got.sort_unstable();
            assert_eq!(got, expected, "the two writers disagree about the shape");
        }
    }

    /// What the two paths *do* differ in is recoverable, which is the reason
    /// they are separate paths.
    ///
    /// A question noticed during a reflection carries no stakes and says so; one
    /// opened deliberately could not have been created without them. A reader
    /// deciding what to spend a run on needs to tell those apart, and a sentinel
    /// that only existed as a string literal in the driver could not be checked
    /// for anywhere else.
    #[test]
    fn a_noticed_question_is_distinguishable_from_an_asked_one() {
        let asked = open(
            "q_1",
            "BTC-USD",
            "does the funding leg survive costs",
            "the whole carry thesis rests on it",
            true,
            QuestionValue::Decisive,
            "run_1",
        )
        .expect("a complete question");
        let noticed_one = noticed(
            "question-0",
            "BTC-USD",
            "something odd in the tail",
            "run_1",
        );

        assert!(asked.stakes_were_stated());
        assert!(!noticed_one.stakes_were_stated());
        assert_eq!(noticed_one.why_it_matters, STAKES_NOT_STATED);
        // And nothing is invented for it: a reflection did not say the answer
        // blocks anything, so it does not lead the next run's brief.
        assert!(!noticed_one.blocking);
        assert_eq!(noticed_one.value, QuestionValue::Curious);
        assert_eq!(brief_order(&[noticed_one, asked])[0].id, "q_1");
    }

    /// A closed question's `extra` says who closed it and how.
    ///
    /// The status is the field the desk filters on, and "closed" with no closer
    /// is the row that makes "who answered this" unanswerable — which is the
    /// part `close_as_answered` exists to keep.
    #[test]
    fn a_closed_questions_extra_names_its_closer() {
        let mut q = open(
            "q_1",
            "BTC-USD",
            "does the funding leg survive costs",
            "the carry thesis rests on it",
            false,
            QuestionValue::Useful,
            "run_1",
        )
        .expect("complete");
        assert_eq!(q.extra()["status"], serde_json::json!("open"));
        assert!(q.close_as_answered("finding_9"));
        let extra = q.extra();
        assert_eq!(extra["status"]["closed"]["by"], "finding_9");
        assert_eq!(extra["status"]["closed"]["how"], "answered");
    }
}
