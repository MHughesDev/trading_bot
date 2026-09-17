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
        let err = open("q1", "BTC-USD", "is it zero?", "  ", false, QuestionValue::Curious, "run_1")
            .expect_err("refused");
        assert_eq!(err, QuestionError::NoStakes);
        assert!(
            err.to_string().contains("nobody will prioritise"),
            "{err}"
        );
    }

    /// And a subject, so the run that could answer it is told.
    #[test]
    fn a_question_without_a_subject_reaches_nobody() {
        assert_eq!(
            open("q1", " ", "is it zero?", "it matters", false, QuestionValue::Curious, "r"),
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
}
